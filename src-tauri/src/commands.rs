//! 框选遮罩相关命令

use crate::capture::{self, MonitorRect};
use crate::config::PortableWebview;
use crate::{live, translate};
use serde::Deserialize;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindowBuilder};

const OVERLAY_LABEL: &str = "overlay";

#[derive(Default)]
pub struct AppState {
    /// 当前框选遮罩所在显示器
    overlay_monitor: Mutex<Option<MonitorRect>>,
}

/// 选区，物理像素，相对遮罩所在显示器左上角
#[derive(Debug, Deserialize)]
pub struct Rect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

/// 快捷键触发：在光标所在显示器上打开框选遮罩
pub fn open_overlay(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(OVERLAY_LABEL).is_some() {
        return Ok(());
    }
    let cursor = app.cursor_position().map_err(|e| e.to_string())?;
    let m = capture::monitor_at(cursor.x.round() as i32, cursor.y.round() as i32)?;
    let (x, y, w, h) = (m.x, m.y, m.w, m.h);
    *app.state::<AppState>().overlay_monitor.lock().unwrap() = Some(m);

    let win = WebviewWindowBuilder::new(app, OVERLAY_LABEL, WebviewUrl::App("overlay.html".into()))
        .portable_data()
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

fn close_overlay(app: &AppHandle) -> Option<MonitorRect> {
    if let Some(win) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = win.destroy();
    }
    app.state::<AppState>().overlay_monitor.lock().unwrap().take()
}

#[tauri::command]
pub fn select_cancel(app: AppHandle) {
    close_overlay(&app);
}

#[tauri::command]
pub async fn select_done(app: AppHandle, _state: State<'_, AppState>, rect: Rect) -> Result<(), String> {
    let m = close_overlay(&app).ok_or("框选会话已失效")?;
    live::open_frame(&app, m.x + rect.x, m.y + rect.y, rect.w, rect.h)
}

/// 可选翻译引擎清单（供设置界面使用）
#[tauri::command]
pub fn engine_catalog() -> &'static [translate::EngineInfo] {
    translate::CATALOG
}
