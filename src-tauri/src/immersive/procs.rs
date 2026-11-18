//! Windows 进程工具：端口归属、进程路径与命令行、识别可接管的应用（Electron / CEF / WebView2）。
//!
//! 沉浸模式靠它做"身份校验"：调试端口背后的进程必须就是配置里的那个程序（或由它启动的浏览器进程），
//! 否则（比如别的程序恰好占了同一端口）一律不接管。

use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use windows::Wdk::System::Threading::{NtQueryInformationProcess, PROCESSINFOCLASS};
use windows::Win32::Foundation::{CloseHandle, HANDLE, UNICODE_STRING};
use windows::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER};
use windows::Win32::Networking::WinSock::AF_INET;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::PWSTR;

/// 正在本机 127.0.0.1 / 0.0.0.0 上监听该 TCP 端口的进程 id
pub fn port_owner(port: u16) -> Option<u32> {
    unsafe {
        let mut size = 0u32;
        let _ = GetExtendedTcpTable(None, &mut size, false, AF_INET.0 as u32, TCP_TABLE_OWNER_PID_LISTENER, 0);
        if size == 0 {
            return None;
        }
        // 两次调用之间表可能变大，多留一些余量
        let mut buf = vec![0u8; size as usize + 4096];
        size = buf.len() as u32;
        if GetExtendedTcpTable(
            Some(buf.as_mut_ptr() as *mut c_void),
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        ) != 0
        {
            return None;
        }
        let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
        let rows = std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
        rows.iter()
            .find(|r| u16::from_be((r.dwLocalPort & 0xFFFF) as u16) == port)
            .map(|r| r.dwOwningPid)
    }
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn open(pid: u32) -> Option<Handle> {
    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok().map(Handle) }
}

/// 进程的完整程序路径（无权限读取的系统进程返回 None）
pub fn exe_path(pid: u32) -> Option<PathBuf> {
    let h = open(pid)?;
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    unsafe { QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).ok()? };
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
}

/// 进程的完整命令行
pub fn command_line(pid: u32) -> Option<String> {
    // ProcessCommandLineInformation：返回 UNICODE_STRING + 紧随其后的字符缓冲区（Windows 8.1 起）
    const PROCESS_COMMAND_LINE_INFORMATION: PROCESSINFOCLASS = PROCESSINFOCLASS(60);
    let h = open(pid)?;
    unsafe {
        let mut len = 0u32;
        let _ = NtQueryInformationProcess(h.0, PROCESS_COMMAND_LINE_INFORMATION, std::ptr::null_mut(), 0, &mut len);
        if len == 0 {
            return None;
        }
        // 用 u64 数组保证对齐
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let status = NtQueryInformationProcess(
            h.0,
            PROCESS_COMMAND_LINE_INFORMATION,
            buf.as_mut_ptr() as *mut c_void,
            len,
            &mut len,
        );
        if status.is_err() {
            return None;
        }
        let us = &*(buf.as_ptr() as *const UNICODE_STRING);
        if us.Buffer.is_null() {
            return None;
        }
        let chars = std::slice::from_raw_parts(us.Buffer.0, us.Length as usize / 2);
        Some(String::from_utf16_lossy(chars))
    }
}

/// 所有进程（pid, 父进程 pid, 进程名）
fn processes() -> Vec<(u32, u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let snap = Handle(snap);
        let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap.0, &mut e).is_ok();
        while ok {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.push((e.th32ProcessID, e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..n])));
            ok = Process32NextW(snap.0, &mut e).is_ok();
        }
    }
    out
}

/// 监听调试端口的进程 `pid` 是否属于程序 `exe`：就是它本身，或由它启动（沿父进程链往上找几层）。
/// Electron 和多数 CEF 应用由主进程自己监听；WebView2 应用监听端口的是它启动的 `msedgewebview2.exe`，
/// Steam 这类把 CEF 放在辅助进程里的也一样
pub fn owned_by(pid: u32, exe: &Path) -> bool {
    let parent: HashMap<u32, u32> = processes().into_iter().map(|(p, pp, _)| (p, pp)).collect();
    let mut cur = pid;
    for _ in 0..4 {
        if exe_path(cur).is_some_and(|p| same_path(&p, exe)) {
            return true;
        }
        match parent.get(&cur) {
            Some(&pp) if pp != 0 && pp != cur => cur = pp,
            _ => break,
        }
    }
    false
}

/// 路径比较：不区分大小写、统一斜杠
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    norm(a) == norm(b)
}

