//! 沉浸模式：直接改写目标应用界面里的文字，而不是在上面盖一层浮窗。
//!
//! 第一个通道面向 Electron / Chromium 内核应用（reclaude 客户端、VS Code 等）：
//! 应用带 `--remote-debugging-port=<端口>` 启动后，本模块通过 Chrome DevTools Protocol
//! 连上它的每个页面，往隔离环境里注入 `inject.js`，由脚本在 DOM 里原地替换文字。
//! 软件本体（exe、asar）一个字节都不改，关掉 translate-filter 或按切换快捷键即恢复原文。
//!
//! 数据流：页面脚本收集原文 → binding 回调到这里 → `translate::translate_batch_with_fallback`
//! （先查本地译文库，未命中才请求引擎）→ `Runtime.evaluate` 调页面的 `__tf.receive()` 回填。

mod cdp;

use crate::config::{self, Config, ImmersiveTarget};
use crate::{store, translate};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::sync::watch;

/// 注入脚本运行的隔离环境名
const WORLD: &str = "translate-filter";
/// 页面脚本回调主程序用的函数名
const BINDING: &str = "__tfSend";
const SCRIPT: &str = include_str!("inject.js");
const CONFIG_SLOT: &str = "/*__TF_CONFIG__*/ {}";
/// 注入时预加载的最近常用译文条数
const PRELOAD: usize = 5000;

/// 沉浸模式运行状态（Tauri 托管）
pub struct ImmersiveState {
    /// true = 显示译文，false = 显示原文
    show: watch::Sender<bool>,
}

impl Default for ImmersiveState {
    fn default() -> Self {
        Self { show: watch::channel(true).0 }
    }
}

/// 启动：为每个配置的目标应用开一个守护任务，应用开着就接管，关了就等它再开
pub fn start(app: &AppHandle) {
    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}\n沉浸模式未启动");
            return;
        }
    };
    if !cfg.immersive.enabled {
        return;
    }
    let show = app.state::<ImmersiveState>().show.subscribe();
    for target in cfg.immersive.targets.into_iter().filter(|t| t.port != 0) {
        println!("沉浸模式：等待「{}」（调试端口 {}）", target.name, target.port);
        tauri::async_runtime::spawn(supervise(target, show.clone()));
    }
}

/// 在"译文 / 原文"之间切换，返回切换后是否显示译文
pub fn toggle(app: &AppHandle) -> bool {
    let state = app.state::<ImmersiveState>();
    state.show.send_modify(|v| *v = !*v);
    let now = *state.show.borrow();
    println!("沉浸模式：{}", if now { "显示译文" } else { "显示原文" });
    now
}

#[tauri::command]
pub fn immersive_toggle(app: AppHandle) -> bool {
    toggle(&app)
}

#[derive(Debug, Deserialize)]
struct TargetInfo {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    url: String,
    #[serde(rename = "webSocketDebuggerUrl", default)]
    ws: String,
}

async fn list_targets(port: u16) -> Result<Vec<TargetInfo>, String> {
    // 必须直连：本机环境变量里的代理会把 127.0.0.1 的请求也转走
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;
    client
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}

fn wanted(t: &TargetInfo, target: &ImmersiveTarget) -> bool {
    if !matches!(t.kind.as_str(), "page" | "iframe" | "webview") || t.ws.is_empty() {
        return false;
    }
    if t.url.starts_with("devtools://") || t.url.starts_with("chrome") || t.url == "about:blank" {
        return false;
    }
    target.url_prefixes.is_empty() || target.url_prefixes.iter().any(|p| t.url.starts_with(p.as_str()))
}

