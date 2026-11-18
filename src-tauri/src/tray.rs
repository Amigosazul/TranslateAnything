//! 系统托盘：常用开关（各应用的沉浸模式、原文/译文切换）+ 打开设置 + 退出

use crate::immersive::{self, State, TargetStatus};
use crate::{commands, config, settings};
use std::sync::Mutex;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Wry};

const TRAY_ID: &str = "main";

/// 上次构建菜单用的摘要，没变化就不重建（翻译计数之类的变化不影响菜单）
static LAST: Mutex<String> = Mutex::new(String::new());

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app, &immersive::statuses(app))?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("翻译滤镜")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                let _ = settings::open_settings(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn short_state(s: &TargetStatus) -> &'static str {
    match s.state {
        State::Off => "总开关已关",
        State::Disabled => "已停用",
        State::NotRunning => "未运行",
        State::NoPort => "未以沉浸模式启动",
        State::Conflict => "端口冲突",
        State::Waiting => "等待退出",
        State::Attached if s.show => "显示译文",
        State::Attached => "显示原文",
    }
}

fn build_menu(app: &AppHandle, list: &[TargetStatus]) -> tauri::Result<Menu<Wry>> {
    let hotkey = config::load().map(|c| c.hotkey).unwrap_or_default();
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(app, "select", format!("框选翻译\t{hotkey}"), true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let attached: Vec<_> = list.iter().filter(|s| s.state == State::Attached).collect();
    let showing = attached.iter().any(|s| s.show);
    menu.append(&CheckMenuItem::with_id(
        app,
        "show",
        "沉浸模式：显示译文",
        !attached.is_empty(),
        showing,
        None::<&str>,
    )?)?;
    for s in list {
        let text = format!("{}（{}）", s.name, short_state(s));
        let id = format!("target:{}", s.port);
        menu.append(&CheckMenuItem::with_id(app, id, text, s.state != State::Off, s.enabled, None::<&str>)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?)?;
    Ok(menu)
}

/// 应用状态变化后更新托盘菜单
pub fn refresh(app: &AppHandle, list: &[TargetStatus]) {
    let sig: Vec<_> = list.iter().map(|s| (s.port, &s.name, s.enabled, s.state, s.show)).collect();
    let sig = format!("{sig:?}");
    {
        let mut last = LAST.lock().unwrap();
        if *last == sig {
            return;
        }
        *last = sig;
    }
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    match build_menu(app, list) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => eprintln!("更新托盘菜单失败：{e}"),
    }
}

fn on_menu(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref().to_string();
    match id.as_str() {
        "select" => {
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(e) = commands::open_overlay(&app) {
                    eprintln!("打开框选遮罩失败：{e}");
                }
            });
        }
        "show" => {
            immersive::toggle(app);
        }
        "settings" => {
            let _ = settings::open_settings(app);
        }
        "quit" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                // 先让所有页面恢复原文再退出
                immersive::shutdown(&app).await;
                app.exit(0);
            });
        }
        other => {
            if let Some(port) = other.strip_prefix("target:").and_then(|p| p.parse::<u16>().ok()) {
                let enabled = immersive::statuses(app).iter().find(|s| s.port == port).is_some_and(|s| s.enabled);
                if let Err(e) = settings::set_target_enabled(app, port, !enabled) {
                    eprintln!("{e}");
                }
            }
        }
    }
}
