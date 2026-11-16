//! 屏幕截取（xcap）。所有坐标均为物理像素、虚拟桌面坐标系。

use image::RgbaImage;
use xcap::Monitor;

/// 按下快捷键瞬间"冻结"的整屏截图，框选完成后从中裁剪选区
pub struct Frozen {
    /// 显示器左上角在虚拟桌面中的位置
    pub mon_x: i32,
    pub mon_y: i32,
    pub image: RgbaImage,
}

impl Frozen {
    pub fn width(&self) -> u32 {
        self.image.width()
    }

    pub fn height(&self) -> u32 {
        self.image.height()
    }
}

/// 截取包含点 (x, y) 的那块显示器
pub fn freeze_at(x: i32, y: i32) -> Result<Frozen, String> {
    let monitor = Monitor::from_point(x, y).map_err(|e| format!("找不到光标所在显示器：{e}"))?;
    let mon_x = monitor.x().map_err(|e| e.to_string())?;
    let mon_y = monitor.y().map_err(|e| e.to_string())?;
    let image = monitor
        .capture_image()
        .map_err(|e| format!("截屏失败：{e}"))?;
    Ok(Frozen { mon_x, mon_y, image })
}

/// 从图像中裁剪矩形，超出边界的部分自动截断；结果为空时返回 None
pub fn crop(image: &RgbaImage, x: i32, y: i32, w: u32, h: u32) -> Option<RgbaImage> {
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = ((x as i64 + w as i64).max(0) as u32).min(image.width());
    let y1 = ((y as i64 + h as i64).max(0) as u32).min(image.height());
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(image::imageops::crop_imm(image, x0, y0, x1 - x0, y1 - y0).to_image())
}

/// 截取虚拟桌面上的一块区域（区域左上角所在的显示器）
pub fn capture_region(x: i32, y: i32, w: u32, h: u32) -> Result<RgbaImage, String> {
    let frozen = freeze_at(x, y)?;
    crop(&frozen.image, x - frozen.mon_x, y - frozen.mon_y, w, h).ok_or_else(|| "截取区域为空".into())
}
