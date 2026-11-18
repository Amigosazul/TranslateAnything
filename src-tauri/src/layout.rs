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
/// 相邻两行文字色差（RGB 各通道差之和）超过该值视为不同组件（如深色标题 + 灰色说明）
const MAX_COLOR_DIFF: u32 = 80;
/// 相邻两行估算字号之比在此范围内才可能属于同一段（大小标题、标题与正文分开）
const FONT_RATIO: (f32, f32) = (0.85, 1.18);

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
    /// 各单词的（文字，框高），用于估算字号
    words: Vec<(String, f32)>,
    /// 该行的文字色与估算字号（归并时用来区分组件）
    fg: Rgb,
    em: f32,
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
    /// 行数与平均行高（OCR 框高）
    pub lines: u32,
    pub line_h: f32,
    /// 估算出的原文字号（em，物理像素）：译文用同样的字号显示
    pub font: f32,
    /// 原文行距（相邻两行基线间距，物理像素）；单行块为 0，前端让文字在块内垂直居中
    pub pitch: f32,
    /// 多行块首行的上留白（物理像素），让译文首行与原文首行对齐
    pub pad_top: f32,
    /// 字重：400 常规 / 600 半粗 / 700 粗体
    pub weight: u16,
    /// 对齐方式：`left` / `center`
    pub align: &'static str,
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
    for seg in &mut segments {
        let r = &seg.rect;
        let probe = Rect { x: r.x - 2.0, y: r.y - 2.0, w: r.w + 4.0, h: r.h + 4.0 };
        seg.fg = sample_colors(image, &probe).1;
        seg.em = estimate_font(std::slice::from_ref(seg), r.h, ocr.cjk);
    }
    segments.sort_by(|a, b| a.rect.y.total_cmp(&b.rect.y));

    // 贪心归并：每个片段找一个"正下方紧邻、行高相近、字号相近、颜色相同、水平有重叠"的块接上
    let mut groups: Vec<Vec<Segment>> = Vec::new();
    for seg in segments {
        let h = seg.rect.h;
        let target = groups.iter_mut().rev().find(|g| {
            let last = g.last().unwrap();
            let ratio = h / last.rect.h;
            let gap = seg.rect.y - last.rect.bottom();
            let overlap = seg.rect.right().min(last.rect.right()) - seg.rect.x.max(last.rect.x);
            let font_ratio = seg.em / last.em.max(1.0);
            ratio > HEIGHT_RATIO.0
                && ratio < HEIGHT_RATIO.1
                && font_ratio > FONT_RATIO.0
                && font_ratio < FONT_RATIO.1
                && dist(seg.fg, last.fg) <= MAX_COLOR_DIFF
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
            // 色块四周外扩，确保完整盖住原文笔画（OCR 框不一定包住升部、降部和抗锯齿边缘）
            let pad = (line_h * 0.18).clamp(2.0, 5.0);
            let x = (rect.x - pad).max(0.0);
            let y = (rect.y - pad).max(0.0);
            let cover = Rect {
                x,
                y,
                w: (rect.right() + pad).min(iw) - x,
                h: (rect.bottom() + pad).min(ih) - y,
            };
            let (bg, fg) = sample_colors(image, &cover);
            let font = estimate_font(&g, line_h, ocr.cjk);
            let pitch = if lines > 1 { line_pitch(&g) } else { 0.0 };
            // 首行行框中心对齐原文首行中心
            let first_center = g[0].rect.y + g[0].rect.h / 2.0 - cover.y;
            let pad_top = if lines > 1 { (first_center - pitch / 2.0).max(0.0) } else { 0.0 };
            let stroke = stroke_width(image, &rect, bg, fg);
            let weight = weight_from_stroke(stroke, font, luma(fg) > luma(bg));
            Some(Block {
                source,
                x: cover.x,
                y: cover.y,
                w: cover.w,
                h: cover.h,
                lines,
                line_h,
                font,
                pitch,
                pad_top,
                weight,
                align: if lines > 1 { detect_align(&g) } else { boxed_align(image, &cover, bg, font) },
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
    let mut metrics = Vec::with_capacity(words.len());
    for w in words {
        metrics.push((w.text.clone(), w.h));
        rect = rect.union(&Rect {
            x: w.x,
            y: w.y,
            w: w.w,
            h: w.h,
        });
        push_word(&mut text, &w.text, cjk);
    }
    Segment {
        text,
        rect,
        words: metrics,
        fg: [0, 0, 0],
        em: 0.0,
    }
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

/// 由笔画宽度判断字重。实测（Segoe UI / Georgia，12~28px）笔画宽度 / 字号：
/// 浅底深字：常规体 ≈ 0.08~0.09，半粗 ≈ 0.11，粗体 ≈ 0.15~0.17；
/// 深底浅字的抗锯齿会让笔画显得更粗，同字重的比例约为浅底的 1.25 倍，先折算回来。
fn weight_from_stroke(stroke: f32, em: f32, light_on_dark: bool) -> u16 {
    let mut ratio = stroke / em.max(1.0);
    if light_on_dark {
        ratio /= 1.25;
    }
    if ratio > 0.13 {
        700
    } else if ratio > 0.10 {
        600
    } else {
        400
    }
}

/// 根据单词由哪些字母组成，推算其框高占字号（em）的比例。
/// 例如 "mean" 只有 x 高度（≈0.52em），"The" 到大写高度（≈0.73em），"lightly" 上下都伸出（≈0.94em）。
/// 含括号等高度不确定的符号时返回 None。
fn height_ratio(word: &str, cjk: bool) -> Option<f32> {
    let word = word.trim_matches(|c: char| matches!(c, '.' | ',' | ':' | ';' | '!' | '?' | '"' | '\u{201c}' | '\u{201d}'));
    if word.is_empty() {
        return None;
    }
    if word.chars().any(is_cjk_char) || (cjk && word.chars().any(|c| !c.is_ascii())) {
        // 中日文字形基本占满字身框
        return Some(0.88);
    }
    let (mut asc, mut desc) = (false, false);
    for c in word.chars() {
        match c {
            'a' | 'c' | 'e' | 'm' | 'n' | 'o' | 'r' | 's' | 'u' | 'v' | 'w' | 'x' | 'z' | '-' | '\u{2013}' => {}
            'g' | 'p' | 'q' | 'y' => desc = true,
            'j' => {
                asc = true;
                desc = true;
            }
            'A'..='Z' | '0'..='9' | 'b' | 'd' | 'f' | 'h' | 'i' | 'k' | 'l' | 't' | '\'' | '\u{2019}' | '%' | '&' | '#' => {
                asc = true
            }
            _ => return None,
        }
    }
    Some(match (asc, desc) {
        (true, true) => 0.93,
        (true, false) => 0.73,
        (false, true) => 0.72,
        (false, false) => 0.52,
    })
}

/// 估算段落原文字号（em，物理像素）：逐词按字母组成换算后取中位数
fn estimate_font(group: &[Segment], line_h: f32, cjk: bool) -> f32 {
    let mut ems: Vec<f32> = group
        .iter()
        .flat_map(|s| &s.words)
        .filter_map(|(t, h)| height_ratio(t, cjk).map(|r| h / r))
        .collect();
    if ems.is_empty() {
        // 没有可用的单词：按"整行框高 ≈ 0.9em"估
        return line_h / 0.9;
    }
    ems.sort_by(f32::total_cmp);
    let em = ems[ems.len() / 2];
    // 防止个别识别异常把字号带偏太多
    em.clamp(line_h * 0.9, line_h * 2.0)
}

/// 多行段落的行距：相邻行顶部间距的中位数
fn line_pitch(group: &[Segment]) -> f32 {
    let mut d: Vec<f32> = group.windows(2).map(|w| w[1].rect.y - w[0].rect.y).filter(|d| *d > 0.0).collect();
    if d.is_empty() {
        return 0.0;
    }
    d.sort_by(f32::total_cmp);
    d[d.len() / 2]
}

/// 多行段落：各行左边缘参差、但中心对齐时判为居中
fn detect_align(group: &[Segment]) -> &'static str {
    if group.len() < 2 {
        return "left";
    }
    let spread = |v: Vec<f32>| {
        let lo = v.iter().cloned().fold(f32::MAX, f32::min);
        let hi = v.iter().cloned().fold(f32::MIN, f32::max);
        hi - lo
    };
    let h = group[0].rect.h;
    let lefts = spread(group.iter().map(|s| s.rect.x).collect());
    let centers = spread(group.iter().map(|s| s.rect.x + s.rect.w / 2.0).collect());
    if lefts > h && centers < h * 0.5 { "center" } else { "left" }
}

/// 单行文字：从块的左右边缘沿水平方向往外找第一个与背景色明显不同的像素（按钮边框、色块边界），
/// 两侧留白基本对称时说明文字居中放在按钮/标签里，译文也居中
fn boxed_align(image: &RgbaImage, cover: &Rect, bg: Rgb, em: f32) -> &'static str {
    let (iw, ih) = image.dimensions();
    let max_scan = (em * 3.0).max(8.0) as i32;
    let rows = [0.3f32, 0.5, 0.7].map(|t| (cover.y + cover.h * t) as i32);
    // 返回该方向上到边界的距离；超出扫描范围或图像边缘视为没有边界
    let scan = |from: i32, dir: i32| -> Option<i32> {
        let mut found = Vec::new();
        for &y in &rows {
            if y < 0 || y >= ih as i32 {
                return None;
            }
            let hit = (1..=max_scan).find(|&d| {
                let x = from + dir * d;
                x >= 0 && x < iw as i32 && {
                    let p = image.get_pixel(x as u32, y as u32).0;
                    dist([p[0], p[1], p[2]], bg) > 60
                }
            })?;
            found.push(hit);
        }
        found.sort_unstable();
        Some(found[1])
    };
    let left = scan(cover.x as i32, -1);
    let right = scan((cover.x + cover.w) as i32 - 1, 1);
    match (left, right) {
        (Some(l), Some(r)) if (l - r).abs() as f32 <= (l.max(r) as f32 * 0.3).max(3.0) => "center",
        _ => "left",
    }
}

/// 估算笔画宽度（物理像素，可带小数）：每个像素按"离背景色多远"折算成覆盖度（抗锯齿边缘算半个像素），
/// 一行里连续的有墨像素构成一段，段内覆盖度之和即该处笔画宽度，取所有段的中位数
fn stroke_width(image: &RgbaImage, r: &Rect, bg: Rgb, fg: Rgb) -> f32 {
    let full = dist(bg, fg) as f32;
    if full < 60.0 {
        return 0.0;
    }
    let (iw, ih) = image.dimensions();
    let x0 = (r.x.max(0.0) as u32).min(iw);
    let y0 = (r.y.max(0.0) as u32).min(ih);
    let x1 = (r.right().ceil() as u32).min(iw);
    let y1 = (r.bottom().ceil() as u32).min(ih);
    let mut runs: Vec<f32> = Vec::new();
    for y in y0..y1 {
        let mut run = 0.0f32;
        let mut peak = 0.0f32;
        for x in x0..=x1 {
            let a = if x < x1 {
                let p = image.get_pixel(x, y).0;
                (dist([p[0], p[1], p[2]], bg) as f32 / full).min(1.0)
            } else {
                0.0
            };
            if a > 0.15 {
                run += a;
                peak = peak.max(a);
            } else if run > 0.0 {
                // 只统计真正压到文字色的段，过滤掉浅色噪点
                if peak > 0.6 {
                    runs.push(run);
                }
                run = 0.0;
                peak = 0.0;
            }
        }
    }
    if runs.len() < 8 {
        return 0.0;
    }
    runs.sort_by(f32::total_cmp);
    runs[runs.len() / 2]
}

type Rgb = [u8; 3];

/// 背景色 = 外框一圈像素的中位数；文字色 = 框内笔画核心（与背景差异接近最大）像素的平均值
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

    // 内部采样。细字的大部分"墨"像素是抗锯齿后的浅色边缘，平均进来会让文字色发灰，
    // 所以只取笔画核心：与背景差异达到"近似最大差异"85% 以上的像素
    let mut inner: Vec<(u32, Rgb)> = Vec::new();
    let sx = ((x1 - x0) / 200).max(1);
    let sy = ((y1 - y0) / 60).max(1);
    for y in (y0..y1).step_by(sy as usize) {
        for x in (x0..x1).step_by(sx as usize) {
            let c = px(x, y);
            inner.push((dist(c, bg), c));
        }
    }
    inner.sort_by(|a, b| b.0.cmp(&a.0));
    // 取第 1 百分位作为"最大差异"，避开个别噪点
    let peak = inner.get(inner.len() / 100).map_or(0, |p| p.0);
    let take = inner.iter().take_while(|p| p.0 * 100 >= peak * 85).count().max(1).min(inner.len());
    let fg = if take == 0 || peak < 40 {
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

    /// 构造单词：框高按字母组成换算（字号 18px），与真实 OCR 结果一致
    fn word(text: &str, x: f32, y: f32, w: f32) -> Word {
        let h = height_ratio(text, false).unwrap_or(0.93) * 18.0;
        Word {
            text: text.into(),
            x,
            y: y + 0.93 * 18.0 - h,
            w,
            h,
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
        let mut texts: Vec<_> = blocks.iter().map(|b| b.source.as_str()).collect();
        texts.sort();
        assert_eq!(texts, ["Hello world again.", "Next", "Right"]);
        let first = blocks.iter().find(|b| b.source.starts_with("Hello")).unwrap();
        assert_eq!(first.lines, 2);
        assert_eq!(first.bg, "#ffffff");
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

    /// 真机校准（后台进行）：抓取窗口 TF_HWND 的画面裁出 TF_RECT（窗口内坐标），或读取图片 TF_IMG
    /// → OCR → 打印每段估算的字号/行距/笔画比；TF_JSON 指定时把版面存成 JSON。
    /// TF_HWND=句柄 TF_RECT=x,y,w,h cargo test 字号校准 -- --ignored --nocapture
    #[test]
    #[ignore]
    fn 字号校准() {
        let v: Vec<i32> = std::env::var("TF_RECT")
            .unwrap_or_else(|_| "0,0,900,900".into())
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        // TF_IMG：直接读已保存的截图；否则抓取后台窗口 TF_HWND
        let image = if let Ok(p) = std::env::var("TF_IMG") {
            image::open(p).unwrap().to_rgba8()
        } else {
            let hwnd: isize = std::env::var("TF_HWND").unwrap().parse().unwrap();
            let full = crate::capture::capture_window(hwnd).unwrap();
            image::imageops::crop_imm(&full, v[0] as u32, v[1] as u32, v[2] as u32, v[3] as u32).to_image()
        };
        if let Ok(p) = std::env::var("TF_SHOT") {
            image.save(p).unwrap();
        }
        let out = crate::ocr::recognize(&image, "auto", "zh-CN").unwrap();
        let blocks = build_blocks(&out, &image, true);
        if let Ok(p) = std::env::var("TF_JSON") {
            std::fs::write(p, serde_json::to_string_pretty(&blocks).unwrap()).unwrap();
        }
        for b in blocks {
            let rect = Rect { x: b.x, y: b.y, w: b.w, h: b.h };
            let bg = parse_hex(&b.bg);
            let fg = parse_hex(&b.fg);
            let stroke = stroke_width(&image, &rect, bg, fg);
            println!(
                "字号 {:5.1} 行距 {:5.1} 行高 {:5.1} 笔画 {:4.2} 比 {:.3} 字重 {} {} 色 {} | {}",
                b.font, b.pitch, b.line_h, stroke, stroke / b.font, b.weight, b.align, b.fg,
                b.source.chars().take(40).collect::<String>()
            );
        }
    }

    fn parse_hex(s: &str) -> Rgb {
        let v = u32::from_str_radix(&s[1..], 16).unwrap();
        [(v >> 16) as u8, (v >> 8) as u8, v as u8]
    }

    #[test]
    fn 按钮内文字居中() {
        // 蓝色按钮（20..180）里一行文字（50..150），左右留白对称
        let mut img = RgbaImage::from_pixel(240, 60, Rgba([255, 255, 255, 255]));
        for y in 10..50 {
            for x in 20..180 {
                img.put_pixel(x, y, Rgba([47, 140, 255, 255]));
            }
        }
        let bg = [47, 140, 255];
        let cover = Rect { x: 50.0, y: 20.0, w: 100.0, h: 20.0 };
        assert_eq!(boxed_align(&img, &cover, bg, 14.0), "center");
        // 文字靠左放：左留白 10、右留白 70，不对称
        let cover = Rect { x: 30.0, y: 20.0, w: 80.0, h: 20.0 };
        assert_eq!(boxed_align(&img, &cover, bg, 14.0), "left");
        // 白底上的普通一行字：两侧找不到边界
        let cover = Rect { x: 190.0, y: 5.0, w: 40.0, h: 4.0 };
        assert_eq!(boxed_align(&img, &cover, [255, 255, 255], 14.0), "left");
    }
}
