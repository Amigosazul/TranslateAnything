// 翻译引擎：翻译方式、传统引擎（启用 / 停用 + 拖动排序 = 调用顺序 + 参数）、大模型配置
import {
  S,
  applyMethod,
  cfg,
  engineChain,
  engineInfo,
  kindOf,
  llmId,
  methodOf,
  refresh,
  saveConfig,
  setEngineChain,
  saving,
  settled,
  testEngine,
  type ProviderConfig,
} from "../state";
import { invoke } from "@tauri-apps/api/core";
import { t } from "../../i18n";
import { combo, dropdown, field, h, icon, pageHead, section, segmented, switchEl, toast, type DropOption } from "../ui";

/** 可配置的参数；标签和提示文字取 engines.field.<参数>.label / .ph */
const FIELDS = new Set(["base_url", "api_key", "region", "model", "proxy"]);
const SECRET = new Set(["api_key"]);

/** 大模型的接口地址是必填的，提示换成示例（engines.field.<参数>.llmPh） */
const LLM_PLACEHOLDER = new Set(["base_url"]);

/** 展开了参数区的引擎（只在本次打开设置窗口期间记住） */
const opened = new Set<string>();

const blank = (): ProviderConfig => ({ base_url: "", api_key: "", region: "", model: "", proxy: null });

function setParam(id: string, key: string, raw: string) {
  const v = raw.trim();
  return saveConfig((c) => {
    const p = (c.engines[id] ??= blank());
    if (key === "proxy") p.proxy = v || null;
    else (p as unknown as Record<string, string>)[key] = v;
  });
}

// ───────────────────────── 候选项 ─────────────────────────
// 用户多半不知道这些参数能填什么，给出常用值供挑选，同时允许自己填写

/** Azure 区域代码与英文显示名（与 Azure 门户一致）；global 的说明取 engines.region.global */
const AZURE_REGIONS: [string, string][] = [
  ["eastasia", "East Asia (Hong Kong)"],
  ["southeastasia", "Southeast Asia (Singapore)"],
  ["japaneast", "Japan East"],
  ["japanwest", "Japan West"],
  ["koreacentral", "Korea Central"],
  ["centralindia", "Central India"],
  ["australiaeast", "Australia East"],
  ["eastus", "East US"],
  ["eastus2", "East US 2"],
  ["westus", "West US"],
  ["westus2", "West US 2"],
  ["westus3", "West US 3"],
  ["centralus", "Central US"],
  ["northcentralus", "North Central US"],
  ["southcentralus", "South Central US"],
  ["westcentralus", "West Central US"],
  ["canadacentral", "Canada Central"],
  ["brazilsouth", "Brazil South"],
  ["northeurope", "North Europe"],
  ["westeurope", "West Europe"],
  ["uksouth", "UK South"],
  ["francecentral", "France Central"],
  ["germanywestcentral", "Germany West Central"],
  ["switzerlandnorth", "Switzerland North"],
  ["norwayeast", "Norway East"],
  ["swedencentral", "Sweden Central"],
  ["uaenorth", "UAE North"],
  ["qatarcentral", "Qatar Central"],
  ["southafricanorth", "South Africa North"],
  ["chinaeast2", "China East 2 (21Vianet)"],
  ["chinanorth3", "China North 3 (21Vianet)"],
];

function regionOptions(): DropOption<string>[] {
  return [
    { value: "global", label: t("engines.region.global"), hint: "global" },
    ...AZURE_REGIONS.map(([value, label]) => ({ value, label, hint: value })),
  ];
}

/**
 * 各引擎接口地址的常用值。label 以 "@" 开头的取文案键 engines.preset.<名>，其余是服务商名称原样显示。
 * 大模型（openai）列的都是 OpenAI 兼容接口
 */
