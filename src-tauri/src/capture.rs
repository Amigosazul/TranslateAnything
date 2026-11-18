//! 屏幕截取。所有坐标均为物理像素、虚拟桌面坐标系。
//!
//! - 显示器信息：xcap。
//! - 翻译框的实时截屏：用 `PrintWindow(PW_RENDERFULLCONTENT)` 直接让翻译框**下面的窗口**
//!   把自己画出来，再拼成该区域的画面。这样截到的永远是原始内容，不会把翻译框自己的译文截进去，
//!   也无需隐藏窗口（不闪烁）。
//!   （曾用 `WDA_EXCLUDEFROMCAPTURE` 排除翻译框，但它会让透明的 WebView2 窗口在屏幕上也不可见，已弃用。）

use image::RgbaImage;
use xcap::Monitor;

/// 显示器的位置与尺寸
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

pub fn monitor_at(x: i32, y: i32) -> Result<MonitorRect, String> {
    let m = Monitor::from_point(x, y).map_err(|e| format!("找不到显示器：{e}"))?;
    let err = |e: xcap::XCapError| e.to_string();
    Ok(MonitorRect {
        x: m.x().map_err(err)?,
        y: m.y().map_err(err)?,
        w: m.width().map_err(err)?,
        h: m.height().map_err(err)?,
    })
}

/// 截取某区域内"本程序窗口之下"的画面：按 Z 序找出与区域相交的其他程序窗口，
/// 从下往上逐个 `PrintWindow` 后拼到画布上。
///
/// 返回的图像与请求区域等大，坐标偏移恒为 (0, 0)（保留偏移字段，便于以后换成会裁剪的截屏方式）。
pub fn capture_under(x: i32, y: i32, w: u32, h: u32) -> Result<(RgbaImage, i32, i32), String> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::*;

    if w == 0 || h == 0 {
        return Err("截取区域为空".into());
    }
    let (rw, rh) = (w as i32, h as i32);
    let own_pid = std::process::id();

    // 1. 自顶向下收集与区域相交的窗口，直到遇到一个完全盖住区域的窗口
    let mut layers: Vec<(HWND, RECT)> = Vec::new();
    unsafe {
        let mut cur = GetTopWindow(None).ok();
        while let Some(hwnd) = cur {
            cur = GetWindow(hwnd, GW_HWNDNEXT).ok();
            if !IsWindowVisible(hwnd).as_bool() {
                continue;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == own_pid {
                continue;
            }
            let mut cloaked = 0u32;
            let is_cloaked = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as _, 4).is_ok() && cloaked != 0;
            if is_cloaked {
                continue;
            }
            let ex = WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE) as u32);
            // 鼠标穿透的悬浮层（别的程序的提示框、覆盖层等）不算内容
            if ex.contains(WS_EX_TRANSPARENT) {
                continue;
            }
            let mut r = RECT::default();
            if GetWindowRect(hwnd, &mut r).is_err() || r.right <= r.left || r.bottom <= r.top {
                continue;
            }
            if r.right <= x || r.left >= x + rw || r.bottom <= y || r.top >= y + rh {
                continue;
            }
            let covers = r.left <= x && r.top <= y && r.right >= x + rw && r.bottom >= y + rh;
            // 只盖住一部分的分层窗口多半是阴影/半透明特效，PrintWindow 画不出透明度，跳过
            if !covers && ex.contains(WS_EX_LAYERED) {
                continue;
            }
            layers.push((hwnd, r));
            if covers {
                break;
            }
        }
    }
    if layers.is_empty() {
        return Err("翻译框下面没有可截取的窗口".into());
    }

    // 2. 从下往上画到画布
    unsafe {
        let screen = GetDC(None);
        let canvas_dc = CreateCompatibleDC(Some(screen));
        let canvas = CreateCompatibleBitmap(screen, rw, rh);
        let old_canvas = SelectObject(canvas_dc, canvas.into());
        let _ = PatBlt(canvas_dc, 0, 0, rw, rh, BLACKNESS);
        for &(hwnd, r) in layers.iter().rev() {
            let (ww, wh) = (r.right - r.left, r.bottom - r.top);
            let win_dc = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, ww, wh);
            let old = SelectObject(win_dc, bmp.into());
            if PrintWindow(hwnd, win_dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool() {
                let _ = BitBlt(canvas_dc, r.left - x, r.top - y, ww, wh, Some(win_dc), 0, 0, SRCCOPY);
            }
            SelectObject(win_dc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(win_dc);
        }

        // 3. 读出像素（BGRA，自上而下）
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: rw,
                biHeight: -rh,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (rw * rh * 4) as usize];
        SelectObject(canvas_dc, old_canvas);
        let lines = GetDIBits(canvas_dc, canvas, 0, rh as u32, Some(buf.as_mut_ptr() as _), &mut info, DIB_RGB_COLORS);
        let _ = DeleteObject(canvas.into());
        let _ = DeleteDC(canvas_dc);
        ReleaseDC(None, screen);
        if lines == 0 {
            return Err("读取截图像素失败".into());
        }
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        let image = RgbaImage::from_raw(w, h, buf).ok_or("截图尺寸异常")?;
        Ok((image, 0, 0))
    }
}

/// 当前光标位置（物理像素）与鼠标左键是否按下
pub fn cursor_state() -> Option<(i32, i32, bool)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe {
        GetCursorPos(&mut p).ok()?;
        let down = GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0;
        Some((p.x, p.y, down))
    }
}

/// 是否按着翻页类按键（PgUp/PgDn/方向上下/空格/Home/End）。只查询这几个键的当前状态，不记录任何输入。
pub fn nav_key_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    [VK_PRIOR, VK_NEXT, VK_UP, VK_DOWN, VK_SPACE, VK_HOME, VK_END]
        .iter()
        .any(|k| unsafe { GetAsyncKeyState(k.0 as i32) } < 0)
}

/// 测试用：直接让指定窗口把自己画出来（窗口可以在后台被遮挡，不必切到前台）
#[cfg(test)]
pub fn capture_window(hwnd_raw: isize) -> Result<RgbaImage, String> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let hwnd = HWND(hwnd_raw as _);
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).map_err(|e| e.to_string())?;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bmp.into());
        let ok = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        SelectObject(dc, old);
        GetDIBits(dc, bmp, 0, h as u32, Some(buf.as_mut_ptr() as _), &mut info, DIB_RGB_COLORS);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        if !ok {
            return Err("PrintWindow 失败".into());
        }
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        RgbaImage::from_raw(w as u32, h as u32, buf).ok_or_else(|| "尺寸异常".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真机实测：截取屏幕中央区域下层窗口画面并存成 PNG。
    /// TF_SHOT=输出路径 cargo test 下层窗口截取 -- --ignored --nocapture
    #[test]
    #[ignore]
    fn 下层窗口截取() {
        let out = std::env::var("TF_SHOT").unwrap_or_else(|_| "under.png".into());
        let (x, y, w, h) = std::env::var("TF_RECT")
            .ok()
            .and_then(|s| {
                let v: Vec<i32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                (v.len() == 4).then(|| (v[0], v[1], v[2] as u32, v[3] as u32))
            })
            .unwrap_or((400, 300, 600, 400));
        let t = std::time::Instant::now();
        let (img, _, _) = capture_under(x, y, w, h).unwrap();
        println!("耗时 {:?}", t.elapsed());
        img.save(&out).unwrap();
    }
}
