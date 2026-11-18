//! 以沉浸模式启动应用、识别可接管的应用（Electron / CEF / WebView2）。
//!
//! 目标应用（比如 Claude 客户端）通常由别的程序带着一组自己的参数启动（代理、用户数据目录……），
//! 我们不自己拼参数，而是：应用在运行时读出它主进程的完整命令行记进配置，
//! 等用户从托盘把它完全退出，再用同样的参数加上调试端口重新打开。
//! 这类应用多是单实例的，不先退出的话新启动的进程会把参数转交给旧实例然后自己退出，端口开不起来。
//!
//! 调试端口怎么带：Electron / CEF 加命令行参数 `--remote-debugging-port`；
//! WebView2 应用的浏览器进程由系统运行时启动，参数只能通过环境变量 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`
//! 传进去——只设在我们启动的这个进程上，不写进用户环境变量，否则本机所有 WebView2 程序都会抢同一个端口。

use super::{State, notify, procs, status_of};
use crate::config;
use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// 等用户退出应用的最长时间
const WAIT_EXIT: Duration = Duration::from_secs(300);

/// 以沉浸模式启动（或重启）配置里端口为 `port` 的应用，返回给人看的结果
pub async fn launch(app: &AppHandle, port: u16) -> Result<String, String> {
    let cfg = config::load()?;
    let target = cfg
        .immersive
        .targets
        .iter()
        .find(|t| t.port == port)
        .cloned()
        .ok_or_else(|| t!("err.immersive.no_config"))?;
    let exe = PathBuf::from(target.exe.trim());
    if target.exe.trim().is_empty() || !exe.exists() {
        return Err(t!("err.immersive.bad_exe", path = target.exe));
    }
    if blocking(move || procs::port_owner(port)).await.is_some() {
        return Ok(t!("status.port_already_open"));
    }

    let e = exe.clone();
    if let Some(pid) = blocking(move || procs::main_process(&e)).await {
        // 记下当前启动参数
        let args = blocking(move || procs::command_line(pid)).await.map(|c| procs::args_of(&c));
        if let Some(args) = args {
            save_args(port, &args)?;
            let _ = app.emit("config-changed", ());
        }
        set_waiting(app, port, Some(t!("status.wait_exit", name = target.name)));
        let start = Instant::now();
        loop {
            let e = exe.clone();
            if blocking(move || procs::main_process(&e)).await.is_none() {
                break;
            }
            if start.elapsed() > WAIT_EXIT {
                set_waiting(app, port, None);
                return Err(t!("err.immersive.wait_timeout"));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        set_waiting(app, port, Some(t!("status.reopening")));
        // 等子进程收尾、单实例锁释放
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    let args = config::load()?
        .immersive
        .targets
        .into_iter()
        .find(|t| t.port == port)
        .map(|t| t.args)
        .unwrap_or_default();
    let e = exe.clone();
    let kind = blocking(move || procs::detect_kind(&e)).await;
    let r = if kind == Some(procs::Kind::WebView2) {
        spawn(&exe, args.trim(), Some(port))
    } else {
        let line = format!("{} --remote-debugging-port={port}", args.trim()).trim().to_string();
        spawn(&exe, &line, None)
    };
    set_waiting(app, port, None);
    r?;
    Ok(t!("status.launched", name = target.name))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tauri::async_runtime::spawn_blocking(f).await.expect("后台任务异常")
}

/// 启动程序；`webview2_port` 有值时通过环境变量让它的 WebView2 浏览器进程开调试端口
fn spawn(exe: &Path, args: &str, webview2_port: Option<u16>) -> Result<(), String> {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const WEBVIEW2_ARGS: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";
    let mut cmd = std::process::Command::new(exe);
    if let Some(port) = webview2_port {
        // 保留用户自己设过的附加参数
        let old = std::env::var(WEBVIEW2_ARGS).unwrap_or_default();
        cmd.env(WEBVIEW2_ARGS, format!("{old} --remote-debugging-port={port}").trim());
    }
    cmd.raw_arg(args)
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(|_| ())
        .map_err(|e| t!("err.immersive.launch", e = e))
}

fn save_args(port: u16, args: &str) -> Result<(), String> {
    let mut cfg = config::load()?;
    if let Some(t) = cfg.immersive.targets.iter_mut().find(|t| t.port == port) {
        if t.args != args {
            t.args = args.to_string();
            config::save(&cfg)?;
        }
    }
    Ok(())
}

fn set_waiting(app: &AppHandle, port: u16, msg: Option<String>) {
    if let Some(s) = status_of(app, port) {
        let mut s = s.lock().unwrap();
        if let Some(m) = &msg {
            s.state = State::Waiting;
            s.message = m.clone();
        }
        s.waiting = msg;
    }
    notify(app);
}

/// 识别到的、可以接管的应用
#[derive(Debug, Serialize)]
pub struct DetectedApp {
    pub name: String,
    pub exe: String,
    /// 应用类型
    pub kind: procs::Kind,
    /// 已经开着的调试端口（如果是以调试模式启动的）
    pub port: Option<u16>,
    /// 是否已在配置里
    pub configured: bool,
}

/// 列出正在运行、可以接管的应用（Electron / CEF / WebView2）
pub async fn detect_apps() -> Vec<DetectedApp> {
    let configured: Vec<String> = config::load()
        .map(|c| c.immersive.targets.into_iter().map(|t| t.exe).collect())
        .unwrap_or_default();
    let apps = blocking(procs::running_apps).await;
    apps.into_iter()
        .map(|(exe, (kind, port))| DetectedApp {
            name: display_name(&exe),
            configured: configured.iter().any(|c| procs::same_path(Path::new(c), &exe)),
            exe: exe.to_string_lossy().into_owned(),
            kind,
            port,
        })
        .collect()
}

/// 显示名：程序名；像 app.exe 这种通用名字时带上目录名
fn display_name(exe: &Path) -> String {
    let stem = exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let generic = ["app", "electron", "main", "launcher"];
    match exe.parent().and_then(|d| d.file_name()) {
        Some(dir) if generic.contains(&stem.to_lowercase().as_str()) => format!("{}（{stem}）", dir.to_string_lossy()),
        _ => stem,
    }
}
