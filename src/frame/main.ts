// 实时翻译框：按后端推送的版面，在原文位置盖上色块并显示译文（沉浸式效果）
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ScrollFollower } from "./scroll";

interface Block {
  /** 物理像素，相对翻译框左上角 */
  x: number;
  y: number;
  w: number;
  h: number;
  lines: number;
  line_h: number;
  /** 原文字号（em，物理像素） */
  font: number;
  /** 原文行距（物理像素），单行块为 0 */
  pitch: number;
  /** 多行块首行上留白（物理像素） */
  pad_top: number;
  weight: number;
  align: "left" | "center";
  bg: string;
  fg: string;
  source: string;
  text: string;
}

interface Layout {
  /** 几何变化计数 */
  epoch: number;
  /** 版面坐标对应的累计滚动量、当前累计滚动量（物理像素） */
  base: [number, number];
  total: [number, number];
  blocks: Block[];
  pending: number;
  error: string | null;
  engine: string | null;
}

interface Shift {
  epoch: number;
  total: [number, number];
  /** 这帧画面截于多少毫秒前 */
  age: number;
}

/** 一个块最终采用的排版参数（CSS 像素） */
interface Fit {
  size: number;
  lineHeight: number;
  padTop: number;
}

type ResizeDir =
  | "North" | "South" | "West" | "East"
  | "NorthWest" | "NorthEast" | "SouthWest" | "SouthEast";

const win = getCurrentWindow();
const blocksEl = document.getElementById("blocks")!;
const statusEl = document.getElementById("status")!;
const errorEl = document.getElementById("error")!;
const btnPause = document.getElementById("btn-pause")!;
const btnOriginal = document.getElementById("btn-original")!;

/** 最小字号（CSS 像素） */
const MIN_FONT = 9;
/** 放不下时收紧到的行距倍数 */
const TIGHT_LINE = 1.2;

let paused = false;
let showOriginal = false;
/** 当前版面所属的几何变化计数、基准滚动量、最新累计滚动量（物理像素） */
let epoch = -1;
/** 翻译框移动/缩放后，丢弃计数小于该值的消息 */
let minEpoch = 0;
let base: [number, number] = [0, 0];
/** 滚动跟随：实测位移 + 滚轮预测，逐帧给出译文层该平移到哪 */
const follower = new ScrollFollower();
let rafId = 0;
/** 排版结果缓存：滚动时同一段反复重绘，不必每次都量一遍 */
const fitCache = new Map<string, Fit>();

function css(v: number) {
  return v / window.devicePixelRatio;
}

function px(v: number) {
  return `${css(v)}px`;
}

function overflows(el: HTMLElement) {
  return el.scrollHeight > el.clientHeight + 1 || el.scrollWidth > el.clientWidth + 1;
}

function applyFit(el: HTMLElement, f: Fit) {
  el.style.fontSize = `${f.size}px`;
  el.style.lineHeight = `${f.lineHeight}px`;
  el.style.paddingTop = `${f.padTop}px`;
}

/**
 * 译文与原文同字号、同行距：单行块让文字在块内垂直居中，多行块首行对齐原文首行。
 * 放不下时先把行距收紧，再逐步缩小字号。
 */
function fitText(el: HTMLElement, b: Block) {
  const key = `${b.text}|${b.w}|${b.h}|${b.font}|${b.pitch}|${b.weight}`;
  const cached = fitCache.get(key);
  if (cached) {
    applyFit(el, cached);
    return;
  }
  let size = Math.max(MIN_FONT, css(b.font));
  const f: Fit = {
    size,
    lineHeight: b.pitch > 0 ? css(b.pitch) : css(b.h),
    padTop: b.pitch > 0 ? css(b.pad_top) : 0,
  };
  applyFit(el, f);
  if (overflows(el)) {
    f.padTop = 0;
    f.lineHeight = Math.min(f.lineHeight, size * TIGHT_LINE);
    applyFit(el, f);
    while (size > MIN_FONT && overflows(el)) {
      size = Math.max(MIN_FONT, size * 0.94);
      f.size = size;
      f.lineHeight = size * TIGHT_LINE;
      applyFit(el, f);
    }
  }
  if (fitCache.size > 2000) fitCache.clear();
  fitCache.set(key, { ...f });
}

