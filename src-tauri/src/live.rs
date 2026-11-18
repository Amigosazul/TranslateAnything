//! 实时翻译框：透明、置顶、鼠标穿透；实时截取的是框下面的窗口画面（不含自身）。
//!
//! 每个翻译框有两个后台循环：
//! - 实时循环：定时截取框内画面 → 有变化才 OCR → 归并段落 → 命中缓存的直接出译文，
//!   新出现的文字批量请求翻译 → 推送版面给前端按原位置覆盖渲染。
//!   滚动时先推送估算出的位移让译文即时跟随，再用 OCR 结果校正。
//! - 悬停循环：鼠标在边缘或工具条上时临时关闭穿透，以便拖动、缩放、点击按钮。

use crate::capture;
use crate::config;
use crate::layout::{self, Block};
use crate::motion::{self, Frame};
use crate::ocr;
use crate::translate;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// 截屏检测间隔
const TICK: Duration = Duration::from_millis(150);
/// 悬停检测间隔
const HOVER_TICK: Duration = Duration::from_millis(40);
/// 变化像素比例超过该值才重新识别
const CHANGE_THRESHOLD: f32 = 0.002;
/// 翻译失败后多久再重试
const RETRY_AFTER: Duration = Duration::from_secs(3);
/// 每个翻译框最多缓存的译文条数
const CACHE_LIMIT: usize = 3000;
/// 边缘可拖拽缩放的宽度、工具条尺寸（CSS 像素，与前端保持一致）
const EDGE_CSS: f64 = 6.0;
const TOOLBAR_W_CSS: f64 = 132.0;
const TOOLBAR_H_CSS: f64 = 30.0;
/// 翻译框最小尺寸（物理像素）
const MIN_W: u32 = 80;
const MIN_H: u32 = 40;

#[derive(Default)]
pub struct LiveState {
    frames: Mutex<HashMap<String, Arc<FrameCtl>>>,
    seq: AtomicU32,
}

#[derive(Default)]
struct FrameCtl {
    paused: AtomicBool,
    /// 有新译文到达，需要重新推送版面
    dirty: AtomicBool,
}

/// 推送给前端的版面
#[derive(Serialize, Clone)]
struct LayoutPayload {
    blocks: Vec<RenderBlock>,
    /// 等待翻译的块数
    pending: usize,
    /// 最近一次错误（OCR 或翻译）
    error: Option<String>,
    /// 最近一次成功使用的引擎名
    engine: Option<String>,
}

#[derive(Serialize, Clone)]
struct RenderBlock {
    #[serde(flatten)]
    block: Block,
    text: String,
}

#[derive(Serialize, Clone)]
struct ShiftPayload {
    dx: i32,
    dy: i32,
}

/// 新建翻译框（物理像素，虚拟桌面坐标）
pub fn open_frame(app: &AppHandle, x: i32, y: i32, w: u32, h: u32) -> Result<(), String> {
    let state = app.state::<LiveState>();
    let label = format!("frame-{}", state.seq.fetch_add(1, Ordering::Relaxed) + 1);
    let win = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("frame.html".into()))
        .title("翻译滤镜")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    win.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())?;
    win.set_size(PhysicalSize::new(w.max(MIN_W), h.max(MIN_H)))
        .map_err(|e| e.to_string())?;
    win.set_ignore_cursor_events(true).map_err(|e| e.to_string())?;
    win.show().map_err(|e| e.to_string())?;

    let ctl = Arc::new(FrameCtl::default());
    state.frames.lock().unwrap().insert(label.clone(), ctl.clone());
    tauri::async_runtime::spawn(live_loop(app.clone(), label.clone(), ctl));
    tauri::async_runtime::spawn(hover_loop(app.clone(), label));
    Ok(())
}

#[tauri::command]
pub fn frame_set_paused(window: WebviewWindow, state: tauri::State<'_, LiveState>, paused: bool) {
    if let Some(ctl) = state.frames.lock().unwrap().get(window.label()) {
        ctl.paused.store(paused, Ordering::Relaxed);
        ctl.dirty.store(true, Ordering::Relaxed);
    }
}

