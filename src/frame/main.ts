// 实时翻译框：按后端推送的版面，在原文位置盖上色块并显示译文（沉浸式效果）
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface Block {
  /** 物理像素，相对翻译框左上角 */
  x: number;
  y: number;
  w: number;
  h: number;
  lines: number;
  line_h: number;
  bg: string;
  fg: string;
  source: string;
  text: string;
}

interface Layout {
  blocks: Block[];
  pending: number;
  error: string | null;
  engine: string | null;
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

let paused = false;
let showOriginal = false;
/** 收到上次版面后累计的滚动位移（物理像素） */
let shift = { dx: 0, dy: 0 };

function px(v: number) {
  return `${v / window.devicePixelRatio}px`;
}

/** 在块内放下译文：从接近原文行高的字号开始逐步缩小，直到不溢出 */
function fitText(el: HTMLElement, b: Block) {
  let size = Math.max(MIN_FONT, (b.line_h / window.devicePixelRatio) * 0.82);
  el.style.fontSize = `${size}px`;
  while (size > MIN_FONT && (el.scrollHeight > el.clientHeight + 1 || el.scrollWidth > el.clientWidth + 1)) {
    size = Math.max(MIN_FONT, size * 0.9);
    el.style.fontSize = `${size}px`;
  }
}

function render(layout: Layout) {
  shift = { dx: 0, dy: 0 };
  blocksEl.style.transform = "";
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
listen<{ dx: number; dy: number }>("shift", (e) => {
  shift.dx += e.payload.dx;
  shift.dy += e.payload.dy;
  blocksEl.style.transform = `translate(${px(shift.dx)}, ${px(shift.dy)})`;
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