const BASE_PRESETS: Record<string, [string, string][]> = {
  openai: [
    ["https://api.openai.com/v1", "OpenAI"],
    ["https://api.anthropic.com/v1", "Anthropic Claude"],
    ["https://generativelanguage.googleapis.com/v1beta/openai", "Google Gemini"],
    ["https://api.deepseek.com/v1", "DeepSeek"],
    ["https://dashscope.aliyuncs.com/compatible-mode/v1", "@dashscope"],
    ["https://ark.cn-beijing.volces.com/api/v3", "@volcengine"],
    ["https://open.bigmodel.cn/api/paas/v4", "@zhipu"],
    ["https://api.moonshot.cn/v1", "@moonshot"],
    ["https://api.siliconflow.cn/v1", "@siliconflow"],
    ["https://openrouter.ai/api/v1", "OpenRouter"],
    ["https://api.groq.com/openai/v1", "Groq"],
    ["https://api.mistral.ai/v1", "Mistral"],
    ["https://api.x.ai/v1", "xAI Grok"],
    ["http://localhost:11434/v1", "@ollama"],
    ["http://localhost:1234/v1", "@lmstudio"],
  ],
  microsoft: [
    ["", "@msGlobal"],
    ["https://api-apc.cognitive.microsofttranslator.com", "@msApc"],
    ["https://api-eur.cognitive.microsofttranslator.com", "@msEur"],
    ["https://api-nam.cognitive.microsofttranslator.com", "@msNam"],
    ["https://api.translator.azure.cn", "@msChina"],
  ],
  deepl: [
    ["", "@deeplAuto"],
    ["https://api-free.deepl.com", "@deeplFree"],
    ["https://api.deepl.com", "@deeplPro"],
  ],
  deeplx: [["http://127.0.0.1:1188/translate", "@deeplxLocal"]],
};

function presetOptions(kind: string): DropOption<string>[] | null {
  const list = BASE_PRESETS[kind];
  if (!list) return null;
  return list.map(([value, label]) => ({
    value,
    label: label.startsWith("@") ? t(`engines.preset.${label.slice(1)}`) : label,
    hint: value || t("engines.preset.defaultHint"),
  }));
}

/** 获取到的模型列表，按"地址 + 密钥 + 代理"缓存，界面重画后不必重新请求 */
const modelCache = new Map<string, string[]>();

/** 大模型的模型输入框：点右侧按钮从接口获取可用模型，也可以直接填写 */
export function modelCombo(id: string, onCommit: (v: string) => void, key = `eng-${id}-model`): HTMLElement {
  const cacheKey = () => {
    const p = cfg().engines[id] ?? blank();
    return `${p.base_url}\n${p.api_key}\n${p.proxy ?? ""}\n${cfg().proxy ?? ""}`;
  };
  const toOptions = (ids: string[]) => ids.map((m) => ({ value: m, label: m }));
  const cachedOptions = () => {
    // 刚改的地址 / 密钥还在保存时不信缓存，交给 load 等保存完再判断
    if (saving()) return undefined;
    const ids = modelCache.get(cacheKey());
    return ids && toOptions(ids);
  };
  return combo(cfg().engines[id]?.model ?? "", onCommit, {
    key,
    placeholder: t("engines.field.model.ph"),
    title: t("engines.model.fetch"),
    // 地址、密钥、代理未变时直接用已获取的列表（输入筛选和点按钮展开）；变了才重新获取
    options: cachedOptions(),
    cached: cachedOptions,
    load: async () => {
      // 刚改的接口地址 / 密钥可能还在保存，等保存完再按新配置获取
      await settled();
      if (!cfg().engines[id]?.base_url?.trim()) throw new Error(t("engines.model.needBase"));
      const hit = modelCache.get(cacheKey());
      if (hit) return toOptions(hit);
      const ids = await invoke<string[]>("llm_models", { id });
      modelCache.set(cacheKey(), ids);
      return toOptions(ids);
    },
  });
}

/** 代理的几种取值：跟随全局（仅单个引擎）/ 直连 / 系统代理 / 自定义地址 */
type ProxyMode = "" | "direct" | "system" | "custom";
const PROXY_SUGGEST: DropOption<string>[] = [
  { value: "http://127.0.0.1:7890", label: "http://127.0.0.1:7890", hint: "Clash / Mihomo" },
  { value: "http://127.0.0.1:7897", label: "http://127.0.0.1:7897", hint: "Clash Verge Rev" },
  { value: "http://127.0.0.1:10809", label: "http://127.0.0.1:10809", hint: "v2rayN" },
  { value: "http://127.0.0.1:1080", label: "http://127.0.0.1:1080", hint: "Shadowsocks" },
];
/** 选了"自定义"但还没填地址的代理框（键：引擎 id，全局为空串） */
const customProxy = new Set<string>();

/**
 * 代理选择：先选方式，选"自定义"再填地址。
 * `scope` 为引擎 id 时多一个"跟随全局"（值为空）；为 null 时是全局代理，空值即直连
 */