/// 翻译结果的共享状态（翻译任务在后台完成后写入）
#[derive(Default)]
struct Shared {
    cache: HashMap<String, String>,
    in_flight: bool,
    error: Option<String>,
    failed_at: Option<Instant>,
    engine: Option<String>,
}

async fn live_loop(app: AppHandle, label: String, ctl: Arc<FrameCtl>) {
    let shared = Arc::new(Mutex::new(Shared::default()));
    let mut prev: Option<Frame> = None;
    let mut last_rect: Option<(i32, i32, u32, u32)> = None;
    let mut blocks: Vec<Block> = Vec::new();
    let mut ocr_error: Option<String> = None;

    loop {
        tokio::time::sleep(TICK).await;
        let Some(win) = app.get_webview_window(&label) else { break };
        if ctl.paused.load(Ordering::Relaxed) {
            continue;
        }
        let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) else { continue };
        let rect = (pos.x, pos.y, size.width, size.height);
        let rect_changed = last_rect != Some(rect);

        // 截屏 + 变化检测
        let shot = tauri::async_runtime::spawn_blocking(move || capture::capture_under(rect.0, rect.1, rect.2, rect.3)).await;
        let (image, ox, oy) = match shot {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                ocr_error = Some(e);
                continue;
            }
            Err(_) => continue,
        };
        let frame = Frame::from_rgba(&image);
        let changed = rect_changed
            || prev
                .as_ref()
                .is_none_or(|p| motion::changed_fraction(p, &frame) > CHANGE_THRESHOLD);

        if changed {
            // 内容滚动：先让前端把现有译文整体平移
            if !rect_changed {
                if let Some((dx, dy)) = prev.as_ref().and_then(|p| motion::estimate_shift(p, &frame)) {
                    let _ = app.emit_to(&label, "shift", ShiftPayload { dx, dy });
                }
            }
            let cfg = config::load().unwrap_or_default();
            let skip_cjk = cfg.target_lang.to_ascii_lowercase().starts_with("zh");
            let (ocr_lang, target) = (cfg.ocr_lang.clone(), cfg.target_lang.clone());
            let result = tauri::async_runtime::spawn_blocking(move || {
                let out = ocr::recognize(&image, &ocr_lang, &target)?;
                let mut blocks = layout::build_blocks(&out, &image, skip_cjk);
                // 截图可能因超出屏幕被裁过，换算回相对翻译框的坐标
                for b in &mut blocks {
                    b.x += ox as f32;
                    b.y += oy as f32;
                }
                Ok::<_, String>(blocks)
            })
            .await;
            match result {
                Ok(Ok(b)) => {
                    blocks = b;
                    ocr_error = None;
                }
                Ok(Err(e)) => ocr_error = Some(e),
                Err(e) => ocr_error = Some(e.to_string()),
            }
            prev = Some(frame);
            last_rect = Some(rect);
        }

        // 新出现的文字：批量请求翻译（同一时间只有一个请求在途）
        let mut s = shared.lock().unwrap();
        let mut seen = HashSet::new();
        let missing: Vec<String> = blocks
            .iter()
            .map(|b| b.source.clone())
            .filter(|t| !s.cache.contains_key(t) && seen.insert(t.clone()))
            .collect();
        let can_retry = s.failed_at.is_none_or(|t| t.elapsed() > RETRY_AFTER);
        if !missing.is_empty() && !s.in_flight && can_retry {
            s.in_flight = true;
            let (shared, ctl) = (shared.clone(), ctl.clone());
            tauri::async_runtime::spawn(async move {
                let cfg = config::load().unwrap_or_default();
                let result = translate::translate_batch_with_fallback(&cfg, &missing).await;
                let mut s = shared.lock().unwrap();
                s.in_flight = false;
                match result {
                    Ok((texts, engine)) => {
                        if s.cache.len() > CACHE_LIMIT {
                            s.cache.clear();
                        }
                        s.cache.extend(missing.into_iter().zip(texts));
                        s.error = None;
                        s.failed_at = None;
                        s.engine = Some(engine);
                    }
                    Err(e) => {
                        s.error = Some(format!("翻译失败：\n{e}"));
                        s.failed_at = Some(Instant::now());
                    }
                }
                ctl.dirty.store(true, Ordering::Relaxed);
            });
        }

        let dirty = ctl.dirty.swap(false, Ordering::Relaxed);
        if !changed && !dirty {
            continue;
        }
        let payload = LayoutPayload {
            blocks: blocks
                .iter()
                .filter_map(|b| {
                    s.cache.get(&b.source).map(|t| RenderBlock {
                        block: b.clone(),
                        text: t.clone(),
                    })
                })
                .collect(),
            pending: blocks.iter().filter(|b| !s.cache.contains_key(&b.source)).count(),
            error: ocr_error.clone().or_else(|| s.error.clone()),
            engine: s.engine.clone(),
        };
        drop(s);
        let _ = app.emit_to(&label, "layout", payload);
    }

    app.state::<LiveState>().frames.lock().unwrap().remove(&label);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真机端到端：截取主屏左上角 → OCR → 分段 → 批量翻译。cargo test 端到端 -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn 端到端实测屏幕() {
        let (image, _, _) = capture::capture_under(0, 0, 1000, 700).unwrap();
        let t = Instant::now();
        let out = ocr::recognize(&image, "auto", "zh-CN").unwrap();
        let blocks = layout::build_blocks(&out, &image, true);
        println!("OCR + 分段耗时 {:?}，{} 行 → {} 段", t.elapsed(), out.lines.len(), blocks.len());
        let texts: Vec<String> = blocks.iter().map(|b| b.source.clone()).collect();
        let t = Instant::now();
        let (tr, engine) = translate::translate_batch_with_fallback(&config::Config::default(), &texts)
            .await
            .unwrap();
        println!("批量翻译耗时 {:?}（{engine}）", t.elapsed());
        for (b, t) in blocks.iter().zip(&tr).take(12) {
            println!("[{:.0},{:.0} {:.0}x{:.0} {} {}] {} → {}", b.x, b.y, b.w, b.h, b.bg, b.fg, b.source, t);
        }
    }
}