/// 指定程序的所有进程 id
pub fn pids_of(exe: &Path) -> Vec<u32> {
    let Some(name) = exe.file_name().map(|n| n.to_string_lossy().to_lowercase()) else { return Vec::new() };
    processes()
        .into_iter()
        .filter(|(_, _, n)| n.to_lowercase() == name)
        .filter(|(pid, _, _)| exe_path(*pid).is_some_and(|p| same_path(&p, exe)))
        .map(|(pid, _, _)| pid)
        .collect()
}

/// Electron / Chromium 程序的主进程：命令行里不带 `--type=`（子进程都带 --type=renderer 等）
pub fn main_process(exe: &Path) -> Option<u32> {
    pids_of(exe)
        .into_iter()
        .find(|pid| command_line(*pid).is_some_and(|c| !c.contains("--type=")))
}

/// 去掉命令行开头的程序路径（可能带引号）和已有的调试端口参数，只留参数部分
pub fn args_of(cmdline: &str) -> String {
    let c = cmdline.trim();
    let rest = if let Some(stripped) = c.strip_prefix('"') {
        stripped.find('"').map(|i| &stripped[i + 1..]).unwrap_or("")
    } else {
        c.find(' ').map(|i| &c[i + 1..]).unwrap_or("")
    };
    rest.split(' ')
        .filter(|a| !a.starts_with("--remote-debugging-port="))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// 程序目录看起来是 Electron 应用（带 asar 资源包和 Chromium 资源）
pub fn is_electron(exe: &Path) -> bool {
    let Some(dir) = exe.parent() else { return false };
    let electron_dir = |d: &Path| {
        let res = d.join("resources");
        (res.join("app.asar").exists() || res.join("app").join("package.json").exists())
            && (d.join("resources.pak").exists() || d.join("chrome_100_percent.pak").exists())
    };
    // 新版 VS Code 把资源放在以提交号命名的子目录里（Microsoft VS Code\df53daabb1\resources\app）
    electron_dir(dir)
        || std::fs::read_dir(dir).is_ok_and(|rd| {
            rd.flatten().filter(|e| e.path().is_dir()).any(|e| electron_dir(&e.path()))
        })
}

/// 可接管的应用类型（都基于 Chromium，走同一套 CDP 通道，区别只在怎么带上调试端口）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// 命令行参数 `--remote-debugging-port`
    Electron,
    /// 多数也认命令行参数；把 CEF 放在辅助进程里的（如 Steam）参数传不过去
    Cef,
    /// 系统共用的 WebView2 运行时：参数要通过环境变量 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 传给浏览器进程
    WebView2,
}

/// 程序目录带 CEF 运行库
fn is_cef(exe: &Path) -> bool {
    exe.parent().is_some_and(|d| d.join("libcef.dll").exists())
}

/// 程序用 WebView2：目录里有加载器，或程序本身静态链接了加载器（Tauri、Wails 等），
/// 后者在程序文件里找加载器要用的 `EmbeddedBrowserWebView.dll` 字样（ASCII 或 UTF-16）
fn is_webview2(exe: &Path) -> bool {
    // 加载器可能放在程序目录或其下一级；.NET 程序（WPF / WinForms）带的是 Microsoft.Web.WebView2.Core.dll
    let loader = |d: &Path| d.join("WebView2Loader.dll").exists() || d.join("Microsoft.Web.WebView2.Core.dll").exists();
    if let Some(dir) = exe.parent() {
        if loader(dir) || std::fs::read_dir(dir).is_ok_and(|rd| rd.flatten().any(|e| e.path().is_dir() && loader(&e.path()))) {
            return true;
        }
    }
    // 只看合理大小的程序文件，避免把大型程序整个读进内存
    const MAX: u64 = 256 * 1024 * 1024;
    if std::fs::metadata(exe).map_or(true, |m| m.len() > MAX) {
        return false;
    }
    let Ok(bytes) = std::fs::read(exe) else { return false };
    let ascii = b"EmbeddedBrowserWebView";
    let wide: Vec<u8> = ascii.iter().flat_map(|&b| [b, 0]).collect();
    contains(&bytes, ascii) || contains(&bytes, &wide)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// 判断程序属于哪一类；都不是返回 None
pub fn detect_kind(exe: &Path) -> Option<Kind> {
    if is_electron(exe) {
        Some(Kind::Electron)
    } else if is_cef(exe) {
        Some(Kind::Cef)
    } else if is_webview2(exe) {
        Some(Kind::WebView2)
    } else {
        None
    }
}

/// 命令行里的调试端口
fn port_in(cmdline: &str) -> Option<u16> {
    cmdline
        .split([' ', '"'])
        .find_map(|a| a.strip_prefix("--remote-debugging-port="))
        .and_then(|s| s.parse().ok())
}

/// CEF 的辅助进程、崩溃上报等，不是用户打开的那个程序
fn cef_helper(exe: &Path) -> bool {
    let n = exe.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    ["helper", "subprocess", "crashpad", "renderer", "gpu", "service"].iter().any(|k| n.contains(k))
}

/// 系统自带的程序（资源管理器、开始菜单、小组件等也用 WebView2），不列出来
fn system_program(exe: &Path) -> bool {
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()).to_lowercase();
    exe.to_string_lossy().to_lowercase().starts_with(&windir)
}

