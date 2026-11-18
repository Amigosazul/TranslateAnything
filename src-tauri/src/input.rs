//! 全局滚轮监听（只读）：用 Raw Input 接收鼠标滚轮事件，不拦截、不修改任何输入。
//!
//! 用途：滚轮一动就立刻唤醒对应翻译框的跟踪循环（不必等空闲时的低频截屏），
//! 并把滚动量告诉前端做预测平移。键盘翻页键不走这里，由悬停循环顺带轮询，避免监听全局键盘。
//!
//! 说明：同一进程里每类设备只有最后一次 `RegisterRawInputDevices` 生效。tao 启动时会为自己注册
//! 鼠标/键盘（只用于 Tauri 不转发的 DeviceEvent），本模块之后注册鼠标会接管鼠标这一类，不影响现有功能。

use std::sync::OnceLock;
use tokio::sync::broadcast;

/// 一次滚轮事件
#[derive(Clone, Copy, Debug)]
pub struct Wheel {
    /// 事件发生时的光标位置（物理像素，虚拟桌面坐标）
    pub x: i32,
    pub y: i32,
    /// 滚轮增量：一格 = ±120；竖直滚轮向上为正
    pub delta: i32,
    pub horizontal: bool,
}

static TX: OnceLock<broadcast::Sender<Wheel>> = OnceLock::new();
/// 收到的鼠标原始输入总数（含移动），仅用于自检
static RAW_EVENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn sender() -> &'static broadcast::Sender<Wheel> {
    TX.get_or_init(|| broadcast::channel(256).0)
}

pub fn subscribe() -> broadcast::Receiver<Wheel> {
    sender().subscribe()
}

/// 在独立线程里启动监听（只需调用一次）
pub fn start() {
    std::thread::Builder::new()
        .name("wheel-listener".into())
        .spawn(|| {
            if let Err(e) = run() {
                eprintln!("滚轮监听启动失败（译文仍会跟随滚动，只是起步稍慢）：{e}");
            }
        })
        .ok();
}

fn run() -> windows::core::Result<()> {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::{RAWINPUTDEVICE, RIDEV_INPUTSINK, RegisterRawInputDevices};
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::w;

    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let class = w!("TranslateFilterWheelListener");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        // 仅接收消息的隐藏窗口
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst.into()),
            None,
        )?;
        // 通用桌面设备页（0x01）下的鼠标（0x02）；INPUTSINK：不在前台也能收到
        let device = RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };
        RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32)?;

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// 鼠标原始输入里的滚轮标志位
const RI_MOUSE_WHEEL: u16 = 0x0400;
const RI_MOUSE_HWHEEL: u16 = 0x0800;

unsafe extern "system" fn wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::{GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTHEADER, RID_INPUT, RIM_TYPEMOUSE};
    use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, GetCursorPos, WM_INPUT};

    if msg == WM_INPUT {
        unsafe {
            let mut raw = RAWINPUT::default();
            let mut size = std::mem::size_of::<RAWINPUT>() as u32;
            let got = GetRawInputData(
                HRAWINPUT(lparam.0 as _),
                RID_INPUT,
                Some(&mut raw as *mut RAWINPUT as *mut _),
                &mut size,
                std::mem::size_of::<RAWINPUTHEADER>() as u32,
            );
            if got != u32::MAX && raw.header.dwType == RIM_TYPEMOUSE.0 {
                RAW_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let buttons = raw.data.mouse.Anonymous.Anonymous;
                let flags = buttons.usButtonFlags;
                if flags & (RI_MOUSE_WHEEL | RI_MOUSE_HWHEEL) != 0 {
                    let mut p = POINT::default();
                    let _ = GetCursorPos(&mut p);
                    let _ = sender().send(Wheel {
                        x: p.x,
                        y: p.y,
                        delta: buttons.usButtonData as i16 as i32,
                        horizontal: flags & RI_MOUSE_HWHEEL != 0,
                    });
                }
            }
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真机检查（只读，不模拟任何输入）：启动监听 TF_SECS 秒，打印期间收到的滚轮事件。
    /// cargo test 滚轮监听 -- --ignored --nocapture
    #[test]
    #[ignore]
    fn 滚轮监听() {
        let mut rx = subscribe();
        start();
        let secs: u64 = std::env::var("TF_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        let mut n = 0;
        while std::time::Instant::now() < until {
            match rx.try_recv() {
                Ok(w) => {
                    n += 1;
                    println!("{w:?}");
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
            }
        }
        let raw = RAW_EVENTS.load(std::sync::atomic::Ordering::Relaxed);
        println!("{secs} 秒内收到 {n} 次滚轮事件，鼠标原始输入共 {raw} 次");
    }
}
