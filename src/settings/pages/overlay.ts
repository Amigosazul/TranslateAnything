// 悬浮窗模式：框选翻译的入口、快捷键、识别方式与识别语言
import { invoke } from "@tauri-apps/api/core";
import { S, cfg, refresh, saveConfig } from "../state";
import { dropdown, errText, h, icon, keys, pageHead, segmented, toast } from "../ui";

export async function loadFrames() {
  try {
    S.frames = await invoke<number>("frames_count");
  } catch {
    S.frames = 0;
  }
}

export async function loadOcrLangs() {
  if (S.ocrLangs) return;
  try {
    S.ocrLangs = await invoke<{ tag: string; name: string }[]>("ocr_languages");
  } catch {
    S.ocrLangs = [];
  }
}

export async function startSelect() {
  try {
    await invoke("overlay_start");
  } catch (e) {
    toast(errText(e), true);
  }
}

export async function closeAllFrames() {
  try {
    const n = await invoke<number>("frames_close_all");
    toast(n ? `已关闭 ${n} 个翻译框` : "没有打开的翻译框");
  } catch (e) {
    toast(errText(e), true);
  }
  setTimeout(async () => {
    await loadFrames();
    refresh();
  }, 300);
}

export function renderOverlay(root: HTMLElement) {
  const c = cfg();
  const frames = S.frames;

  const hero = h(
    "div",
    { class: "card hero" },
    h(
      "div",
      { class: "row", style: "gap:16px" },
      h("div", { class: "hero-icon" }, icon("frame")),
      h(
        "div",
        { class: "grow" },
        h("h3", {}, "框选翻译"),
        h("div", { class: "desc" }, "随时按 ", keys(c.hotkey), " 框选屏幕上的区域，译文会盖在原处。"),
      ),
      h("button", { class: "btn primary", onclick: startSelect }, icon("frame"), "开始框选"),
    ),
    h(
      "div",
      { class: "metrics" },
      h("div", { class: "metric" }, h("b", {}, String(frames)), h("span", {}, "打开中的翻译框")),
      h("span", { class: "spacer" }),
      h("button", { class: "btn ghost sm", disabled: frames === 0, onclick: closeAllFrames }, icon("x"), "全部关闭"),
    ),
  );

  const hotkey = h("input", { type: "text", value: c.hotkey, "data-k": "overlay-hotkey" });
  hotkey.addEventListener("change", () => {
    const v = hotkey.value.trim();
    if (v && v !== c.hotkey) saveConfig((x) => (x.hotkey = v), "已保存，重启随译后生效");
  });

  const langs = S.ocrLangs;
  const langPick = dropdown(
    [
      { value: "auto", label: "自动", hint: "优先选一个与目标语言不同的已安装语言" },
      ...(langs ?? []).map((l) => ({ value: l.tag, label: l.name, hint: l.tag })),
    ],
    c.ocr_lang || "auto",
    (v) => saveConfig((x) => (x.ocr_lang = v), "识别语言已更改"),
    { width: "100%" },
  );

  const row = (label: string, desc: string, control: HTMLElement, auto = false) =>
    h(
      "div",
      { class: "setting-row" },
      h("div", { class: "label" }, h("div", {}, label), h("div", { class: "desc" }, desc)),
      h("div", { class: `control${auto ? " auto" : ""}` }, control),
    );

  root.replaceChildren(
    pageHead("悬浮窗模式", "适用于图片、视频和任何接管不了的软件。翻译框可以拖动、缩放，位置或画面变化后自动重新识别翻译。"),
    hero,
    h(
      "div",
      { class: "card flush" },
      row("框选翻译快捷键", "重启随译后生效。", hotkey),
      row(
        "识别方式",
        c.mode === "vision" ? "大模型直接看图翻译。" : "先用系统自带的文字识别（离线）读出文字，再交给翻译引擎。视觉直译即将支持。",
        segmented(
          [
            ["ocr", "文字识别 + 翻译"],
            ["vision", "视觉直译"],
          ],
          c.mode === "vision" ? "vision" : "ocr",
          (m) => saveConfig((x) => (x.mode = m)),
          ["vision"],
        ),
        true,
      ),
      row(
        "识别语言",
        langs && langs.length === 0 ? "没有读到已安装的文字识别语言，请在系统设置的“语言”里添加语言包。" : "画面里文字的语言。需要在系统设置里安装对应语言包。",
        langPick,
      ),
    ),
  );
}