/** 按显示用的滚动量平移整个译文层 */
function applyOffset() {
  const [x, y] = follower.display;
  blocksEl.style.transform = `translate3d(${px(x - base[0])}, ${px(y - base[1])}, 0)`;
}

function frameTick(now: number) {
  follower.tick(now);
  applyOffset();
  rafId = follower.active ? requestAnimationFrame(frameTick) : 0;
}

/** 有新的位移或滚轮输入时启动逐帧刷新（已在跑就不重复启动） */
function kick() {
  if (!rafId) rafId = requestAnimationFrame(frameTick);
}

function render(layout: Layout) {
  if (layout.epoch < minEpoch) return;
  if (layout.epoch !== epoch) follower.reset(layout.total);
  else follower.onMeasured(layout.total, performance.now());
  epoch = layout.epoch;
  base = layout.base;
  applyOffset();
  kick();
  const frag = document.createDocumentFragment();
  const toFit: [HTMLElement, Block][] = [];
  for (const b of layout.blocks) {
    const el = document.createElement("div");
    el.className = "block";
    Object.assign(el.style, {
      left: px(b.x),
      top: px(b.y),
      width: px(b.w),
      height: px(b.h),
      background: b.bg,
      color: b.fg,
      fontWeight: String(b.weight),
      textAlign: b.align,
    });
    el.textContent = b.text;
    frag.appendChild(el);
    toFit.push([el, b]);
  }
  blocksEl.replaceChildren(frag);
  for (const [el, b] of toFit) fitText(el, b);

  statusEl.className = layout.error ? "err" : layout.pending > 0 ? "busy" : "";
  statusEl.title = [
    layout.engine ? `引擎：${layout.engine}` : "",
    layout.pending > 0 ? `待翻译 ${layout.pending} 段` : "",
  ]
    .filter(Boolean)
    .join("\n");
  errorEl.textContent = layout.error ?? "";
  errorEl.hidden = !layout.error || !document.body.classList.contains("hover");
}

listen<Layout>("layout", (e) => render(e.payload));

// 内容滚动：OCR 结果出来之前先整体平移现有译文
listen<Shift>("shift", (e) => {
  if (e.payload.epoch !== epoch) return;
  follower.onMeasured(e.payload.total, performance.now(), e.payload.age);
  kick();
});

// 框内滚轮：先按学到的滚动量预测平移，不等截屏比对
listen<{ epoch: number; delta: number }>("wheel", (e) => {
  if (e.payload.epoch !== epoch) return;
  follower.onWheel(e.payload.delta, performance.now());
  kick();
});

// 悬停区域由后端轮询鼠标得出（窗口平时是鼠标穿透的，自己收不到事件）
listen<string>("hover", (e) => {
  const inside = e.payload !== "outside";
  document.body.classList.toggle("hover", inside);
  errorEl.hidden = !inside || !errorEl.textContent;
});

// 移动或缩放时旧译文位置已失效，先清掉，等新版面
let clearTimer = 0;
function onGeometryChange() {
  // 后端检测到几何变化后会递增计数，在此之前发出的旧版面一律丢弃
  minEpoch = epoch + 1;
  blocksEl.replaceChildren();
  window.clearTimeout(clearTimer);
}
win.onMoved(onGeometryChange);
win.onResized(onGeometryChange);

document.getElementById("btn-move")!.addEventListener("mousedown", (e) => {
  if (e.button === 0) win.startDragging();
});

document.querySelectorAll<HTMLElement>(".edge").forEach((el) => {
  el.addEventListener("mousedown", (e) => {
    if (e.button === 0) win.startResizeDragging(el.dataset.dir as ResizeDir);
  });
});

btnPause.addEventListener("click", () => {
  paused = !paused;
  document.body.classList.toggle("paused", paused);
  btnPause.textContent = paused ? "▶" : "❚❚";
  btnPause.title = paused ? "继续实时翻译" : "暂停实时翻译";
  invoke("frame_set_paused", { paused });
});

btnOriginal.addEventListener("click", () => {
  showOriginal = !showOriginal;
  blocksEl.classList.toggle("hidden", showOriginal);
  btnOriginal.classList.toggle("on", showOriginal);
  btnOriginal.title = showOriginal ? "显示译文" : "显示原文";
});

document.getElementById("btn-close")!.addEventListener("click", () => win.close());
