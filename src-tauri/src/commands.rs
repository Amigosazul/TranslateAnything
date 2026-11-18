//! 框选遮罩相关命令

use crate::capture::{self, MonitorRect};
use crate::config::PortableWebview;
use crate::{live, translate};
use serde::Deserialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindowBuilder};

const OVERLAY_LABEL: &str = "overlay";

#[derive(Default)]
pub struct AppState {
    /// 当前框选遮罩所在显示器
    overlay_monitor: Mutex<Option<MonitorRect>>,
    /// 已框选完成、正等新翻译框画出首帧后再关闭的遮罩
    overlay_closing: Mutex<bool>,
}

/// 新翻译框迟迟未报告首帧时，最多让遮罩再留这么久
const OVERLAY_LINGER_MAX: Duration = Duration::from_millis(1500);

/// 选区，物理像素，相对遮罩所在显示器左上角
#[derive(Debug, Deserialize)]
pub struct Rect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

/// 关闭窗口显示/隐藏时系统自带的位移 + 淡入淡出动画。
/// 遮罩与翻译框需要原地无缝切换，带动画会看到边框先缩放淡出、再淡入，两者对不上。
pub fn disable_transitions(win: &tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::TRUE;
        use windows::Win32::Graphics::Dwm::{DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute};
        if let Ok(hwnd) = win.hwnd() {
            let on = TRUE;
            unsafe {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_TRANSITIONS_FORCEDISABLED,
                    &on as *const _ as _,
                    std::mem::size_of_val(&on) as u32,
                );
            }
        }
    }
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
        .title(t!("window.select"))
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    disable_transitions(&win);
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

/// 框选完成。遮罩（连同选区边框）先不关：新翻译框是透明窗口，WebView 画出首帧前什么都看不到，
/// 若立即关遮罩，选区会先消失、片刻后翻译框才出现。等翻译框报告首帧（`frame_ready`）再关。
#[tauri::command]
pub async fn select_done(app: AppHandle, state: State<'_, AppState>, rect: Rect) -> Result<(), String> {
    let m = state.overlay_monitor.lock().unwrap().take().ok_or_else(|| t!("err.select.expired"))?;
    *state.overlay_closing.lock().unwrap() = true;
    if let Err(e) = live::open_frame(&app, m.x + rect.x, m.y + rect.y, rect.w, rect.h) {
        finish_overlay(&app);
        return Err(e);
    }
    // 兜底：翻译框页面加载异常时也不能让遮罩一直挡着屏幕
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(OVERLAY_LINGER_MAX).await;
        finish_overlay(&app2);
    });
    Ok(())
}

/// 关闭框选完成后保留的遮罩（只关一次，不误关之后新开的遮罩）
fn finish_overlay(app: &AppHandle) {
    let closing = std::mem::take(&mut *app.state::<AppState>().overlay_closing.lock().unwrap());
    if closing {
        close_overlay(app);
    }
}

/// 翻译框页面已画出首帧：此时关掉遮罩，选区边框与翻译框边框无缝衔接
#[tauri::command]
pub fn frame_ready(app: AppHandle) {
    finish_overlay(&app);
}

/// 可选翻译引擎清单（供设置界面使用）
#[tauri::command]
pub fn engine_catalog() -> &'static [translate::EngineInfo] {
    translate::CATALOG
}
