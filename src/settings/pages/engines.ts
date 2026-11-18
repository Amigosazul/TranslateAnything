// 翻译引擎：翻译方式、传统引擎（主用 + 回退顺序 + 参数）、大模型配置
import {
  S,
  applyMethod,
  cfg,
  engineChain,
  engineInfo,
  kindOf,
  llmId,
  methodOf,
  saveConfig,
  testEngine,
  type ProviderConfig,
} from "../state";
import { t } from "../../i18n";
import { dropdown, field, h, icon, pageHead, section, segmented } from "../ui";

/** 可配置的参数；标签和提示文字取 engines.field.<参数>.label / .ph */
const FIELDS = new Set(["base_url", "api_key", "region", "model", "proxy"]);
const SECRET = new Set(["api_key"]);

/** 大模型的接口地址是必填的，提示换成示例（engines.field.<参数>.llmPh） */
const LLM_PLACEHOLDER = new Set(["base_url"]);

const blank = (): ProviderConfig => ({ base_url: "", api_key: "", region: "", model: "", proxy: null });

function setParam(id: string, key: string, raw: string) {
  const v = raw.trim();
  return saveConfig((c) => {
    const p = (c.engines[id] ??= blank());
    if (key === "proxy") p.proxy = v || null;
    else (p as unknown as Record<string, string>)[key] = v;
  });
}

/** 某个引擎的参数表单；改完一项（失焦或回车）就保存 */
export function paramFields(id: string, fields: string[], llm = false): HTMLElement {
  const p = cfg().engines[id] ?? blank();
  const grid = h("div", { class: "form-grid two" });
  for (const key of fields) {
    if (!FIELDS.has(key)) continue;
    const secret = SECRET.has(key);
    const value = key === "proxy" ? p.proxy ?? "" : ((p as unknown as Record<string, string>)[key] ?? "");
    const input = h("input", {
      type: secret ? "password" : "text",
      value,
      placeholder: llm && LLM_PLACEHOLDER.has(key) ? t(`engines.field.${key}.llmPh`) : t(`engines.field.${key}.ph`),
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
    grid.append(field(t(`engines.field.${key}.label`), control, key === "proxy" || key === "base_url" ? "wide" : ""));
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
function testNote(id: string): HTMLElement | null {
  const r = S.tests[id];
  if (!r || r.busy) return null;
  if (r.error) return h("div", { class: "note bad", style: "margin-top:12px" }, r.error);
  if (r.result) return h("div", { class: "note ok", style: "margin-top:12px" }, `${r.result.source} → ${r.result.text}`);
  return null;
}

function tags(kind: string): HTMLElement[] {
  const info = engineInfo(kind);
  if (!info) return [];
  const out = [h("span", { class: `tag${info.official ? " accent" : ""}` }, info.official ? t("engines.tag.official") : t("engines.tag.free"))];
  if (info.needs_key) out.push(h("span", { class: "tag" }, t("engines.tag.needsKey")));
  return out;
}

export function renderEngines(root: HTMLElement) {
  const c = cfg();
  const method = methodOf(c);
  const chain = engineChain(c);
  const primary = chain[0];
  const fallback = chain.slice(1);
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

  // ── 传统引擎：主用 ──
  const engineList = h("div", { class: "card flush engine-list" });
  for (const e of traditional) {
    const id = e.id;
    const isPrimary = id === primary;
    const fbIndex = fallback.indexOf(id);
    const row = h(
      "div",
      { class: `engine-row${isPrimary ? " selected" : ""}` },
      h(
        "button",
        {
          class: "engine-pick",
          role: "radio",
          "aria-checked": String(isPrimary),
          onclick: () => {
            if (isPrimary) return;
            saveConfig((x) => {
              const ch = engineChain(x).filter((v) => v !== id);
              const order = [id, ...ch];
              if (methodOf(x) === "llm") x.fallback = order;
              else {
                x.engine = id;
                x.fallback = order.slice(1);
              }
            }, t("engines.toast.primary", { name: e.name }));
          },
        },
        h("span", { class: "radio" }),
        h(
          "div",
          { class: "grow" },
          h("div", { class: "engine-name" }, e.name, ...tags(id), fbIndex >= 0 && h("span", { class: "tag" }, t("engines.fallbackNth", { n: fbIndex + 1 }))),
          h("div", { class: "desc" }, e.note),
        ),
      ),
      isPrimary && testButton(id),
    );
    if (isPrimary && e.fields.length) {
      row.append(h("div", { class: "engine-params pop-in" }, paramFields(id, e.fields), testNote(id)));
    } else if (isPrimary) {
      const n = testNote(id);
      if (n) row.append(h("div", { class: "engine-params" }, n));
    }
    engineList.append(row);
  }

  // ── 回退顺序 ──
  const fbCard = h("div", { class: "card" });
  fbCard.append(
    h("h3", {}, method === "llm" ? t("engines.fallback.titleLlm") : t("engines.fallback.title")),
    h("div", { class: "desc" }, t("engines.fallback.desc")),
  );
  const order = method === "llm" ? chain : fallback;
  const fbList = h("div", { class: "fb-list" });
  order.forEach((id, i) => {
    const move = (d: number) =>
      saveConfig((x) => {
        const ch = engineChain(x);
        const list = methodOf(x) === "llm" ? ch : ch.slice(1);
        const [it] = list.splice(i, 1);
        list.splice(i + d, 0, it);
        x.fallback = list;
      }, "");
    const remove = () =>
      saveConfig((x) => {
        const ch = engineChain(x);
        if (methodOf(x) === "llm") x.fallback = ch.filter((v) => v !== id);
        else x.fallback = ch.slice(1).filter((v) => v !== id);
      }, "");
    fbList.append(
      h(
        "div",
        { class: "fb-item" },
        h("span", { class: "fb-num" }, String(i + 1)),
        h("span", { class: "grow" }, engineInfo(kindOf(c, id))?.name ?? id),
        h("button", { class: "icon-btn plain", title: t("engines.fallback.up"), disabled: i === 0, onclick: () => move(-1) }, icon("up")),
        h("button", { class: "icon-btn plain", title: t("engines.fallback.down"), disabled: i === order.length - 1, onclick: () => move(1) }, icon("down")),
        // 大模型方式下第一个是兜底的主力传统引擎，至少保留一个
        !(method === "llm" && order.length === 1) && h("button", { class: "icon-btn", title: t("engines.fallback.remove"), onclick: remove }, icon("x")),
      ),
    );
  });
  if (order.length === 0) fbList.append(h("div", { class: "desc", style: "padding:6px 0" }, t("engines.fallback.none")));
  const candidates = traditional.filter((e) => !chain.includes(e.id));
  fbCard.append(fbList);
  if (candidates.length)
    fbCard.append(
      h(
        "div",
        { class: "row", style: "margin-top:10px" },
        dropdown(
          candidates.map((e) => ({ value: e.id, label: e.name, hint: e.note })),
          "" as string,
          (id) =>
            saveConfig((x) => {
              x.fallback = [...x.fallback.filter((v) => v !== id), id];
            }, ""),
          { placeholder: t("engines.fallback.add"), width: "260px" },
        ),
      ),
    );

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
    pageHead(t("engines.title"), t("engines.lead")),
    methodCard,
    section(t("engines.section.engines"), method === "llm" && h("span", { class: "count" }, t("engines.section.asBackup"))),
    engineList,
    fbCard,
    section(t("engines.llm.title")),
    llmCard,
  );
}