export function proxyField(value: string, scope: string | null, onCommit: (v: string) => void): HTMLElement {
  const engine = scope !== null;
  const tag = scope ?? "";
  const v = value.trim();
  let mode: ProxyMode = v === "system" ? "system" : v === "direct" ? (engine ? "direct" : "") : v ? "custom" : "";
  if (!v && customProxy.has(tag)) mode = "custom";
  const modes: DropOption<ProxyMode>[] = [
    engine ? { value: "", label: t("proxy.followGlobal"), hint: t("proxy.followGlobalHint") } : { value: "", label: t("proxy.direct") },
    ...(engine ? [{ value: "direct" as ProxyMode, label: t("proxy.direct") }] : []),
    { value: "system", label: t("proxy.system"), hint: t("proxy.systemHint") },
    { value: "custom", label: t("proxy.custom"), hint: t("proxy.customHint") },
  ];
  const pick = dropdown(
    modes,
    mode,
    (m) => {
      if (m === "custom") {
        customProxy.add(tag);
        refresh();
        return;
      }
      customProxy.delete(tag);
      onCommit(m);
    },
    { width: "100%" },
  );
  if (mode !== "custom") return pick;
  const addr = combo(
    v,
    (a) => {
      if (a) customProxy.delete(tag);
      onCommit(a);
    },
    { key: `proxy-${tag}`, placeholder: t("proxy.customPh"), options: PROXY_SUGGEST },
  );
  return h("div", { class: "proxy-field" }, pick, addr);
}

/** 某个引擎的参数表单；改完一项（失焦、回车或选中候选项）就保存 */
export function paramFields(id: string, fields: string[], llm = false): HTMLElement {
  const p = cfg().engines[id] ?? blank();
  const kind = kindOf(cfg(), id);
  const grid = h("div", { class: "form-grid two" });
  for (const key of fields) {
    if (!FIELDS.has(key)) continue;
    const secret = SECRET.has(key);
    const value = key === "proxy" ? p.proxy ?? "" : ((p as unknown as Record<string, string>)[key] ?? "");
    const label = t(`engines.field.${key}.label`);
    const wide = key === "proxy" || key === "base_url" ? "wide" : "";
    const save = (v: string) => void setParam(id, key, v);
    if (key === "model") {
      grid.append(field(label, modelCombo(id, save), wide));
      continue;
    }
    if (key === "proxy") {
      grid.append(field(label, proxyField(value, id, save), wide));
      continue;
    }
    const ph = llm && LLM_PLACEHOLDER.has(key) ? t(`engines.field.${key}.llmPh`) : t(`engines.field.${key}.ph`);
    if (key === "region") {
      grid.append(field(label, combo(value, save, { key: `eng-${id}-region`, placeholder: ph, options: regionOptions() }), wide));
      continue;
    }
    const presets = key === "base_url" ? presetOptions(kind) : null;
    if (presets) {
      grid.append(field(label, combo(value, save, { key: `eng-${id}-base_url`, placeholder: ph, options: presets }), wide));
      continue;
    }
    const input = h("input", {
      type: secret ? "password" : "text",
      value,
      placeholder: ph,
      "data-k": `eng-${id}-${key}`,
      autocomplete: "off",
      spellcheck: false,
    });
    input.addEventListener("change", () => setParam(id, key, input.value));
    let control: HTMLElement = input;
    if (secret) {
      const eye = h("button", { type: "button", class: "input-icon", title: t("engines.toggleSecret"), onclick: () => (input.type = input.type === "password" ? "text" : "password") }, icon("eye"));
      control = h("div", { class: "input-wrap" }, input, eye);
    }
    grid.append(field(label, control, wide));
  }
  return grid;
}

/** "测试"按钮：显示延迟（绿色）或失败（红色，悬停看原因） */
export function testButton(id: string, cls = "btn sm"): HTMLElement {
  const r = S.tests[id];
  const label = r?.busy
    ? t("engines.test.busy")
    : r?.result
      ? t("engines.test.ms", { ms: r.result.ms })
      : r?.error
        ? t("engines.test.failed")
        : t("engines.test.label");
  const tone = r?.busy ? "" : r?.result ? (r.result.ms > 2000 ? " slow" : " good") : r?.error ? " fail" : "";
  const title = r?.result ? `${r.result.source}\n→ ${r.result.text}` : r?.error ?? t("engines.test.hint");
  return h("button", { class: `${cls} test-btn${tone}`, disabled: r?.busy, title, onclick: () => testEngine(id) }, icon("bolt"), label);
}

/** 测试结果的说明行（失败原因或示例译文） */
function testNote(id: string, style = "margin-top:12px"): HTMLElement | null {
  const r = S.tests[id];
  if (!r || r.busy) return null;
  if (r.error) return h("div", { class: "note bad", style }, r.error);
  if (r.result) return h("div", { class: "note ok", style }, `${r.result.source} → ${r.result.text}`);
  return null;
}

