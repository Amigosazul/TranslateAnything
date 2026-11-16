//! 前端可调用的命令，以及遮罩窗 / 浮窗的创建

use crate::capture::{self, Frozen};
use crate::{config, ocr, translate};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

const OVERLAY_LABEL: &str = "overlay";
/// 浮窗最小尺寸（物理像素），太小的话按钮都放不下
const PANEL_MIN_W: u32 = 60;
const PANEL_MIN_H: u32 = 28;

#[derive(Default)]
pub struct AppState {
    /// 当前框选会话冻结的整屏截图
    frozen: Mutex<Option<Frozen>>,
    /// 浮窗首次翻译用的选区图像（按窗口 label 存放，取用后删除）
    pending: Mutex<HashMap<String, RgbaImage>>,
    panel_seq: AtomicU32,
}

/// 选区，物理像素，相对遮罩所在显示器左上角
#[derive(Debug, Deserialize)]
pub struct Rect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

#[derive(Debug, Serialize)]
pub struct TranslateResult {
    source: String,
    translated: String,
    /// 实际使用的引擎名
    engine: String,
}

/// 快捷键触发：冻结光标所在显示器的画面，并在其上打开框选遮罩
pub fn open_overlay(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(OVERLAY_LABEL).is_some() {
        return Ok(());
    }
    let cursor = app.cursor_position().map_err(|e| e.to_string())?;
    let frozen = capture::freeze_at(cursor.x.round() as i32, cursor.y.round() as i32)?;
    let (x, y, w, h) = (frozen.mon_x, frozen.mon_y, frozen.width(), frozen.height());
    *app.state::<AppState>().frozen.lock().unwrap() = Some(frozen);

    let win = WebviewWindowBuilder::new(app, OVERLAY_LABEL, WebviewUrl::App("overlay.html".into()))
        .title("框选翻译区域")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    // 先定位再设尺寸，避免跨 DPI 显示器时尺寸被系统换算
    win.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())?;
    win.set_size(PhysicalSize::new(w, h)).map_err(|e| e.to_string())?;
    win.show().map_err(|e| e.to_string())?;
    let _ = win.set_focus();
    Ok(())
}

fn close_overlay(app: &AppHandle, state: &AppState) {
    if let Some(win) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = win.destroy();
    }
    state.frozen.lock().unwrap().take();
}

#[tauri::command]
pub fn select_cancel(app: AppHandle, state: State<'_, AppState>) {
    close_overlay(&app, &state);
}

#[tauri::command]
pub async fn select_done(
    app: AppHandle,
    state: State<'_, AppState>,
    rect: Rect,
) -> Result<(), String> {
    let frozen = state.frozen.lock().unwrap().take();
    close_overlay(&app, &state);
    let frozen = frozen.ok_or("框选会话已失效")?;
    let image = capture::crop(&frozen.image, rect.x, rect.y, rect.w, rect.h).ok_or("选区为空")?;

    let label = format!("panel-{}", state.panel_seq.fetch_add(1, Ordering::Relaxed) + 1);
    state.pending.lock().unwrap().insert(label.clone(), image);

    let win = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("panel.html".into()))
        .title("翻译滤镜")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    win.set_position(PhysicalPosition::new(frozen.mon_x + rect.x, frozen.mon_y + rect.y))
        .map_err(|e| e.to_string())?;
    win.set_size(PhysicalSize::new(rect.w.max(PANEL_MIN_W), rect.h.max(PANEL_MIN_H)))
        .map_err(|e| e.to_string())?;
    win.show().map_err(|e| e.to_string())?;
    let _ = win.set_focus();
    Ok(())
}

/// 浮窗请求翻译自己覆盖的区域：首次用框选时冻结的图像，之后实时截屏
#[tauri::command]
pub async fn translate_panel(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<TranslateResult, String> {
    let pending = state.pending.lock().unwrap().remove(window.label());
    let image = match pending {
        Some(img) => img,
        None => capture_under(&window).await?,
    };

    // 配置读取失败也先做 OCR（用默认参数），这样能单独验证识别效果
    let cfg = config::load();
    let defaults = config::Config::default();
    let base = cfg.as_ref().unwrap_or(&defaults);
    let (ocr_lang, target) = (base.ocr_lang.clone(), base.target_lang.clone());
    let source = tauri::async_runtime::spawn_blocking(move || ocr::recognize(image, &ocr_lang, &target))
        .await
        .map_err(|e| e.to_string())??;
    if source.trim().is_empty() {
        return Err("未识别到文字".into());
    }

    // 翻译失败时把识别出的原文一起带回去
    let with_source = |e: String| format!("翻译失败：\n{e}\n\n—— 识别到的原文 ——\n{source}");
    let cfg = cfg.map_err(with_source)?;
    let (translated, engine) = translate::translate_with_fallback(&cfg, &source)
        .await
        .map_err(with_source)?;
    Ok(TranslateResult {
        source,
        translated,
        engine,
    })
}

/// 可选翻译引擎清单（供设置界面使用）
#[tauri::command]
pub fn engine_catalog() -> &'static [translate::EngineInfo] {
    translate::CATALOG
}

/// 暂时隐藏浮窗，截取它下面的屏幕内容
async fn capture_under(window: &WebviewWindow) -> Result<RgbaImage, String> {
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    window.hide().map_err(|e| e.to_string())?;
    // 等窗口真正从屏幕上消失
    tokio::time::sleep(Duration::from_millis(80)).await;
    let result = tauri::async_runtime::spawn_blocking(move || {
        capture::capture_region(pos.x, pos.y, size.width, size.height)
    })
    .await
    .map_err(|e| e.to_string());
    let _ = window.show();
    result?
}
