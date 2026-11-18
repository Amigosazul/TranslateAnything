//! Windows 原生 OCR（Windows.Media.Ocr）封装。离线、免依赖。
//!
//! 调用会阻塞当前线程，请在 `spawn_blocking` 中使用。

use image::RgbaImage;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::HSTRING;

/// 文字高度太小时 OCR 识别率很差，图像高度低于该值（物理像素）先放大
const UPSCALE_BELOW: u32 = 48;

/// 一个识别出的单词，坐标为原图物理像素
#[derive(Debug, Clone)]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// 一行文字
#[derive(Debug, Clone)]
pub struct Line {
    pub words: Vec<Word>,
}

#[derive(Debug, Clone)]
pub struct OcrOutput {
    pub lines: Vec<Line>,
    /// 识别语言是否为中日文（拼接单词时不加空格）
    pub cjk: bool,
}

/// 识别图像中的文字（含位置）
///
/// - `lang_pref`：`auto` 或空 = 自动挑选；否则为语言标签，如 `en-US`
/// - `target_lang`：翻译目标语言，自动模式下优先选一个与之不同的 OCR 语言
pub fn recognize(image: &RgbaImage, lang_pref: &str, target_lang: &str) -> Result<OcrOutput, String> {
    // 当前线程可能还没初始化 WinRT；重复初始化返回的错误可以忽略
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    recognize_inner(image, lang_pref, target_lang).map_err(|e| format!("OCR 失败：{e}"))
}

fn recognize_inner(
    image: &RgbaImage,
    lang_pref: &str,
    target_lang: &str,
) -> windows::core::Result<OcrOutput> {
    let (engine, tag) = create_engine(lang_pref, target_lang)?;
    let (scaled, scale) = fit_size(image, OcrEngine::MaxImageDimension()?);
    let (w, h) = scaled.dimensions();

    // RGBA → BGRA
    let mut bytes = scaled.into_raw();
    for px in bytes.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let writer = DataWriter::new()?;
    writer.WriteBytes(&bytes)?;
    let buffer = writer.DetachBuffer()?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        w as i32,
        h as i32,
        BitmapAlphaMode::Premultiplied,
    )?;

    let result = engine.RecognizeAsync(&bitmap)?.join()?;
    let mut lines = Vec::new();
    for line in result.Lines()? {
        let mut words = Vec::new();
        for word in line.Words()? {
            let r = word.BoundingRect()?;
            words.push(Word {
                text: word.Text()?.to_string_lossy(),
                x: r.X / scale,
                y: r.Y / scale,
                w: r.Width / scale,
                h: r.Height / scale,
            });
        }
        if !words.is_empty() {
            lines.push(Line { words });
        }
    }
    Ok(OcrOutput {
        lines,
        cjk: is_cjk_lang(&tag),
    })
}

/// 选 OCR 引擎，返回（引擎，语言标签）
fn create_engine(lang_pref: &str, target_lang: &str) -> windows::core::Result<(OcrEngine, String)> {
    let pref = lang_pref.trim();
    if !pref.is_empty() && !pref.eq_ignore_ascii_case("auto") {
        let lang = Language::CreateLanguage(&HSTRING::from(pref))?;
        if !OcrEngine::IsLanguageSupported(&lang)? {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x80004005u32 as i32),
                format!("系统未安装 {pref} 的 OCR 语言包"),
            ));
        }
        return Ok((OcrEngine::TryCreateFromLanguage(&lang)?, pref.to_string()));
    }

    // 自动：优先选一个与目标语言不同的已安装 OCR 语言（例如目标中文时优先英文）
    let target_primary = target_lang.split('-').next().unwrap_or("").to_ascii_lowercase();
    for lang in OcrEngine::AvailableRecognizerLanguages()? {
        let tag = lang.LanguageTag()?.to_string_lossy();
        let primary = tag.split('-').next().unwrap_or("").to_ascii_lowercase();
        if primary != target_primary {
            return Ok((OcrEngine::TryCreateFromLanguage(&lang)?, tag));
        }
    }
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
    let tag = engine.RecognizerLanguage()?.LanguageTag()?.to_string_lossy();
    Ok((engine, tag))
}

/// 太大则缩小到 OCR 上限内，太小则放大以提高识别率；返回（图像，缩放倍数）
fn fit_size(image: &RgbaImage, max_dim: u32) -> (RgbaImage, f32) {
    use image::imageops::{FilterType, resize};
    let (w, h) = image.dimensions();
    let longest = w.max(h);
    if longest > max_dim {
        let s = max_dim as f32 / longest as f32;
        let (nw, nh) = (((w as f32 * s) as u32).max(1), ((h as f32 * s) as u32).max(1));
        return (resize(image, nw, nh, FilterType::Triangle), nw as f32 / w as f32);
    }
    if h < UPSCALE_BELOW {
        let s = (UPSCALE_BELOW.div_ceil(h.max(1))).min(4);
        if longest * s <= max_dim {
            return (resize(image, w * s, h * s, FilterType::CatmullRom), s as f32);
        }
    }
    (image.clone(), 1.0)
}

fn is_cjk_lang(tag: &str) -> bool {
    let t = tag.to_ascii_lowercase();
    t.starts_with("zh") || t.starts_with("ja")
}