function tags(kind: string): HTMLElement[] {
  const info = engineInfo(kind);
  if (!info) return [];
  const out = [h("span", { class: `tag${info.official ? " accent" : ""}` }, info.official ? t("engines.tag.official") : t("engines.tag.free"))];
  if (info.needs_key) out.push(h("span", { class: "tag" }, t("engines.tag.needsKey")));
  return out;
}

/** 启用：先用已保存的参数测试，通过了才加到调用顺序末尾 */
async function enable(id: string, name: string) {
  const ok = await testEngine(id);
  if (!ok) {
    opened.add(id);
    refresh();
    toast(t("engines.toast.testFirst", { name }), true);
    return;
  }
  const n = engineChain(cfg()).filter((v) => v !== id).length + 1;
  await saveConfig((x) => setEngineChain(x, [...engineChain(x).filter((v) => v !== id), id]), t("engines.toast.enabled", { name, n }));
}

function disable(id: string, name: string) {
  return saveConfig((x) => setEngineChain(x, engineChain(x).filter((v) => v !== id)), t("engines.toast.disabled", { name }));
}

/** 键盘调整顺序：拖动手柄聚焦时按上 / 下方向键 */
function moveBy(id: string, d: number) {
  const ch = engineChain(cfg());
  const i = ch.indexOf(id);
  const j = i + d;
  if (i < 0 || j < 0 || j >= ch.length) return;
  [ch[i], ch[j]] = [ch[j], ch[i]];
  void saveConfig((x) => setEngineChain(x, ch), "");
}

function engineRow(id: string, chain: string[]): HTMLElement {
  const c = cfg();
  const info = engineInfo(kindOf(c, id));
  const name = info?.name ?? id;
  const fields = info?.fields ?? [];
  const pos = chain.indexOf(id);
  const on = pos >= 0;
  const last = on && chain.length === 1;
  const open = opened.has(id) && fields.length > 0;
  const busy = !!S.tests[id]?.busy;

  const grip = h(
    "button",
    {
      class: "engine-grip",
      type: "button",
      disabled: !on,
      title: on ? t("engines.order.drag") : t("engines.order.dragOff"),
      "aria-label": t("engines.order.drag"),
      "data-k": `grip-${id}`,
      onkeydown: (e: KeyboardEvent) => {
        if (e.key === "ArrowUp" || e.key === "ArrowDown") {
          e.preventDefault();
          moveBy(id, e.key === "ArrowUp" ? -1 : 1);
        }
      },
    },
    icon("grip"),
  );

  const sw = switchEl(on, (v) => void (v ? enable(id, name) : disable(id, name)), busy || last);
  sw.title = last ? t("engines.order.keepOne") : on ? t("engines.order.off") : t("engines.order.on");

  const row = h(
    "div",
    { class: `engine-row${on ? " on" : ""}${open ? " open" : ""}`, "data-id": id },
    grip,
    h(
      "button",
      {
        class: `engine-pick${fields.length ? "" : " static"}`,
        type: "button",
        "aria-expanded": fields.length ? String(open) : undefined,
        title: fields.length ? t("engines.order.params") : undefined,
        onclick: () => {
          if (!fields.length) return;
          if (opened.has(id)) opened.delete(id);
          else opened.add(id);
          refresh();
        },
      },
      h("span", { class: "engine-num" }, on ? String(pos + 1) : ""),
      h("div", { class: "grow" }, h("div", { class: "engine-name" }, name, ...tags(kindOf(c, id))), h("div", { class: "desc" }, info?.note ?? "")),
      fields.length > 0 && h("span", { class: "engine-chev" }, icon("down")),
    ),
    testButton(id),
    sw,
  );
  if (open) row.append(h("div", { class: "engine-params pop-in" }, paramFields(id, fields), testNote(id)));
  else {
    const n = testNote(id, "");
    if (n) row.append(h("div", { class: "engine-params" }, n));
  }
  return row;
}

/**
 * 按住手柄拖动排序（只在启用的引擎之间移动）。
 * 用指针事件自己实现，不用 HTML5 拖放——Tauri 在 Windows 上会接管系统拖放。
 */
