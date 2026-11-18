//! 版面分析：把 OCR 的行/单词归并成"段落块"，并为每块取样背景色与文字色，
//! 供前端按原位置覆盖渲染译文（沉浸式效果）。

use crate::ocr::{Line, OcrOutput, Word};
use image::RgbaImage;
use serde::Serialize;

/// 一行内单词间距超过 行高 × 该值 时拆成两段（多栏、表格单元格）
const SPLIT_GAP: f32 = 1.6;
/// 相邻两行的行高比在此范围内才可能属于同一段
const HEIGHT_RATIO: (f32, f32) = (0.7, 1.45);
/// 相邻两行的垂直间距不超过 行高 × 该值 才属于同一段
const MAX_LINE_GAP: f32 = 0.9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn right(&self) -> f32 {
        self.x + self.w
    }
    fn bottom(&self) -> f32 {
        self.y + self.h
    }
    fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect {
            x,
            y,
            w: self.right().max(o.right()) - x,
            h: self.bottom().max(o.bottom()) - y,
        }
    }
}

/// 一行中的一个片段（按大间距拆开后的连续单词）
#[derive(Debug, Clone)]
struct Segment {
    text: String,
    rect: Rect,
}

/// 段落块：前端在 `rect` 处盖上 `bg` 色块并以 `fg` 色显示译文
#[derive(Debug, Clone, Serialize)]
pub struct Block {
    /// 原文（已合并成一段）
    pub source: String,
    /// 物理像素，相对截图左上角
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// 行数与平均行高，前端据此估算字号
    pub lines: u32,
    pub line_h: f32,
    /// `#rrggbb`
    pub bg: String,
    pub fg: String,
}

/// OCR 结果 → 段落块；`skip_cjk` 为真时跳过本身就是中日文的块（目标语言为中文时不必翻译）
pub fn build_blocks(ocr: &OcrOutput, image: &RgbaImage, skip_cjk: bool) -> Vec<Block> {
    let (iw, ih) = (image.width() as f32, image.height() as f32);
    let mut segments: Vec<Segment> = ocr
        .lines
        .iter()
        .flat_map(|l| split_line(l, ocr.cjk))
        // 贴着上下边缘的行多半被截断，识别结果不可靠
        .filter(|s| s.rect.y > 1.0 && s.rect.bottom() < ih - 1.0)
        .collect();
    segments.sort_by(|a, b| a.rect.y.total_cmp(&b.rect.y));

    // 贪心归并：每个片段找一个"正下方紧邻、行高相近、水平有重叠"的块接上
    let mut groups: Vec<Vec<Segment>> = Vec::new();
    for seg in segments {
        let h = seg.rect.h;
        let target = groups.iter_mut().rev().find(|g| {
            let last = g.last().unwrap();
            let ratio = h / last.rect.h;
            let gap = seg.rect.y - last.rect.bottom();
            let overlap = seg.rect.right().min(last.rect.right()) - seg.rect.x.max(last.rect.x);
            ratio > HEIGHT_RATIO.0
                && ratio < HEIGHT_RATIO.1
                && gap > -0.3 * h
                && gap < MAX_LINE_GAP * h
                && overlap > 0.0
        });
        match target {
            Some(g) => g.push(seg),
            None => groups.push(vec![seg]),
        }
    }

    groups
        .into_iter()
        .filter_map(|g| {
            let rect = g.iter().skip(1).fold(g[0].rect, |r, s| r.union(&s.rect));
            let source = join_lines(g.iter().map(|s| s.text.as_str()), ocr.cjk);
            if !needs_translation(&source, skip_cjk) {
                return None;
            }
            let lines = g.len() as u32;
            let line_h = g.iter().map(|s| s.rect.h).sum::<f32>() / lines as f32;
            // 色块四周略微外扩，确保盖住原文笔画
            let pad = (line_h * 0.12).clamp(1.0, 4.0);
            let x = (rect.x - pad).max(0.0);
            let y = (rect.y - pad).max(0.0);
            let cover = Rect {
                x,
                y,
                w: (rect.right() + pad).min(iw) - x,
                h: (rect.bottom() + pad).min(ih) - y,
            };
            let (bg, fg) = sample_colors(image, &cover);
            Some(Block {
                source,
                x: cover.x,
                y: cover.y,
                w: cover.w,
                h: cover.h,
                lines,
                line_h,
                bg: hex(bg),
                fg: hex(fg),
            })
        })
        .collect()
}

