// 悬浮窗模式：框选入口、快捷键、框内鼠标操作与识别设置
import { invoke } from "@tauri-apps/api/core";
import { S, cfg, refresh, saveConfig } from "../state";
import { dropdown, errText, h, hotkeyInput, icon, keys, pageHead, segmented, toast } from "../ui";
import { t } from "../../i18n";

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
    toast(n ? t("overlayPage.closed", { n }) : t("overlayPage.noneOpen"));
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
        h("h3", {}, t("overlayPage.heroTitle")),
        h("div", { class: "desc" }, t("overlayPage.heroDescBefore"), keys(c.hotkey), t("overlayPage.heroDescAfter")),
      ),
      h("button", { class: "btn primary", onclick: startSelect }, icon("frame"), t("overlayPage.start")),
    ),
    h(
      "div",
      { class: "metrics" },
      h("div", { class: "metric" }, h("b", {}, String(frames)), h("span", {}, t("overlayPage.openFrames"))),
      h("span", { class: "spacer" }),
      h("button", { class: "btn ghost sm", disabled: frames === 0, onclick: closeAllFrames }, icon("x"), t("overlayPage.closeAll")),
    ),
  );

  const hotkey = hotkeyInput(c.hotkey, (v) => saveConfig((x) => (x.hotkey = v), t("overlayPage.hotkeySaved", { app: t("app.name") })), "overlay-hotkey");

  const mouseModeLabel = t("overlayPage.mouseMode");
  const mouseModeDesc = t("overlayPage.mouseModeDesc");
  const mouseMode = segmented(
    [
      ["clickThrough", t("overlayPage.mouseModeClickThrough")],
      ["drag", t("overlayPage.mouseModeDrag")],
    ],
    (c.frame_click_through ?? true) ? "clickThrough" : "drag",
    (mode) => saveConfig((x) => (x.frame_click_through = mode === "clickThrough")),
  );
  mouseMode.setAttribute("aria-label", mouseModeLabel);
  mouseMode.setAttribute("aria-description", mouseModeDesc);
  mouseMode.querySelectorAll("button").forEach((button, i) => {
    button.dataset.k = `overlay-mouse-mode-${i}`;
  });

  const langs = S.ocrLangs;
  const langPick = dropdown(
    [
      { value: "auto", label: t("overlayPage.ocrAuto"), hint: t("overlayPage.ocrAutoHint") },
      ...(langs ?? []).map((l) => ({ value: l.tag, label: l.name, hint: l.tag })),
    ],
    c.ocr_lang || "auto",
    (v) => saveConfig((x) => (x.ocr_lang = v), t("overlayPage.ocrLangChanged")),
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
    pageHead(t("overlayPage.lead")),
    hero,
    h(
      "div",
      { class: "card flush" },
      row(t("overlayPage.hotkey"), t("overlayPage.hotkeyDesc", { app: t("app.name") }), hotkey),
      row(mouseModeLabel, mouseModeDesc, mouseMode, true),
      row(
        t("overlayPage.mode"),
        c.mode === "vision" ? t("overlayPage.modeVisionDesc") : t("overlayPage.modeOcrDesc"),
        segmented(
          [
            ["ocr", t("overlayPage.modeOcr")],
            ["vision", t("overlayPage.modeVision")],
          ],
          c.mode === "vision" ? "vision" : "ocr",
          (m) => saveConfig((x) => (x.mode = m)),
          ["vision"],
        ),
        true,
      ),
      row(
        t("overlayPage.ocrLang"),
        langs && langs.length === 0 ? t("overlayPage.ocrLangNone") : t("overlayPage.ocrLangDesc"),
        langPick,
      ),
    ),
  );
}