function sortable(list: HTMLElement, onDrop: (ids: string[]) => void) {
  const enabledRows = () => [...list.querySelectorAll<HTMLElement>(".engine-row.on")];
  const order = () => enabledRows().map((r) => r.dataset.id!);

  // 被挤开的行从原位置平滑滑到新位置
  const flip = (el: HTMLElement, mutate: () => void) => {
    const a = el.offsetTop;
    mutate();
    const b = el.offsetTop;
    el.style.transition = "none";
    el.style.transform = `translateY(${a - b}px)`;
    void el.offsetHeight;
    el.style.transition = "";
    el.style.transform = "";
  };

  list.addEventListener("pointerdown", (e) => {
    const grip = (e.target as Element).closest<HTMLButtonElement>(".engine-grip");
    if (!grip || grip.disabled || e.button !== 0) return;
    const row = grip.closest<HTMLElement>(".engine-row")!;
    e.preventDefault();
    grip.setPointerCapture(e.pointerId);
    const before = order().join();
    const listTop = () => list.getBoundingClientRect().top;
    const offset = e.clientY - listTop() - row.offsetTop;
    row.classList.add("dragging");
    list.classList.add("sorting");

    const place = (clientY: number) => {
      const rows = enabledRows();
      const first = rows[0];
      const lastRow = rows[rows.length - 1];
      let top = clientY - listTop() - offset;
      top = Math.max(first.offsetTop, Math.min(top, lastRow.offsetTop + lastRow.offsetHeight - row.offsetHeight));
      for (let guard = 0; guard < rows.length; guard++) {
        const cur = enabledRows();
        const i = cur.indexOf(row);
        const prev = cur[i - 1];
        const next = cur[i + 1];
        // 只挪邻居、不挪被拖的行本身：节点一旦被移出 DOM，指针捕获就丢了
        if (prev && top < prev.offsetTop + prev.offsetHeight / 2) flip(prev, () => list.insertBefore(prev, row.nextSibling));
        else if (next && top + row.offsetHeight > next.offsetTop + next.offsetHeight / 2) flip(next, () => list.insertBefore(next, row));
        else break;
      }
      row.style.transform = `translateY(${top - row.offsetTop}px)`;
    };

    const move = (ev: PointerEvent) => place(ev.clientY);
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      row.style.transform = "";
      row.classList.remove("dragging");
      list.classList.remove("sorting");
      const after = order();
      if (after.join() !== before) onDrop(after);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  });
}

export function renderEngines(root: HTMLElement) {
  const c = cfg();
  const method = methodOf(c);
  const chain = engineChain(c);
  const traditional = S.catalog.filter((e) => e.id !== "openai");

  // ── 翻译方式 ──
  const methodCard = h(
    "div",
    { class: "card" },
    h(
      "div",
      { class: "row spread", style: "gap:16px" },
      h(
        "div",
        { class: "grow" },
        h("h3", {}, t("engines.method.title")),
        h(
          "div",
          { class: "desc" },
          method === "llm"
            ? t("engines.method.llmDesc")
            : t("engines.method.engineDesc"),
        ),
      ),
      segmented(
        [
          ["engine", t("engines.method.engine")],
          ["llm", t("engines.method.llm")],
        ],
        method,
        (m) => saveConfig((x) => applyMethod(x, m), m === "llm" ? t("engines.toast.toLlm") : t("engines.toast.toEngine")),
      ),
    ),
  );

  // ── 传统引擎：启用的按调用顺序排在前面，停用的排在后面 ──
  const ids = [...chain, ...traditional.map((e) => e.id).filter((id) => !chain.includes(id))];
  const engineList = h("div", { class: "card flush engine-list" });
  for (const id of ids) engineList.append(engineRow(id, chain));
  sortable(engineList, (order) => saveConfig((x) => setEngineChain(x, order), t("engines.toast.order")));

  // ── 大模型 ──
  const lid = llmId(c);
  const llmCard = h(
    "div",
    { class: `card${method === "llm" ? " ring" : ""}` },
    h(
      "div",
      { class: "row spread" },
      h("div", { class: "grow" }, h("h3", {}, t("engines.llm.title")), h("div", { class: "desc" }, t("engines.llm.desc"))),
      testButton(lid),
    ),
    h("div", { style: "margin-top:16px" }, paramFields(lid, ["base_url", "api_key", "model", "proxy"], true)),
    testNote(lid),
  );

  root.replaceChildren(
    pageHead(t("engines.lead")),
    methodCard,
    section(t("engines.section.engines"), method === "llm" && h("span", { class: "count" }, t("engines.section.asBackup"))),
    h("div", { class: "desc list-hint" }, method === "llm" ? t("engines.order.descLlm") : t("engines.order.desc")),
    engineList,
    section(t("engines.llm.title")),
    llmCard,
  );
}