/// 按单词间的大间距把一行拆成多个片段
fn split_line(line: &Line, cjk: bool) -> Vec<Segment> {
    let line_h = line.words.iter().map(|w| w.h).fold(0.0, f32::max).max(1.0);
    let mut out = Vec::new();
    let mut cur: Vec<&Word> = Vec::new();
    for w in &line.words {
        if let Some(prev) = cur.last() {
            if w.x - (prev.x + prev.w) > SPLIT_GAP * line_h {
                out.push(make_segment(&cur, cjk));
                cur.clear();
            }
        }
        cur.push(w);
    }
    if !cur.is_empty() {
        out.push(make_segment(&cur, cjk));
    }
    out
}

fn make_segment(words: &[&Word], cjk: bool) -> Segment {
    let mut rect = Rect {
        x: words[0].x,
        y: words[0].y,
        w: words[0].w,
        h: words[0].h,
    };
    let mut text = String::new();
    for w in words {
        rect = rect.union(&Rect {
            x: w.x,
            y: w.y,
            w: w.w,
            h: w.h,
        });
        push_word(&mut text, &w.text, cjk);
    }
    Segment { text, rect }
}

/// 拼接单词：中日文字符之间不加空格
fn push_word(out: &mut String, word: &str, cjk: bool) {
    let wide = |c: char| !c.is_ascii();
    let need_space = !out.is_empty()
        && !(cjk
            && out.chars().last().is_some_and(wide)
            && word.chars().next().is_some_and(wide));
    if need_space {
        out.push(' ');
    }
    out.push_str(word);
}

/// 段内多行合并成一段：英文行尾连字符断词时去掉连字符
fn join_lines<'a>(lines: impl Iterator<Item = &'a str>, cjk: bool) -> String {
    let mut out = String::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if out.ends_with('-') && line.chars().next().is_some_and(char::is_lowercase) {
            out.pop();
            out.push_str(line);
        } else {
            push_word(&mut out, line, cjk);
        }
    }
    out
}

/// 纯数字/符号、或本身已是中日文（目标中文时）的块不翻译
fn needs_translation(text: &str, skip_cjk: bool) -> bool {
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    if letters == 0 {
        return false;
    }
    if skip_cjk {
        let cjk = text.chars().filter(|c| is_cjk_char(*c)).count();
        if cjk * 2 >= letters {
            return false;
        }
    }
    true
}

