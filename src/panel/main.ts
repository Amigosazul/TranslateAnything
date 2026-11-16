// 翻译浮窗：加载后向后端请求本窗口覆盖区域的译文并显示
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface TranslateResult {
  source: string;
  translated: string;
  engine: string;
}

const statusEl = document.getElementById("status")!;
const textEl = document.getElementById("text")!;
const win = getCurrentWindow();

function showError(msg: string) {
  textEl.hidden = true;
  statusEl.hidden = false;
  statusEl.className = "error";
  statusEl.textContent = msg;
}

async function translate() {
  textEl.hidden = true;
  statusEl.hidden = false;
  statusEl.className = "";
  statusEl.textContent = "识别翻译中…";
  try {
    const r = await invoke<TranslateResult>("translate_panel");
    statusEl.hidden = true;
    textEl.hidden = false;
    textEl.textContent = r.translated;
    textEl.title = `【${r.engine}】\n${r.source}`;
  } catch (e) {
    showError(String(e));
  }
}

document.getElementById("btn-close")!.addEventListener("click", () => win.close());
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") win.close();
});

translate();
