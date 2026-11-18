mod capture;
mod commands;
mod config;
mod layout;
mod live;
mod motion;
mod ocr;
mod translate;

use tauri::RunEvent;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .manage(commands::AppState::default())
        .manage(live::LiveState::default())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        let app = app.clone();
                        // 建窗口放到后台线程，不阻塞主线程
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(e) = commands::open_overlay(&app) {
                                eprintln!("打开框选遮罩失败：{e}");
                            }
                        });
                    }
                })
                .build(),
        )
        .setup(|app| {
            let hotkey = config::load()
                .map(|c| c.hotkey)
                .unwrap_or_else(|e| {
                    eprintln!("{e}\n使用默认快捷键 Alt+T");
                    "Alt+T".into()
                });
            if let Err(e) = app.global_shortcut().register(hotkey.as_str()) {
                eprintln!("注册快捷键 {hotkey} 失败（可能被其他程序占用）：{e}");
            } else {
                println!("translate-filter 已启动，按 {hotkey} 框选翻译");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::select_done,
            commands::select_cancel,
            commands::engine_catalog,
            live::frame_set_paused,
        ])
        .build(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");

    app.run(|_app, event| {
        // 所有窗口关闭后继续常驻后台，等待快捷键（托盘退出在 M3 实现）
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
