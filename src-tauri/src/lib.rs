mod capture;
mod dialog;
mod commands;
mod config;
mod immersive;
mod input;
mod layout;
mod live;
mod motion;
mod ocr;
mod settings;
mod store;
mod translate;
mod tray;

use std::sync::OnceLock;
use tauri::RunEvent;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// 沉浸模式"译文 / 原文"切换快捷键（注册成功后记下，用来区分是哪个快捷键触发）
static IMMERSIVE_HOTKEY: OnceLock<Shortcut> = OnceLock::new();

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .manage(commands::AppState::default())
        .manage(live::LiveState::default())
        .manage(immersive::ImmersiveState::default())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    if IMMERSIVE_HOTKEY.get() == Some(shortcut) {
                        immersive::toggle(app);
                    } else {
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
            // 只读监听滚轮，让译文起步更跟手
            input::start();
            let cfg = config::load().unwrap_or_else(|e| {
                eprintln!("{e}\n使用默认配置");
                config::Config::default()
            });
            let hotkey = cfg.hotkey;
            if let Err(e) = app.global_shortcut().register(hotkey.as_str()) {
                eprintln!("注册快捷键 {hotkey} 失败（可能被其他程序占用）：{e}");
            } else {
                println!("随译已启动，按 {hotkey} 框选翻译");
            }
            // 切换快捷键总是注册（总开关可在运行中打开），没有接管中的应用时按了也无副作用
            let key = cfg.immersive.hotkey;
            let registered = key
                .parse::<Shortcut>()
                .map_err(|e| e.to_string())
                .and_then(|s| app.global_shortcut().register(s).map(|_| s).map_err(|e| e.to_string()));
            match registered {
                Ok(s) => {
                    let _ = IMMERSIVE_HOTKEY.set(s);
                    println!("沉浸模式：按 {key} 在译文和原文之间切换");
                }
                Err(e) => eprintln!("注册沉浸模式快捷键 {key} 失败：{e}"),
            }
            immersive::reload(app.handle());
            if let Err(e) = tray::init(app.handle()) {
                eprintln!("创建托盘图标失败：{e}");
            }
            // 带 --settings 启动时直接打开设置窗口
            if std::env::args().any(|a| a == "--settings") {
                let _ = settings::open_settings(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::select_done,
            commands::select_cancel,
            commands::engine_catalog,
            live::frame_set_paused,
            settings::settings_get,
            settings::settings_save,
            settings::immersive_status,
            settings::immersive_set_enabled,
            settings::immersive_set_show,
            settings::immersive_launch,
            settings::immersive_detect,
            settings::store_query,
            settings::store_set,
            settings::store_delete,
            settings::store_clear,
            settings::open_settings_window,
            settings::store_stats,
            settings::engine_test,
            settings::overlay_start,
            settings::ocr_languages,
            settings::frames_count,
            settings::frames_close_all,
            settings::store_pick_and_relocate,
            settings::store_reset_location,
            settings::reveal_file,
        ])
        .build(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");

    app.run(|_app, event| {
        // 所有窗口关闭后继续常驻后台（托盘菜单"退出"才真正退出）
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