/// 鼠标悬停区域
#[derive(Clone, Copy, PartialEq)]
enum Zone {
    Outside,
    Inside,
    Edge,
    Toolbar,
}

impl Zone {
    fn name(self) -> &'static str {
        match self {
            Zone::Outside => "outside",
            Zone::Inside => "inside",
            Zone::Edge => "edge",
            Zone::Toolbar => "toolbar",
        }
    }
}

async fn hover_loop(app: AppHandle, label: String) {
    let mut zone = Zone::Outside;
    let mut interactive = false;
    loop {
        tokio::time::sleep(HOVER_TICK).await;
        let Some(win) = app.get_webview_window(&label) else { break };
        let (Ok(pos), Ok(size), Ok(scale)) = (win.outer_position(), win.outer_size(), win.scale_factor()) else {
            continue;
        };
        let Some((cx, cy, button_down)) = capture::cursor_state() else { continue };
        // 拖动/缩放过程中保持当前状态
        if button_down && interactive {
            continue;
        }
        let (l, t) = (pos.x as f64, pos.y as f64);
        let (r, b) = (l + size.width as f64, t + size.height as f64);
        let (cx, cy) = (cx as f64, cy as f64);
        let edge = EDGE_CSS * scale;
        let new_zone = if cx < l || cx >= r || cy < t || cy >= b {
            Zone::Outside
        } else if cx >= r - TOOLBAR_W_CSS * scale - edge && cy < t + TOOLBAR_H_CSS * scale + edge && cx < r - edge && cy > t + edge {
            Zone::Toolbar
        } else if cx < l + edge || cx >= r - edge || cy < t + edge || cy >= b - edge {
            Zone::Edge
        } else {
            Zone::Inside
        };
        let want = matches!(new_zone, Zone::Edge | Zone::Toolbar);
        if want != interactive && win.set_ignore_cursor_events(!want).is_ok() {
            interactive = want;
        }
        if new_zone != zone {
            zone = new_zone;
            let _ = app.emit_to(&label, "hover", zone.name());
        }
    }
}
