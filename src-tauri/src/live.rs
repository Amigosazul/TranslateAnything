//! 实时翻译框：透明、置顶、鼠标穿透；实时截取的是框下面的窗口画面（不含自身）。
//!
//! 每个翻译框有两个后台循环：
//! - 实时循环：画面变化时以截屏能达到的最快速度连续截取框内画面，估算滚动位移并立即推送，
//!   让译文跟手平移（不等 OCR）；同时在后台单独跑 OCR（同一时间只跑一个，总是识别最新画面）
//!   → 归并段落 → 命中缓存的直接出译文，新出现的文字批量请求翻译 → 推送版面给前端按原位置覆盖渲染。
//!   画面静止一段时间后放慢截屏频率以省电。
//! - 悬停循环：鼠标在边缘或工具条上时临时关闭穿透，以便拖动、缩放、点击按钮。
//!
//! 坐标约定：实时循环维护"累计滚动量" `total`（自上次几何变化起，内容总共平移了多少）。
//! 每次 OCR 记下开始时的 `total` 作为该版面的基准 `base`；前端把版面整体平移 `total - base`，
//! 这样 OCR 期间发生的滚动也不会让译文"弹回"。

use crate::capture;
use crate::config;
use crate::input;
use crate::layout::{self, Block};
use crate::motion::{self, Frame};
use crate::ocr;
use crate::config::PortableWebview;
use crate::translate;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// 画面在变化时的截屏间隔（实际受截屏耗时限制，一般 30~40ms 一帧）
const FAST_TICK: Duration = Duration::from_millis(12);
/// 画面静止时的截屏间隔
const IDLE_TICK: Duration = Duration::from_millis(100);
/// 画面静止多久后转入低频截屏
const IDLE_AFTER: Duration = Duration::from_millis(800);
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
    /// 检测到翻页键：立即唤醒跟踪循环
    wake: tokio::sync::Notify,
}

/// 推送给前端的版面
#[derive(Serialize, Clone)]
struct LayoutPayload {
    /// 几何变化计数：翻译框移动/缩放后加一，前端丢弃旧计数的消息
    epoch: u32,
    /// 版面坐标对应的累计滚动量（物理像素）
    base: [i32; 2],
    /// 当前累计滚动量
    total: [i32; 2],
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

/// 内容滚动：当前累计滚动量（发绝对值而非增量，丢一两条也不会累积误差）
#[derive(Serialize, Clone)]
struct ShiftPayload {
    epoch: u32,
    total: [i32; 2],
    /// 这帧画面截于多少毫秒前（前端据此还原真实时刻，学习界面的滑动时长）
    age: u32,
}

/// 翻译框内的竖直滚轮输入：前端据此预测平移（一格 = ±120，向上为正）
#[derive(Serialize, Clone)]
struct WheelPayload {
    epoch: u32,
    delta: i32,
}

/// 新建翻译框（物理像素，虚拟桌面坐标）
pub fn open_frame(app: &AppHandle, x: i32, y: i32, w: u32, h: u32) -> Result<(), String> {
    let state = app.state::<LiveState>();
    let label = format!("frame-{}", state.seq.fetch_add(1, Ordering::Relaxed) + 1);
    let win = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("frame.html".into()))
        .portable_data()
        .title(t!("app.name"))
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    crate::commands::disable_transitions(&win);
    win.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())?;
    win.set_size(PhysicalSize::new(w.max(MIN_W), h.max(MIN_H)))
        .map_err(|e| e.to_string())?;
    win.set_ignore_cursor_events(true).map_err(|e| e.to_string())?;
    win.show().map_err(|e| e.to_string())?;

    let ctl = Arc::new(FrameCtl::default());
    let ctl_for_hover = ctl.clone();
    state.frames.lock().unwrap().insert(label.clone(), ctl.clone());
    tauri::async_runtime::spawn(live_loop(app.clone(), label.clone(), ctl));
    tauri::async_runtime::spawn(hover_loop(app.clone(), label, ctl_for_hover));
    Ok(())
}

/// 当前打开的翻译框个数
pub fn frame_count(app: &AppHandle) -> usize {
    app.state::<LiveState>().frames.lock().unwrap().len()
}

/// 关闭所有翻译框；各自的刷新循环发现窗口没了会自行退出
pub fn close_all_frames(app: &AppHandle) -> usize {
    let labels: Vec<String> = app.state::<LiveState>().frames.lock().unwrap().keys().cloned().collect();
    for label in &labels {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.destroy();
        }
    }
    labels.len()
}