/// 正在运行、可以接管的应用：程序路径 →（类型，已开着的调试端口）
pub fn running_apps() -> BTreeMap<PathBuf, (Kind, Option<u16>)> {
    let me = std::env::current_exe().ok();
    let procs = processes();
    let mine = |p: &Path| me.as_ref().is_some_and(|m| same_path(m, p));
    let mut out: BTreeMap<PathBuf, (Kind, Option<u16>)> = BTreeMap::new();

    // WebView2：浏览器主进程（命令行不带 --type=）的父进程就是宿主程序，端口参数也在浏览器主进程上
    let path_of: HashMap<u32, PathBuf> = procs
        .iter()
        .filter(|(_, _, n)| n.to_lowercase().ends_with(".exe"))
        .filter_map(|(pid, _, _)| exe_path(*pid).map(|p| (*pid, p)))
        .collect();
    for (pid, ppid, name) in &procs {
        if !name.eq_ignore_ascii_case("msedgewebview2.exe") {
            continue;
        }
        let Some(cmd) = command_line(*pid).filter(|c| !c.contains("--type=")) else { continue };
        let Some(host) = path_of.get(ppid) else { continue };
        if mine(host) || system_program(host) {
            continue;
        }
        let entry = out.entry(host.clone()).or_insert((Kind::WebView2, None));
        entry.1 = entry.1.or(port_in(&cmd));
    }

    // Electron / CEF：程序目录特征 + 主进程（命令行不带 --type=）
    let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();
    for p in path_of.into_values() {
        seen.entry(p.to_string_lossy().to_lowercase()).or_insert(p);
    }
    for p in seen.into_values() {
        if mine(&p) || out.contains_key(&p) {
            continue;
        }
        let kind = if is_electron(&p) {
            Kind::Electron
        } else if is_cef(&p) && !cef_helper(&p) {
            Kind::Cef
        } else {
            continue;
        };
        let port = main_process(&p).and_then(command_line).and_then(|c| port_in(&c));
        out.insert(p, (kind, port));
    }
    // 命令行带了端口不等于真开了（实测 NVIDIA App 不把参数交给 CEF），只报确实在监听、且属于该程序的
    for (exe, (_, port)) in out.iter_mut() {
        if port.is_some_and(|p| !port_owner(p).is_some_and(|pid| owned_by(pid, exe))) {
            *port = None;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 拆出启动参数() {
        assert_eq!(args_of(r#""C:\A B\app.exe" --x=1 --remote-debugging-port=9 --y"#), "--x=1 --y");
        assert_eq!(args_of(r"C:\app.exe --lang=en-US"), "--lang=en-US");
        assert_eq!(args_of(r"C:\app.exe"), "");
    }

    #[test]
    fn 读取本进程信息() {
        let pid = std::process::id();
        let me = std::env::current_exe().unwrap();
        assert!(same_path(&exe_path(pid).unwrap(), &me));
        assert!(command_line(pid).unwrap().len() > 3);
        assert!(pids_of(&me).contains(&pid));
    }

    #[test]
    fn 命令行里的调试端口() {
        assert_eq!(port_in(r#""C:\a.exe" --x --remote-debugging-port=39301 --y"#), Some(39301));
        assert_eq!(port_in(r#"msedgewebview2.exe --embedded-browser-webview=1 "--remote-debugging-port=9222""#), Some(9222));
        assert_eq!(port_in("app.exe --lang=en"), None);
    }

    /// 本机实测：列出正在运行、可接管的应用。cargo test -- --ignored --nocapture 识别本机应用
    #[test]
    #[ignore]
    fn 识别本机应用() {
        for (exe, (kind, port)) in running_apps() {
            println!("{kind:?}\t{port:?}\t{}", exe.display());
        }
        for exe in std::env::var("TA_KIND_PROBE").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
            println!("detect_kind {exe} → {:?}", detect_kind(Path::new(exe)));
        }
    }

    #[test]
    fn 进程属于自身() {
        let me = std::env::current_exe().unwrap();
        assert!(owned_by(std::process::id(), &me));
        assert!(!owned_by(std::process::id(), Path::new(r"C:\not\there.exe")));
    }

    #[test]
    fn 端口归属() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert_eq!(port_owner(port), Some(std::process::id()));
    }
}
