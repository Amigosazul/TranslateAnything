//! 画面变化检测与滚动位移估计。
//!
//! 滚动时 OCR 需要一两百毫秒，先用行/列"签名"快速估出内容整体平移了多少，
//! 让译文立即跟着平移，等 OCR 结果出来再精确校正。

use image::RgbaImage;

/// 每行（列）签名分成多少段
const BUCKETS: usize = 32;
/// 视为"有内容"的行签名最小起伏（灰度）
const MIN_ROW_VAR: f32 = 6.0;

/// 灰度帧
pub struct Frame {
    pub w: usize,
    pub h: usize,
    gray: Vec<u8>,
}

impl Frame {
    pub fn from_rgba(img: &RgbaImage) -> Self {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let gray = img
            .as_raw()
            .chunks_exact(4)
            .map(|p| ((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as u8)
            .collect();
        Self { w, h, gray }
    }

    fn at(&self, x: usize, y: usize) -> u8 {
        self.gray[y * self.w + x]
    }
}

/// 两帧之间发生明显变化的像素比例（稀疏采样）；尺寸不同视为完全变化
pub fn changed_fraction(a: &Frame, b: &Frame) -> f32 {
    if a.w != b.w || a.h != b.h || a.w == 0 || a.h == 0 {
        return 1.0;
    }
    let (mut total, mut changed) = (0u32, 0u32);
    for y in (0..a.h).step_by(3) {
        for x in (0..a.w).step_by(3) {
            total += 1;
            if a.at(x, y).abs_diff(b.at(x, y)) > 24 {
                changed += 1;
            }
        }
    }
    changed as f32 / total.max(1) as f32
}

/// 估计内容从 `prev` 到 `cur` 的整体平移（屏幕像素，向下/向右为正）；
/// 只检测单方向滚动，不是平移或不可靠时返回 None
pub fn estimate_shift(prev: &Frame, cur: &Frame) -> Option<(i32, i32)> {
    if prev.w != cur.w || prev.h != cur.h || cur.w < 8 || cur.h < 8 {
        return None;
    }
    let rows_prev = row_signatures(prev);
    let rows_cur = row_signatures(cur);
    if let Some(d) = best_offset(&rows_prev, &rows_cur) {
        return Some((0, d));
    }
    let cols_prev = col_signatures(prev);
    let cols_cur = col_signatures(cur);
    best_offset(&cols_prev, &cols_cur).map(|d| (d, 0))
}

type Sig = [f32; BUCKETS];

fn row_signatures(f: &Frame) -> Vec<Sig> {
    (0..f.h)
        .map(|y| {
            let mut s = [0.0; BUCKETS];
            for (i, v) in s.iter_mut().enumerate() {
                let (x0, x1) = (i * f.w / BUCKETS, ((i + 1) * f.w / BUCKETS).max(i * f.w / BUCKETS + 1));
                let sum: u32 = (x0..x1.min(f.w)).map(|x| f.at(x, y) as u32).sum();
                *v = sum as f32 / (x1 - x0) as f32;
            }
            s
        })
        .collect()
}

fn col_signatures(f: &Frame) -> Vec<Sig> {
    (0..f.w)
        .map(|x| {
            let mut s = [0.0; BUCKETS];
            for (i, v) in s.iter_mut().enumerate() {
                let (y0, y1) = (i * f.h / BUCKETS, ((i + 1) * f.h / BUCKETS).max(i * f.h / BUCKETS + 1));
                let sum: u32 = (y0..y1.min(f.h)).map(|y| f.at(x, y) as u32).sum();
                *v = sum as f32 / (y1 - y0) as f32;
            }
            s
        })
        .collect()
}

fn variance(s: &Sig) -> f32 {
    let mean = s.iter().sum::<f32>() / BUCKETS as f32;
    s.iter().map(|v| (v - mean).abs()).sum::<f32>() / BUCKETS as f32
}

fn sig_diff(a: &Sig, b: &Sig) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / BUCKETS as f32
}

/// 粗搜时最多使用的有效行数（均匀抽样），精搜时再用全部有效行核对
const COARSE_ROWS: usize = 96;
/// 粗搜保留的候选偏移个数
const COARSE_CANDIDATES: usize = 4;

/// 在 [-n/2, n/2] 内找使 cur[i] ≈ prev[i + s] 的 s，返回内容位移 -s。
/// 先用抽样行粗搜所有偏移，再在最优偏移附近用全部有效行精确核对，耗时与画面高度近似线性。
fn best_offset(prev: &[Sig], cur: &[Sig]) -> Option<i32> {
    let n = cur.len() as i32;
    let max = n / 2;
    // 只用"有内容"的行比较，空白行和什么都能匹配
    let informative: Vec<usize> = (0..cur.len()).filter(|&i| variance(&cur[i]) > MIN_ROW_VAR).collect();
    if informative.len() < 4 {
        return None;
    }
    let step = informative.len().div_ceil(COARSE_ROWS);
    let sampled: Vec<usize> = informative.iter().step_by(step).copied().collect();
    let err_at = |rows: &[usize], s: i32| -> Option<f32> {
        let mut sum = 0.0;
        let mut cnt = 0;
        for &i in rows {
            let j = i as i32 + s;
            if j >= 0 && j < n {
                sum += sig_diff(&cur[i], &prev[j as usize]);
                cnt += 1;
            }
        }
        // 重叠的有效行太少不可信
        (cnt * 2 >= rows.len()).then(|| sum / cnt as f32)
    };

    // 粗搜：保留误差最小的几个候选（抽样可能被重复图案骗到，不能只信第一名）
    let mut candidates: Vec<(f32, i32)> = (-max..=max)
        .filter(|&s| s != 0)
        .filter_map(|s| err_at(&sampled, s).map(|e| (e, s)))
        .collect();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    candidates.truncate(COARSE_CANDIDATES);

    // 精搜：在每个候选 ±2 内用全部有效行核对
    let err0 = err_at(&informative, 0)?;
    let (mut best_s, mut best_err) = (0, err0);
    for &(_, c) in &candidates {
        for s in (c - 2)..=(c + 2) {
            if s == 0 || s.abs() > max {
                continue;
            }
            if let Some(e) = err_at(&informative, s) {
                if e < best_err {
                    best_err = e;
                    best_s = s;
                }
            }
        }
    }
    // 必须明显优于"没动"，且本身足够吻合
    if best_s != 0 && best_err < 3.0 && best_err < err0 * 0.4 {
        Some(-best_s)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 生成一张有若干"文字行"的图，`offset` 为内容向下的位移
    fn page(offset: i32) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(200, 300, Rgba([255, 255, 255, 255]));
        for line in 0..20 {
            let top = line * 24 + offset;
            for y in top..top + 10 {
                if !(0..300).contains(&y) {
                    continue;
                }
                for x in 0..200 {
                    // 每行不同的"字形"图案
                    if (x / (3 + line as u32 % 5) + line as u32) % 3 == 0 {
                        img.put_pixel(x, y as u32, Rgba([0, 0, 0, 255]));
                    }
                }
            }
        }
        img
    }

    #[test]
    fn 检测垂直滚动() {
        let a = Frame::from_rgba(&page(0));
        let b = Frame::from_rgba(&page(-37));
        assert_eq!(estimate_shift(&a, &b), Some((0, -37)));
        let c = Frame::from_rgba(&page(13));
        assert_eq!(estimate_shift(&a, &c), Some((0, 13)));
    }

    #[test]
    fn 未变化() {
        let a = Frame::from_rgba(&page(0));
        let b = Frame::from_rgba(&page(0));
        assert_eq!(changed_fraction(&a, &b), 0.0);
        assert_eq!(estimate_shift(&a, &b), None);
    }

    #[test]
    fn 位移估计耗时() {
        // 模拟 900x1400 的大框
        let mk = |offset: i32| {
            let mut img = RgbaImage::from_pixel(900, 1400, Rgba([255, 255, 255, 255]));
            for line in 0..90 {
                let top = line * 16 + offset;
                for y in top..top + 9 {
                    if !(0..1400).contains(&y) {
                        continue;
                    }
                    for x in 0..900 {
                        // 每行不同的伪随机"字形"
                        let h = (x / 3u32).wrapping_mul(2654435761u32).wrapping_add((y - top) as u32 * 7919 + line as u32 * 40503) >> 13;
                        if h % 3 == 0 {
                            img.put_pixel(x, y as u32, Rgba([0, 0, 0, 255]));
                        }
                    }
                }
            }
            img
        };
        let (a, b) = (mk(0), mk(-23));
        let t = std::time::Instant::now();
        let (fa, fb) = (Frame::from_rgba(&a), Frame::from_rgba(&b));
        println!("灰度化 {:?}", t.elapsed());
        let t2 = std::time::Instant::now();
        let _ = row_signatures(&fa);
        println!("行签名 {:?}", t2.elapsed());
        let t2 = std::time::Instant::now();
        let _ = changed_fraction(&fa, &fb);
        println!("变化比例 {:?}", t2.elapsed());
        let d = estimate_shift(&fa, &fb);
        let el = t.elapsed();
        println!("灰度化 + 位移估计耗时 {el:?}");
        assert_eq!(d, Some((0, -23)));
    }
}
