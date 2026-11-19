// 悬浮窗模式：框选入口、快捷键、框内鼠标操作与识别设置
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { clampFrameRadius, FRAME_RADIUS_MAX, FRAME_RADIUS_PREVIEW_EVENT } from "../../frame/appearance";
import { S, cfg, refresh, saveConfig, settled } from "../state";
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

let radiusEdit = 0;
let radiusPreviewRequests = Promise.resolve();

/** 依次推送预览和清除事件，避免上一轮清除覆盖新预览。 */
function publishRadiusPreview(radius: number | null) {
  radiusPreviewRequests = radiusPreviewRequests.then(() => emit(FRAME_RADIUS_PREVIEW_EVENT, radius))
    .catch((error) => console.error("圆角预览同步失败", error));
}

/** 越界数值即时归到边界，有效整数同步预览，回车或失焦后保存。 */
function cornerRadiusInput(label: string, description: string): HTMLElement {
  const savedRadius = () => clampFrameRadius(cfg().frame_corner_radius ?? 0);
  const input = h("input", {
    type: "number", min: "0", max: String(FRAME_RADIUS_MAX), step: "1", inputmode: "numeric",
    value: String(savedRadius()), "aria-label": label, "aria-description": description,
    "data-k": "overlay-corner-radius",
  });
  const control = h("div", { class: "radius-control" }, input, h("span", { "aria-hidden": "true" }, "px"));
  const validValue = () => input.value && input.validity.valid ? input.valueAsNumber : null;
  let changed = false;
  input.addEventListener("input", () => {
    radiusEdit++;
    changed = true;
    if (input.validity.rangeOverflow || input.validity.rangeUnderflow) {
      input.value = String(clampFrameRadius(input.valueAsNumber));
    }
    publishRadiusPreview(validValue());
  });

  const finish = async (cancel = false) => {
    if (!changed) return;
    changed = false;
    const revision = ++radiusEdit;
    const radius = cancel ? null : validValue();
    if (radius !== null) {
      await saveConfig((c) => (c.frame_corner_radius = radius), "", { redraw: false });
    } else {
      await settled();
    }
    if (revision !== radiusEdit) return;
    input.value = String(savedRadius());
    publishRadiusPreview(null);
  };
  input.addEventListener("blur", () => void finish());
  input.addEventListener("keydown", (event) => {
    if (event.isComposing || (event.key !== "Enter" && event.key !== "Escape")) return;
    event.preventDefault();
    void finish(event.key === "Escape");
  });
  return control;
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

  const radiusLabel = t("overlayPage.cornerRadius");
  const radiusDesc = t("overlayPage.cornerRadiusDesc", { max: FRAME_RADIUS_MAX });
  const radius = cornerRadiusInput(radiusLabel, radiusDesc);

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
      row(radiusLabel, radiusDesc, radius),
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
