//! Windows.Graphics.Capture（WGC）按窗口截取：翻译框跟踪滚动用的低延迟截屏。
//!
//! 与 `PrintWindow` 相比：画面由 DWM 合成后直接交给我们（GPU 纹理），不需要目标窗口重新绘制一遍；
//! 窗口内容变化时才来新帧（`FrameArrived`），帧率跟显示器刷新率走，每帧带合成时刻；
//! 只回读框内区域，一帧几毫秒。按窗口截取不受遮挡影响，天然不含翻译框自己。
//!
//! 限制：Windows 10 上被截取的窗口会出现系统强制的黄色边框（Windows 11 才能关掉），
//! 所以默认只在能关掉边框的系统上启用，其余继续用 `PrintWindow`。
//! 调试时可用环境变量 `TA_CAPTURE=wgc` / `TA_CAPTURE=printwindow` 强制选择。

use image::RgbaImage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use windows::Foundation::Metadata::ApiInformation;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureAccess, GraphicsCaptureAccessKind,
    GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::{HSTRING, IInspectable, Interface};

/// 帧池缓冲数：手里留一帧备用，另外两块接收新帧
const BUFFERS: i32 = 3;
/// 新建会话后最多等第一帧这么久
const FIRST_FRAME_WAIT: Duration = Duration::from_millis(150);

/// 关不掉边框等原因，本次运行不再使用 WGC
static DISABLED: AtomicBool = AtomicBool::new(false);

fn forced() -> Option<bool> {
    match std::env::var("TA_CAPTURE").ok()?.to_ascii_lowercase().as_str() {
        "wgc" => Some(true),
        "printwindow" | "gdi" => Some(false),
        _ => None,
    }
}

/// 是否使用 WGC 截取翻译框下面的窗口
pub fn available() -> bool {
    static AVAIL: OnceLock<bool> = OnceLock::new();
    let avail = *AVAIL.get_or_init(|| {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let supported = GraphicsCaptureSession::IsSupported().unwrap_or(false);
        match forced() {
            Some(f) => f && supported,
            None => supported && borderless_supported(),
        }
    });
    avail && !DISABLED.load(Ordering::Relaxed)
}

/// 系统能否关掉截取边框（Windows 11 起）；顺带申请无边框权限（非打包程序会直接放行）
fn borderless_supported() -> bool {
    let present = ApiInformation::IsPropertyPresent(
        &HSTRING::from("Windows.Graphics.Capture.GraphicsCaptureSession"),
        &HSTRING::from("IsBorderRequired"),
    )
    .unwrap_or(false);
    if present {
        if let Ok(op) = GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless) {
            let _ = op.join();
        }
    }
    present
}

/// 当前时刻（100 纳秒为单位，与帧的 `SystemRelativeTime` 同一时间轴）
fn qpc_now_100ns() -> i64 {
    let (mut c, mut f) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
        let _ = QueryPerformanceFrequency(&mut f);
    }
    if f == 0 {
        return 0;
    }
    (c as i128 * 10_000_000 / f as i128) as i64
}

/// 某个窗口的持续截取会话
pub struct WindowCapture {
    pub hwnd: isize,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    d3d: IDirect3DDevice,
    _item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    size: SizeInt32,
    token: i64,
    /// 最新一帧及其合成时刻（留在手里，下次即使没有新帧也能按新区域回读）
    held: Option<(Direct3D11CaptureFrame, Instant)>,
    /// 回读用的暂存纹理及尺寸
    staging: Option<(ID3D11Texture2D, u32, u32)>,
}

// D3D 设备上下文只在持有方的互斥锁内使用；WinRT 对象均为自由线程
unsafe impl Send for WindowCapture {}

impl WindowCapture {
    /// 开始截取窗口；`on_frame` 在每次有新帧时调用（在系统线程上）
    pub fn new(hwnd: isize, on_frame: Arc<dyn Fn() + Send + Sync>) -> Result<Self, String> {
        let err = |e: windows::core::Error| e.to_string();
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(err)?;
            let device: ID3D11Device = device.ok_or("D3D11 device")?;
            let context: ID3D11DeviceContext = context.ok_or("D3D11 context")?;
            let dxgi: IDXGIDevice = device.cast().map_err(err)?;
            let d3d: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi).map_err(err)?.cast().map_err(err)?;