#[tauri::command]
pub fn frame_set_paused(window: WebviewWindow, state: tauri::State<'_, LiveState>, paused: bool) {
    if let Some(ctl) = state.frames.lock().unwrap().get(window.label()) {
        ctl.paused.store(paused, Ordering::Relaxed);
        ctl.dirty.store(true, Ordering::Relaxed);
    }
}

/// 后台任务（OCR、翻译）与实时循环之间的共享状态
#[derive(Default)]
struct Shared {
    cache: HashMap<String, String>,
    in_flight: bool,
    error: Option<String>,
    failed_at: Option<Instant>,
    engine: Option<String>,
    /// OCR 正在进行
    ocr_busy: bool,
    /// 最新一次 OCR 的结果，等实时循环取走
    ocr_done: Option<OcrDone>,
}

struct OcrDone {
    epoch: u32,
    base: [i32; 2],
    result: Result<Vec<Block>, String>,
}

async fn live_loop(app: AppHandle, label: String, ctl: Arc<FrameCtl>) {
    let shared = Arc::new(Mutex::new(Shared::default()));
    let mut prev: Option<Frame> = None;
    let mut last_rect: Option<(i32, i32, u32, u32)> = None;
    // 当前版面及其基准滚动量
    let mut blocks: Vec<Block> = Vec::new();
    let mut base = [0i32; 2];
    let mut ocr_error: Option<String> = None;
    // 累计滚动量与几何变化计数
    let mut total = [0i32; 2];
    let mut epoch = 0u32;
    // 画面自上次 OCR 开始后又变了，需要再识别一次
    let mut ocr_stale = true;
    let mut last_change = Instant::now();
    let mut wheel_rx = input::subscribe();

    loop {
        let tick = if last_change.elapsed() < IDLE_AFTER { FAST_TICK } else { IDLE_TICK };
        // 等下一帧；期间框内有滚轮或翻页键输入就立即醒来（空闲时最多可省 100ms 起步延迟）
        tokio::select! {
            _ = tokio::time::sleep(tick) => {}
            _ = ctl.wake.notified() => last_change = Instant::now(),
            w = wheel_rx.recv() => {
                if let Ok(w) = w {
                    let mut delta = 0;
                    let mut hit = false;
                    for w in std::iter::once(w).chain(std::iter::from_fn(|| wheel_rx.try_recv().ok())) {
                        let inside = last_rect.is_some_and(|(x, y, rw, rh)| {
                            w.x >= x && w.y >= y && w.x < x + rw as i32 && w.y < y + rh as i32
                        });
                        if inside {
                            hit = true;
                            if !w.horizontal {
                                delta += w.delta;
                            }
                        }
                    }
                    if hit {
                        last_change = Instant::now();
                        if delta != 0 && !ctl.paused.load(Ordering::Relaxed) {
                            let _ = app.emit_to(&label, "wheel", WheelPayload { epoch, delta });
                        }
                    } else {
                        // 不是本框的滚轮：照常等到下一帧
                        continue;
                    }
                }
            }
        }
        let Some(win) = app.get_webview_window(&label) else { break };
        if ctl.paused.load(Ordering::Relaxed) {
            continue;
        }
        let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) else { continue };
        let rect = (pos.x, pos.y, size.width, size.height);
        let rect_changed = last_rect != Some(rect);

        // 截屏 + 灰度化（阻塞操作放到线程池）
        let shot_at = Instant::now();
        let shot = tauri::async_runtime::spawn_blocking(move || {
            capture::capture_under(rect.0, rect.1, rect.2, rect.3).map(|(img, ox, oy)| {
                let frame = Frame::from_rgba(&img);
                (img, frame, ox, oy)
            })
        })
        .await;
        let (image, frame, ox, oy) = match shot {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                ocr_error = Some(e);
                continue;
            }
            Err(_) => continue,
        };

        let mut layout_changed = false;
        if rect_changed {
            // 移动/缩放：旧版面作废，坐标重新计起
            epoch += 1;
            total = [0, 0];
            base = [0, 0];
            blocks.clear();
            ocr_stale = true;
            last_change = Instant::now();
            prev = Some(frame);
            last_rect = Some(rect);
            layout_changed = true;
        } else if prev
            .as_ref()
            .is_none_or(|p| motion::changed_fraction(p, &frame) > CHANGE_THRESHOLD)
        {
            // 内容滚动：立即推送新的累计位移，前端马上平移现有译文
            if let Some((dx, dy)) = prev.as_ref().and_then(|p| motion::estimate_shift(p, &frame)) {
                total[0] += dx;
                total[1] += dy;
                let age = shot_at.elapsed().as_millis() as u32;
                let _ = app.emit_to(&label, "shift", ShiftPayload { epoch, total, age });
            }
            ocr_stale = true;
            last_change = Instant::now();
            prev = Some(frame);
        }

        let mut s = shared.lock().unwrap();

        // 取走已完成的 OCR 结果（几何变化前发起的作废）
        if let Some(done) = s.ocr_done.take() {
            if done.epoch == epoch {
                match done.result {
                    Ok(b) => {
                        blocks = b;
                        base = done.base;
                        ocr_error = None;
                    }
                    Err(e) => ocr_error = Some(e),
                }
                layout_changed = true;
            }
        }

        // 画面有变化且没有 OCR 在跑：识别最新这一帧（OCR 期间的滚动由 total - base 补偿）
        if ocr_stale && !s.ocr_busy {
            ocr_stale = false;
            s.ocr_busy = true;
            let (shared, ctl, ocr_base, ocr_epoch) = (shared.clone(), ctl.clone(), total, epoch);
            tauri::async_runtime::spawn_blocking(move || {
                let cfg = config::load().unwrap_or_default();
                let skip_cjk = cfg.target_lang.to_ascii_lowercase().starts_with("zh");
                let result = ocr::recognize(&image, &cfg.ocr_lang, &cfg.target_lang).map(|out| {
                    let mut blocks = layout::build_blocks(&out, &image, skip_cjk);
                    // 截图可能因超出屏幕被裁过，换算回相对翻译框的坐标
                    for b in &mut blocks {
                        b.x += ox as f32;
                        b.y += oy as f32;
                    }
                    blocks
                });
                let mut s = shared.lock().unwrap();
                s.ocr_busy = false;
                s.ocr_done = Some(OcrDone {
                    epoch: ocr_epoch,
                    base: ocr_base,
                    result,
                });
                ctl.dirty.store(true, Ordering::Relaxed);
            });
        }

        // 新出现的文字：批量请求翻译（同一时间只有一个请求在途）
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
                        s.error = Some(t!("err.engine.failed", e = e));
                        s.failed_at = Some(Instant::now());
                    }
                }
                ctl.dirty.store(true, Ordering::Relaxed);
            });
        }

        let dirty = ctl.dirty.swap(false, Ordering::Relaxed);
        if !layout_changed && !dirty {
            continue;
        }
        let payload = LayoutPayload {
            epoch,
            base,
            total,
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

    /// 真机计时（只读屏幕，不操作窗口）：连续"截屏 + 灰度化 + 变化检测 + 位移估计"的帧率。
    /// TF_RECT=x,y,w,h cargo test 跟踪帧率 -- --ignored --nocapture
    #[test]
    #[ignore]
    fn 跟踪帧率() {
        let v: Vec<i32> = std::env::var("TF_RECT")
            .unwrap_or_else(|_| "300,150,900,750".into())
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        let mut prev: Option<Frame> = None;
        let n = 40;
        let t = Instant::now();
        let mut cap = Duration::ZERO;
        for _ in 0..n {
            let t1 = Instant::now();
            let (img, _, _) = capture::capture_under(v[0], v[1], v[2] as u32, v[3] as u32).unwrap();
            cap += t1.elapsed();
            let f = Frame::from_rgba(&img);
            if let Some(p) = &prev {
                let _ = motion::changed_fraction(p, &f);
                let _ = motion::estimate_shift(p, &f);
            }
            prev = Some(f);
        }
        let el = t.elapsed();
        println!("每帧 {:?}（其中截屏 {:?}），约 {:.0} 帧/秒", el / n, cap / n, n as f64 / el.as_secs_f64());
    }

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

async fn hover_loop(app: AppHandle, label: String, ctl: Arc<FrameCtl>) {
    let mut zone = Zone::Outside;
    let mut interactive = false;
    let mut nav_down = false;
    loop {
        tokio::time::sleep(HOVER_TICK).await;
        let Some(win) = app.get_webview_window(&label) else { break };
        // 翻页键（只查这几个键是否按下，不监听其他按键）：按下瞬间唤醒跟踪循环
        let nav = capture::nav_key_down();
        if nav && !nav_down {
            ctl.wake.notify_one();
        }
        nav_down = nav;
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