async fn supervise(target: ImmersiveTarget, show: watch::Receiver<bool>) {
    let attached: Arc<Mutex<HashSet<String>>> = Arc::default();
    let mut online = false;
    loop {
        match list_targets(target.port).await {
            Ok(list) => {
                if !online {
                    online = true;
                    println!("沉浸模式：已连上「{}」", target.name);
                }
                for t in list.into_iter().filter(|t| wanted(t, &target)) {
                    if !attached.lock().unwrap().insert(t.id.clone()) {
                        continue;
                    }
                    let (attached, target, show) = (attached.clone(), target.clone(), show.clone());
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = session(&t, &target, show).await {
                            eprintln!("沉浸模式：页面 {} 接管失败：{e}", t.url);
                        }
                        attached.lock().unwrap().remove(&t.id);
                    });
                }
            }
            Err(_) => {
                if online {
                    online = false;
                    println!("沉浸模式：「{}」已关闭，等待重新打开", target.name);
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// 组装注入脚本：把配置和预加载译文（译文库最近常用 + 术语表）填进占位符
fn build_script(cfg: &Config, target: &ImmersiveTarget, show: bool) -> String {
    let target_lang = cfg.target_lang.as_str();
    let mut preload: HashMap<String, String> = if cfg.persist_cache {
        store::recent(target_lang, PRELOAD).into_iter().collect()
    } else {
        HashMap::new()
    };
    preload.extend(translate::glossary(cfg));
    let cfg = json!({
        "enabled": show,
        "skipCjk": target_lang.to_ascii_lowercase().starts_with("zh"),
        "skipSelectors": target.skip_selectors,
        "preload": preload,
    });
    SCRIPT.replacen(CONFIG_SLOT, &cfg.to_string(), 1)
}

fn frame_ids(tree: &Value, out: &mut Vec<String>) {
    if let Some(id) = tree.pointer("/frame/id").and_then(Value::as_str) {
        out.push(id.to_string());
    }
    for child in tree.get("childFrames").and_then(Value::as_array).into_iter().flatten() {
        frame_ids(child, out);
    }
}

/// 接管一个页面目标，直到它关闭或连接断开
async fn session(t: &TargetInfo, target: &ImmersiveTarget, mut show: watch::Receiver<bool>) -> Result<(), String> {
    let (cdp, mut events) = cdp::connect(&t.ws).await?;
    let script = build_script(&config::load()?, target, *show.borrow());

    cdp.call("Runtime.enable", json!({})).await?;
    cdp.call("Page.enable", json!({})).await?;
    cdp.call("Runtime.addBinding", json!({ "name": BINDING, "executionContextName": WORLD }))
        .await?;
    // 以后每次加载新文档都自动注入（刷新、跳转）
    cdp.call(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": script, "worldName": WORLD }),
    )
    .await?;
    // 当前已加载的文档：给每个框架建隔离环境并注入一次
    let mut frames = Vec::new();
    frame_ids(&cdp.call("Page.getFrameTree", json!({})).await?["frameTree"], &mut frames);
    for frame in frames {
        let world = cdp
            .call("Page.createIsolatedWorld", json!({ "frameId": frame, "worldName": WORLD }))
            .await;
        if let Some(ctx) = world.ok().and_then(|w| w["executionContextId"].as_i64()) {
            let r = cdp
                .call("Runtime.evaluate", json!({ "expression": script, "contextId": ctx }))
                .await;
            if let Err(e) = r {
                eprintln!("沉浸模式：注入框架 {frame} 失败：{e}");
            }
        }
    }
    println!("沉浸模式：已接管页面 {}", t.url);

    // 我们的隔离环境 id，用于切换原文/译文时逐个通知
    let mut contexts: HashSet<i64> = HashSet::new();
    loop {
        tokio::select! {
            ev = events.recv() => {
                let Some(ev) = ev else { break };
                match ev.method.as_str() {
                    "Runtime.bindingCalled" if ev.params["name"] == BINDING => {
                        let ctx = ev.params["executionContextId"].as_i64().unwrap_or(0);
                        contexts.insert(ctx);
                        let payload = ev.params["payload"].as_str().unwrap_or("").to_string();
                        tauri::async_runtime::spawn(answer(cdp.clone(), ctx, payload));
                    }
                    "Runtime.executionContextCreated" => {
                        let c = &ev.params["context"];
                        if c["name"] == WORLD {
                            if let Some(id) = c["id"].as_i64() {
                                contexts.insert(id);
                            }
                        }
                    }
                    "Runtime.executionContextDestroyed" => {
                        if let Some(id) = ev.params["executionContextId"].as_i64() {
                            contexts.remove(&id);
                        }
                    }
                    "Runtime.executionContextsCleared" => contexts.clear(),
                    "Inspector.detached" | "Inspector.targetCrashed" => break,
                    _ => {}
                }
            }
            changed = show.changed() => {
                if changed.is_err() {
                    break;
                }
                let on = *show.borrow_and_update();
                for &ctx in &contexts {
                    let expr = format!("globalThis.__tf && __tf.setEnabled({on})");
                    let _ = cdp.call("Runtime.evaluate", json!({ "expression": expr, "contextId": ctx })).await;
                }
            }
        }
    }
    if !cdp.is_closed() {
        println!("沉浸模式：页面 {} 已断开", t.url);
    }
    Ok(())
}

#[derive(Deserialize)]
struct Request {
    texts: Vec<String>,
}

/// 处理页面发来的一批原文：翻译（先查本地库）后回填
async fn answer(cdp: cdp::Cdp, ctx: i64, payload: String) {
    let Ok(req) = serde_json::from_str::<Request>(&payload) else { return };
    let texts: Vec<String> = req.texts.into_iter().filter(|t| !t.trim().is_empty()).take(200).collect();
    if texts.is_empty() {
        return;
    }
    let expr = match config::load() {
        Ok(cfg) => match translate::translate_batch_with_fallback(&cfg, &texts).await {
            Ok((translated, engine)) => {
                if engine != translate::CACHE_ENGINE {
                    println!("沉浸模式：{} 条由【{engine}】翻译", texts.len());
                }
                let map: HashMap<&String, String> = texts.iter().zip(translated).collect();
                format!("globalThis.__tf && __tf.receive({})", json!({ "map": map }))
            }
            Err(e) => {
                eprintln!("沉浸模式：翻译失败：{e}");
                format!("globalThis.__tf && __tf.failed({})", json!({ "texts": texts }))
            }
        },
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    let _ = cdp.call("Runtime.evaluate", json!({ "expression": expr, "contextId": ctx })).await;
}
