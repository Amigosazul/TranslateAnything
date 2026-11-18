//! 设置窗口与它用到的命令：配置读写、沉浸模式管理、译文库与术语表管理

use crate::config::{self, Config};
use crate::immersive::{self, DetectedApp, TargetStatus};
use crate::{store, translate};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const SETTINGS_LABEL: &str = "settings";

/// 打开设置窗口（已打开则调到前台）
pub fn open_settings(app: &AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(app, SETTINGS_LABEL, WebviewUrl::App("settings.html".into()))
        .title("翻译滤镜 · 设置")
        .inner_size(1000.0, 720.0)
        .min_inner_size(760.0, 520.0)
        .center()
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct SettingsView {
    config: Config,
    config_path: String,
    db_path: String,
    /// 当前目标语言的内置术语表
    builtin_glossary: BTreeMap<String, String>,
}

#[tauri::command]
pub fn settings_get() -> Result<SettingsView, String> {
    let config = config::load()?;
    Ok(SettingsView {
        builtin_glossary: translate::builtin_glossary(&config.target_lang),
        config,
        config_path: config::config_path().map(|p| p.display().to_string()).unwrap_or_default(),
        db_path: store::db_path().map(|p| p.display().to_string()).unwrap_or_default(),
    })
}

/// 保存整份配置并立即生效（快捷键除外，需重启）
#[tauri::command]
pub fn settings_save(app: AppHandle, config: Config) -> Result<(), String> {
    config::save(&config)?;
    immersive::reload(&app);
    Ok(())
}

/// 单独开关某个应用
pub fn set_target_enabled(app: &AppHandle, port: u16, enabled: bool) -> Result<(), String> {
    let mut cfg = config::load()?;
    let t = cfg.immersive.targets.iter_mut().find(|t| t.port == port).ok_or("找不到这个应用")?;
    t.enabled = enabled;
    config::save(&cfg)?;
    immersive::reload(app);
    Ok(())
}

#[tauri::command]
pub fn immersive_set_enabled(app: AppHandle, port: u16, enabled: bool) -> Result<(), String> {
    set_target_enabled(&app, port, enabled)
}

#[tauri::command]
pub fn immersive_status(app: AppHandle) -> Vec<TargetStatus> {
    immersive::statuses(&app)
}

/// 原文 / 译文切换；`port` 为空表示所有应用
#[tauri::command]
pub fn immersive_set_show(app: AppHandle, port: Option<u16>, show: bool) {
    immersive::set_show(&app, port, show);
}

#[tauri::command]
pub async fn immersive_launch(app: AppHandle, port: u16) -> Result<String, String> {
    immersive::launch(&app, port).await
}

#[tauri::command]
pub async fn immersive_detect() -> Vec<DetectedApp> {
    immersive::detect_apps().await
}

#[tauri::command]
pub fn store_query(keyword: String, manual_only: bool, offset: i64, limit: i64) -> Result<store::Page, String> {
    let lang = config::load()?.target_lang;
    store::query(&lang, &keyword, manual_only, offset, limit.clamp(1, 500))
}

/// 改译：写成手工译文，并推送到已接管的页面立即生效
#[tauri::command]
pub fn store_set(app: AppHandle, source: String, target: String) -> Result<(), String> {
    if source.trim().is_empty() || target.trim().is_empty() {
        return Err("原文和译文都不能为空".into());
    }
    let lang = config::load()?.target_lang;
    store::set_manual(&lang, &source, &target)?;
    let set: BTreeMap<String, String> = [(source, target)].into();
    immersive::broadcast_patch(&app, json!({ "set": set }));
    Ok(())
}

/// 删除若干条译文；已接管的页面会重新翻译这些原文
#[tauri::command]
pub fn store_delete(app: AppHandle, sources: Vec<String>) -> Result<usize, String> {
    let lang = config::load()?.target_lang;
    let n = store::delete(&lang, &sources)?;
    immersive::broadcast_patch(&app, json!({ "del": sources }));
    Ok(n)
}

/// 清空当前目标语言的译文（可保留手工译文）。已显示在页面上的译文不受影响，页面刷新后才会重新翻译
#[tauri::command]
pub fn store_clear(keep_manual: bool) -> Result<usize, String> {
    let lang = config::load()?.target_lang;
    store::clear(&lang, keep_manual)
}

#[tauri::command]
pub fn open_settings_window(app: AppHandle) -> Result<(), String> {
    open_settings(&app)
}
