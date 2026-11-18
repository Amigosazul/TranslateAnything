// 概览：仪表盘式首页，常用状态一眼看清、常用操作一步到位
import {
  S,
  applyMethod,
  cfg,
  engineChain,
  engineInfo,
  langs,
  llmId,
  llmReady,
  methodOf,
  saveConfig,
} from "../state";
import { dropdown, fmt, h, icon, keys, pageHead, segmented, switchEl, type Child, type IconName } from "../ui";
import { go } from "../nav";
import { t } from "../../i18n";
import { setMasterSwitch, toggleAllButton } from "./immersive";
import { closeAllFrames, startSelect } from "./overlay";
import { modelCombo, testButton } from "./engines";

function tile(ic: IconName, title: string, right: Child, ...body: Child[]): HTMLElement {
  return h(
    "div",
    { class: "card tile" },
    h("div", { class: "tile-head" }, h("span", { class: "tile-icon" }, icon(ic)), h("h3", { class: "grow" }, title), right),
    ...body,
  );
}

const more = (tab: string, label = t("dash.details")) => h("button", { class: "link-btn", onclick: () => go(tab) }, label, icon("arrow"));

export function renderDashboard(root: HTMLElement) {
  const c = cfg();
  const im = c.immersive;
  const attached = S.statuses.filter((s) => s.state === "attached");
  const translated = attached.reduce((n, s) => n + s.translated, 0);
  const method = methodOf(c);
  const primary = engineChain(c)[0];
  const lid = llmId(c);
  const llm = c.engines[lid];

  // ── 悬浮窗模式 ──
  const overlay = tile(
    "frame",
    t("nav.overlay"),
    more("overlay"),
    h("div", { class: "tile-big" }, h("b", {}, String(S.frames)), h("span", {}, t("dash.framesOpen"))),
    h("div", { class: "tile-line" }, t("dash.hotkey"), " ", keys(c.hotkey)),
    h(
      "div",
      { class: "tile-actions" },
      h("button", { class: "btn primary sm", onclick: startSelect }, icon("frame"), t("overlayPage.start")),
      h("button", { class: "btn ghost sm", disabled: S.frames === 0, onclick: closeAllFrames }, t("overlayPage.closeAll")),
    ),
  );

  // ── 沉浸模式 ──
  const immersive = tile(
    "apps",
    t("nav.immersive"),
    switchEl(im.enabled, setMasterSwitch),
    h(
      "div",
      { class: "tile-big" },
      h("b", {}, `${attached.length}`),
      h("span", {}, im.enabled ? t("dash.appsAttached", { n: im.targets.length }) : t("dash.masterOff")),
    ),
    h("div", { class: "tile-line" }, attached.length ? t("dash.translatedThisRun", { n: fmt(translated) }) : t("dash.attachHint")),
    h("div", { class: "tile-actions" }, toggleAllButton("btn sm"), h("span", { class: "spacer" }), more("immersive", t("dash.manageApps"))),
  );

  // ── 翻译方式 ──
  const traditional = S.catalog.filter((e) => e.id !== "openai");
  const methodBody: Child[] =
    method === "engine"
      ? [
          h(
            "div",
            { class: "row", style: "gap:8px" },
            dropdown(
              traditional.map((e) => ({ value: e.id, label: e.name, hint: e.note })),
              primary,
              (id) =>
                saveConfig((x) => {
                  const rest = engineChain(x).filter((v) => v !== id);
                  x.engine = id;
                  x.fallback = rest;
                }, t("dash.primaryChanged", { name: engineInfo(id)?.name ?? id })),
              { width: "100%" },
            ),
            testButton(primary, "btn sm"),
          ),
        ]
      : [
          h(
            "div",
            { class: "row", style: "gap:8px" },
            h("div", { class: "grow tile-value" }, llm?.model?.trim() || t("dash.noModel")),
            testButton(lid, "btn sm"),
          ),
        ];
  const translation = tile(
    "engine",
    t("dash.method"),
    more("engines", t("dash.configure")),
    h(
      "div",
      { style: "margin-bottom:14px" },
      segmented(
        [
          ["engine", t("dash.methodEngine")],
          ["llm", t("dash.methodLlm")],
        ],
        method,
        (m) => saveConfig((x) => applyMethod(x, m), m === "llm" ? t("dash.switchedLlm") : t("dash.switchedEngine")),
      ),
    ),
    ...methodBody,
    method === "llm" && !llmReady(c) && h("div", { class: "note warn", style: "margin-top:10px" }, t("dash.llmNotReady")),
  );

  // ── 目标语言 ──
  const language = tile(
    "globe",
    t("general.targetLang"),
    null,
    dropdown(
      langs().map(([v, n]) => ({ value: v, label: n, hint: v })),
      c.target_lang,
      (v) => saveConfig((x) => (x.target_lang = v), t("general.targetLangChanged")),
      { width: "100%" },
    ),
    h("div", { class: "tile-line" }, t("dash.targetLangHint")),
  );

  // ── 大模型 ──
  let host = "";
  try {
    host = llm?.base_url ? new URL(llm.base_url).host : "";
  } catch {
    host = llm?.base_url ?? "";
  }
  const model = modelCombo(
    lid,
    (v) =>
      saveConfig((x) => {
        const p = (x.engines[lid] ??= { base_url: "", api_key: "", region: "", model: "", proxy: null });
        p.model = v;
      }, t("dash.modelChanged")),
    "dash-model",
  );
  const llmTile = tile(
    "sparkle",
    t("dash.methodLlm"),
    more("engines", t("dash.configure")),
    h("label", { class: "field" }, h("span", {}, t("dash.model")), model),
    h(
      "div",
      { class: "kv" },
      h("span", {}, t("dash.endpoint")),
      h("b", { title: llm?.base_url ?? "" }, host || t("common.notSet")),
      h("span", {}, t("dash.apiKey")),
      h("b", {}, llm?.api_key?.trim() ? t("dash.keyFilled") : t("dash.keyEmpty")),
    ),
  );

  // ── 译文库与术语 ──
  const st = S.stats;
  const store = tile(
    "db",
    t("dash.store"),
    more("library"),
    h(
      "div",
      { class: "tile-metrics" },
      h("div", {}, h("b", {}, st ? fmt(st.total) : "—"), h("span", {}, t("dash.entries"))),
      h("div", {}, h("b", {}, st ? fmt(st.hits) : "—"), h("span", {}, t("dash.hits"))),
      h("div", {}, h("b", {}, String(Object.keys(c.glossary).length)), h("span", {}, t("dash.terms"))),
    ),
    h(
      "div",
      { class: "tile-line" },
      c.persist_cache ? t("dash.storeOn") : t("dash.storeOff"),
    ),
  );

  root.replaceChildren(
    pageHead(t("dash.lead")),
    h("div", { class: "tiles" }, overlay, immersive, translation, language, llmTile, store),
  );
}