fn is_cjk_char(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

type Rgb = [u8; 3];

/// 背景色 = 外框一圈像素的中位数；文字色 = 框内与背景差异最大的那部分像素的平均值
fn sample_colors(image: &RgbaImage, r: &Rect) -> (Rgb, Rgb) {
    let (iw, ih) = image.dimensions();
    let x0 = (r.x.max(0.0) as u32).min(iw.saturating_sub(1));
    let y0 = (r.y.max(0.0) as u32).min(ih.saturating_sub(1));
    let x1 = (r.right().ceil() as u32).clamp(x0 + 1, iw);
    let y1 = (r.bottom().ceil() as u32).clamp(y0 + 1, ih);
    let px = |x: u32, y: u32| {
        let p = image.get_pixel(x, y).0;
        [p[0], p[1], p[2]]
    };

    let mut border = Vec::new();
    let step = ((x1 - x0) / 64).max(1);
    for x in (x0..x1).step_by(step as usize) {
        border.push(px(x, y0));
        border.push(px(x, y1 - 1));
    }
    let step = ((y1 - y0) / 32).max(1);
    for y in (y0..y1).step_by(step as usize) {
        border.push(px(x0, y));
        border.push(px(x1 - 1, y));
    }
    let bg = median(&mut border);

    // 内部稀疏采样，取与背景差异最大的前 15% 作为文字色
    let mut inner: Vec<(u32, Rgb)> = Vec::new();
    let sx = ((x1 - x0) / 80).max(1);
    let sy = ((y1 - y0) / 30).max(1);
    for y in (y0..y1).step_by(sy as usize) {
        for x in (x0..x1).step_by(sx as usize) {
            let c = px(x, y);
            inner.push((dist(c, bg), c));
        }
    }
    inner.sort_by(|a, b| b.0.cmp(&a.0));
    let take = (inner.len() * 15 / 100).max(1).min(inner.len());
    let fg = if take == 0 || inner[0].0 < 40 {
        // 几乎没有对比：按背景亮度选黑或白
        if luma(bg) > 128 { [0, 0, 0] } else { [255, 255, 255] }
    } else {
        let mut sum = [0u32; 3];
        for (_, c) in &inner[..take] {
            for i in 0..3 {
                sum[i] += c[i] as u32;
            }
        }
        [0, 1, 2].map(|i| (sum[i] / take as u32) as u8)
    };
    (bg, fg)
}

fn median(colors: &mut [Rgb]) -> Rgb {
    if colors.is_empty() {
        return [255, 255, 255];
    }
    let mid = colors.len() / 2;
    [0, 1, 2].map(|i| {
        colors.sort_by_key(|c| c[i]);
        colors[mid][i]
    })
}

fn dist(a: Rgb, b: Rgb) -> u32 {
    (0..3).map(|i| (a[i] as i32 - b[i] as i32).unsigned_abs()).sum()
}

fn luma(c: Rgb) -> u32 {
    (c[0] as u32 * 299 + c[1] as u32 * 587 + c[2] as u32 * 114) / 1000
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn word(text: &str, x: f32, y: f32, w: f32) -> Word {
        Word {
            text: text.into(),
            x,
            y,
            w,
            h: 16.0,
        }
    }

    #[test]
    fn 段落归并与分栏() {
        // 左栏两行属于同一段；右栏与左栏间距大，单独成段；下方隔得远的另起一段
        let ocr = OcrOutput {
            cjk: false,
            lines: vec![
                Line {
                    words: vec![word("Hello", 10.0, 10.0, 50.0), word("wor-", 65.0, 10.0, 40.0), word("Right", 300.0, 10.0, 50.0)],
                },
                Line {
                    words: vec![word("ld", 10.0, 30.0, 20.0), word("again.", 35.0, 30.0, 60.0)],
                },
                Line {
                    words: vec![word("Next", 10.0, 120.0, 40.0)],
                },
            ],
        };
        let img = RgbaImage::from_pixel(400, 200, Rgba([255, 255, 255, 255]));
        let blocks = build_blocks(&ocr, &img, true);
        let texts: Vec<_> = blocks.iter().map(|b| b.source.as_str()).collect();
        assert_eq!(texts, ["Hello world again.", "Right", "Next"]);
        assert_eq!(blocks[0].lines, 2);
        assert_eq!(blocks[0].bg, "#ffffff");
    }

    #[test]
    fn 跳过无需翻译的块() {
        assert!(!needs_translation("123 - 456", false));
        assert!(!needs_translation("已经是中文了", true));
        assert!(needs_translation("Hello", true));
    }

    #[test]
    fn 取样颜色() {
        let mut img = RgbaImage::from_pixel(100, 40, Rgba([30, 30, 30, 255]));
        for x in 20..80 {
            for y in 15..25 {
                img.put_pixel(x, y, Rgba([240, 240, 240, 255]));
            }
        }
        let (bg, fg) = sample_colors(&img, &Rect { x: 0.0, y: 0.0, w: 100.0, h: 40.0 });
        assert_eq!(bg, [30, 30, 30]);
        assert_eq!(fg, [240, 240, 240]);
    }
}
