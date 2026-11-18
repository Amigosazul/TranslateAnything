// 概览：仪表盘式首页，常用状态一眼看清、常用操作一步到位
import {
  LANGS,
  S,
  applyMethod,
  cfg,
  engineChain,
  engineInfo,
  llmId,
  llmReady,
  methodOf,
  saveConfig,
} from "../state";
import { dropdown, fmt, h, icon, keys, segmented, switchEl, type Child, type IconName } from "../ui";
import { go } from "../nav";
import { setMasterSwitch, toggleAllButton } from "./immersive";
import { closeAllFrames, startSelect } from "./overlay";
import { testButton } from "./engines";

function tile(ic: IconName, title: string, right: Child, ...body: Child[]): HTMLElement {
  return h(
    "div",
    { class: "card tile" },
    h("div", { class: "tile-head" }, h("span", { class: "tile-icon" }, icon(ic)), h("h3", { class: "grow" }, title), right),
    ...body,
  );
}

const more = (tab: string, label = "详情") => h("button", { class: "link-btn", onclick: () => go(tab) }, label, icon("arrow"));

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
    "悬浮窗模式",
    more("overlay"),
    h("div", { class: "tile-big" }, h("b", {}, String(S.frames)), h("span", {}, "个翻译框打开中")),
    h("div", { class: "tile-line" }, "快捷键 ", keys(c.hotkey)),
    h(
      "div",
      { class: "tile-actions" },
      h("button", { class: "btn primary sm", onclick: startSelect }, icon("frame"), "开始框选"),
      h("button", { class: "btn ghost sm", disabled: S.frames === 0, onclick: closeAllFrames }, "全部关闭"),
    ),
  );

  // ── 沉浸模式 ──
  const immersive = tile(
    "apps",
    "沉浸模式",
    switchEl(im.enabled, setMasterSwitch),
    h(
      "div",
      { class: "tile-big" },
      h("b", {}, `${attached.length}`),
      h("span", {}, im.enabled ? `/ ${im.targets.length} 个应用已接管` : "总开关已关闭"),
    ),
    h("div", { class: "tile-line" }, attached.length ? `本次已回填 ${fmt(translated)} 条译文` : "启动已添加的应用即可自动接管"),
    h("div", { class: "tile-actions" }, toggleAllButton("btn sm"), h("span", { class: "spacer" }), more("immersive", "管理应用")),
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
                }, `主用引擎已改为「${engineInfo(id)?.name ?? id}」`),
              { width: "100%" },
            ),
            testButton(primary, "btn sm"),
          ),
        ]
      : [
          h(
            "div",
            { class: "row", style: "gap:8px" },
            h("div", { class: "grow tile-value" }, llm?.model?.trim() || "未设置模型"),
            testButton(lid, "btn sm"),
          ),
        ];
  const translation = tile(
    "engine",
    "翻译方式",
    more("engines", "配置"),
    h(
      "div",
      { style: "margin-bottom:14px" },
      segmented(
        [
          ["engine", "翻译引擎"],
          ["llm", "大模型"],
        ],
        method,
        (m) => saveConfig((x) => applyMethod(x, m), m === "llm" ? "已切换为大模型翻译" : "已切换为翻译引擎"),
      ),
    ),
    ...methodBody,
    method === "llm" && !llmReady(c) && h("div", { class: "note warn", style: "margin-top:10px" }, "大模型还没配置好，翻译会先退回传统引擎。"),
  );

  // ── 目标语言 ──
  const language = tile(
    "globe",
    "目标语言",
    null,
    dropdown(
      LANGS.map(([v, n]) => ({ value: v, label: n, hint: v })),
      c.target_lang,
      (v) => saveConfig((x) => (x.target_lang = v), "目标语言已更改"),
      { width: "100%" },
    ),
    h("div", { class: "tile-line" }, "悬浮窗和沉浸模式都翻译成这种语言。"),
  );

  // ── 大模型 ──
  let host = "";
  try {
    host = llm?.base_url ? new URL(llm.base_url).host : "";
  } catch {
    host = llm?.base_url ?? "";
  }
  const model = h("input", { type: "text", value: llm?.model ?? "", placeholder: "例如 gpt-4o-mini", "data-k": "dash-model", spellcheck: false });
  model.addEventListener("change", () =>
    saveConfig((x) => {
      const p = (x.engines[lid] ??= { base_url: "", api_key: "", region: "", model: "", proxy: null });
      p.model = model.value.trim();
    }, "模型已更改"),
  );
  const llmTile = tile(
    "sparkle",
    "大模型",
    more("engines", "配置"),
    h("label", { class: "field" }, h("span", {}, "模型"), model),
    h(
      "div",
      { class: "kv" },
      h("span", {}, "接口"),
      h("b", { title: llm?.base_url ?? "" }, host || "未设置"),
      h("span", {}, "密钥"),
      h("b", {}, llm?.api_key?.trim() ? "已填写" : "未填写"),
    ),
  );

  // ── 译文库与术语 ──
  const st = S.stats;
  const store = tile(
    "db",
    "译文库与术语",
    more("library"),
    h(
      "div",
      { class: "tile-metrics" },
      h("div", {}, h("b", {}, st ? fmt(st.total) : "—"), h("span", {}, "条译文")),
      h("div", {}, h("b", {}, st ? fmt(st.hits) : "—"), h("span", {}, "次直接命中")),
      h("div", {}, h("b", {}, String(Object.keys(c.glossary).length)), h("span", {}, "条自定义术语")),
    ),
    h(
      "div",
      { class: "tile-line" },
      c.persist_cache ? "译文库已开启，相同原文不再请求引擎。" : "译文库已关闭，每次都会请求引擎。",
    ),
  );

  root.replaceChildren(
    h(
      "header",
      { class: "page-head" },
      h("div", { class: "grow" }, h("h1", {}, "概览"), h("p", { class: "lead" }, "常用开关和状态都在这里。")),
    ),
    h("div", { class: "tiles" }, overlay, immersive, translation, language, llmTile, store),
  );
}
