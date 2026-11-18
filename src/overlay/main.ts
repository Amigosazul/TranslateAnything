// 框选遮罩：画选区矩形，松开鼠标后把选区（物理像素、相对本显示器左上角）回传给后端
import { invoke } from "@tauri-apps/api/core";
import { initI18n } from "../i18n";

// 提示文字在 HTML 里用 data-i18n 标注，载入文案后自动填上
void initI18n();

const selection = document.getElementById("selection")!;
const sizeTip = document.getElementById("size-tip")!;
const hint = document.getElementById("hint")!;

/** 选区小于该尺寸（CSS 像素）视为误点，不触发翻译 */
const MIN_SIZE = 8;

let start: { x: number; y: number } | null = null;
let finished = false;

function rectFrom(x: number, y: number) {
  const s = start!;
  return {
    left: Math.min(s.x, x),
    top: Math.min(s.y, y),
    width: Math.abs(x - s.x),
    height: Math.abs(y - s.y),
  };
}

function cancel() {
  if (finished) return;
  finished = true;
  invoke("select_cancel");
}

window.addEventListener("mousedown", (e) => {
  if (e.button === 2) {
    cancel();
    return;
  }
  if (e.button !== 0 || finished) return;
  start = { x: e.clientX, y: e.clientY };
  hint.hidden = true;
  selection.hidden = false;
  sizeTip.hidden = false;
});

window.addEventListener("mousemove", (e) => {
  if (!start) return;
  const r = rectFrom(e.clientX, e.clientY);
  Object.assign(selection.style, {
    left: `${r.left}px`,
    top: `${r.top}px`,
    width: `${r.width}px`,
    height: `${r.height}px`,
  });
  const dpr = window.devicePixelRatio;
  sizeTip.textContent = `${Math.round(r.width * dpr)} × ${Math.round(r.height * dpr)}`;
  Object.assign(sizeTip.style, {
    left: `${r.left}px`,
    top: `${Math.max(0, r.top - 22)}px`,
  });
});

window.addEventListener("mouseup", (e) => {
  if (!start || e.button !== 0 || finished) return;
  const r = rectFrom(e.clientX, e.clientY);
  start = null;
  if (r.width < MIN_SIZE || r.height < MIN_SIZE) {
    // 误点：恢复初始状态，允许重新框选
    selection.hidden = true;
    sizeTip.hidden = true;
    hint.hidden = false;
    return;
  }
  finished = true;
  const dpr = window.devicePixelRatio;
  invoke("select_done", {
    rect: {
      x: Math.round(r.left * dpr),
      y: Math.round(r.top * dpr),
      w: Math.round(r.width * dpr),
      h: Math.round(r.height * dpr),
    },
  });
});

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") cancel();
});

window.addEventListener("contextmenu", (e) => e.preventDefault());