            let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(err)?;
            let item: GraphicsCaptureItem = interop.CreateForWindow(HWND(hwnd as _)).map_err(err)?;
            let size = item.Size().map_err(err)?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &d3d,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                BUFFERS,
                size,
            )
            .map_err(err)?;
            let token = pool
                .FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_, _| {
                    on_frame();
                    Ok(())
                }))
                .map_err(err)?;
            let session = pool.CreateCaptureSession(&item).map_err(err)?;
            // 不截鼠标指针：指针移动不算画面变化
            let _ = session.SetIsCursorCaptureEnabled(false);
            if session.SetIsBorderRequired(false).is_err() && forced() != Some(true) {
                // 关不掉黄色边框：本次运行改用 PrintWindow
                DISABLED.store(true, Ordering::Relaxed);
                let _ = session.Close();
                let _ = pool.Close();
                return Err("WGC border".into());
            }
            session.StartCapture().map_err(err)?;

            let mut cap = Self {
                hwnd,
                device,
                context,
                d3d,
                _item: item,
                pool,
                session,
                size,
                token,
                held: None,
                staging: None,
            };
            // 等第一帧，免得刚开始的几次截取都要回退到 PrintWindow
            let t = Instant::now();
            while cap.held.is_none() && t.elapsed() < FIRST_FRAME_WAIT {
                cap.drain()?;
                if cap.held.is_none() {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            Ok(cap)
        }
    }

    /// 取走帧池里所有新帧，只留最新的一帧
    fn drain(&mut self) -> Result<bool, String> {
        let mut newest: Option<Direct3D11CaptureFrame> = None;
        while let Ok(f) = self.pool.TryGetNextFrame() {
            if let Some(old) = newest.replace(f) {
                let _ = old.Close();
            }
        }
        let Some(frame) = newest else { return Ok(false) };
        // 帧的合成时刻换算成 Instant
        let at = frame
            .SystemRelativeTime()
            .ok()
            .map(|t| (qpc_now_100ns() - t.Duration).clamp(0, 10_000_000))
            .and_then(|age| Instant::now().checked_sub(Duration::from_nanos(age as u64 * 100)))
            .unwrap_or_else(Instant::now);
        // 窗口尺寸变了：按新尺寸重建帧池（这一帧仍按旧纹理读取）
        if let Ok(cs) = frame.ContentSize() {
            if cs.Width != self.size.Width || cs.Height != self.size.Height {
                self.size = cs;
                self.pool
                    .Recreate(&self.d3d, DirectXPixelFormat::B8G8R8A8UIntNormalized, BUFFERS, cs)
                    .map_err(|e| e.to_string())?;
            }
        }
        if let Some((old, _)) = self.held.replace((frame, at)) {
            let _ = old.Close();
        }
        Ok(true)
    }

    /// 截取屏幕区域（物理像素，须完全落在窗口内）。返回 None 表示还没有任何帧
    pub fn grab(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<Option<(RgbaImage, Instant)>, String> {
        self.drain()?;
        let Some((frame, at)) = &self.held else { return Ok(None) };
        let err = |e: windows::core::Error| e.to_string();
        unsafe {
            // 帧画面的原点是窗口的可见边界（不含透明的缩放边框与阴影）
            let mut r = RECT::default();
            DwmGetWindowAttribute(
                HWND(self.hwnd as _),
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut r as *mut _ as _,
                std::mem::size_of::<RECT>() as u32,
            )
            .map_err(err)?;
            let content = frame.ContentSize().map_err(err)?;
            let (sx, sy) = (x - r.left, y - r.top);
            if sx < 0 || sy < 0 || sx + w as i32 > content.Width || sy + h as i32 > content.Height {
                return Err("region outside window".into());
            }

            let access: IDirect3DDxgiInterfaceAccess = frame.Surface().map_err(err)?.cast().map_err(err)?;
            let texture: ID3D11Texture2D = access.GetInterface().map_err(err)?;
            if self.staging.as_ref().is_none_or(|s| s.1 != w || s.2 != h) {
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: w,
                    Height: h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut t = None;
                self.device.CreateTexture2D(&desc, None, Some(&mut t)).map_err(err)?;
                self.staging = Some((t.ok_or("staging texture")?, w, h));
            }
            let staging = &self.staging.as_ref().unwrap().0;
            let src = D3D11_BOX {
                left: sx as u32,
                top: sy as u32,
                front: 0,
                right: sx as u32 + w,
                bottom: sy as u32 + h,
                back: 1,
            };
            self.context.CopySubresourceRegion(staging, 0, 0, 0, 0, &texture, 0, Some(&src));
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).map_err(err)?;
            let row = (w * 4) as usize;
            let mut buf = vec![0u8; row * h as usize];
            for (i, dst) in buf.chunks_exact_mut(row).enumerate() {
                let src = std::slice::from_raw_parts((mapped.pData as *const u8).add(i * mapped.RowPitch as usize), row);
                dst.copy_from_slice(src);
            }
            self.context.Unmap(staging, 0);
            for px in buf.chunks_exact_mut(4) {
                px.swap(0, 2);
                px[3] = 255;
            }
            let image = RgbaImage::from_raw(w, h, buf).ok_or("bad size")?;
            Ok(Some((image, *at)))
        }
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        let _ = self.pool.RemoveFrameArrived(self.token);
        if let Some((f, _)) = self.held.take() {
            let _ = f.Close();
        }
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
