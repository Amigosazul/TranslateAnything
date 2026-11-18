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
import { dropdown, field, h, icon, pageHead, section, segmented } from "../ui";

const FIELD_META: Record<string, { label: string; placeholder: string; secret?: boolean }> = {
  base_url: { label: "接口地址", placeholder: "留空使用默认地址" },
  api_key: { label: "密钥", placeholder: "在服务商后台创建", secret: true },
  region: { label: "区域", placeholder: "例如 eastasia" },
  model: { label: "模型", placeholder: "例如 gpt-4o-mini" },
  proxy: { label: "代理", placeholder: "留空直连；填 system 跟随系统；或填写代理地址，如 http://127.0.0.1:7897" },
};

/** 大模型的接口地址是必填的，提示换成示例 */
const LLM_PLACEHOLDER: Record<string, string> = { base_url: "例如 https://api.openai.com/v1" };

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
    const meta = FIELD_META[key];
    if (!meta) continue;
    const value = key === "proxy" ? p.proxy ?? "" : ((p as unknown as Record<string, string>)[key] ?? "");
    const input = h("input", {
      type: meta.secret ? "password" : "text",
      value,
      placeholder: (llm && LLM_PLACEHOLDER[key]) || meta.placeholder,
      "data-k": `eng-${id}-${key}`,
      autocomplete: "off",
      spellcheck: false,
    });
    input.addEventListener("change", () => setParam(id, key, input.value));
    let control: HTMLElement = input;
    if (meta.secret) {
      const eye = h("button", { type: "button", class: "input-icon", title: "显示 / 隐藏", onclick: () => (input.type = input.type === "password" ? "text" : "password") }, icon("eye"));
      control = h("div", { class: "input-wrap" }, input, eye);
    }
    grid.append(field(meta.label, control, key === "proxy" || key === "base_url" ? "wide" : ""));
  }
  return grid;
}

/** "测试"按钮：显示延迟（绿色）或失败（红色，悬停看原因） */
export function testButton(id: string, cls = "btn sm"): HTMLElement {
  const t = S.tests[id];
  const label = t?.busy
    ? "测试中…"
    : t?.result
      ? `${t.result.ms} 毫秒`
      : t?.error
        ? "测试失败"
        : "测试";
  const tone = t?.busy ? "" : t?.result ? (t.result.ms > 2000 ? " slow" : " good") : t?.error ? " fail" : "";
  const title = t?.result ? `${t.result.source}\n→ ${t.result.text}` : t?.error ?? "用一句英文试翻，测一下能否连通和耗时";
  return h("button", { class: `${cls} test-btn${tone}`, disabled: t?.busy, title, onclick: () => testEngine(id) }, icon("bolt"), label);
}

/** 测试结果的说明行（失败原因或示例译文） */
function testNote(id: string): HTMLElement | null {
  const t = S.tests[id];
  if (!t || t.busy) return null;
  if (t.error) return h("div", { class: "note bad", style: "margin-top:12px" }, t.error);
  if (t.result) return h("div", { class: "note ok", style: "margin-top:12px" }, `${t.result.source} → ${t.result.text}`);
  return null;
}

function tags(kind: string): HTMLElement[] {
  const info = engineInfo(kind);
  if (!info) return [];
  const out = [h("span", { class: `tag${info.official ? " accent" : ""}` }, info.official ? "官方接口" : "免费")];
  if (info.needs_key) out.push(h("span", { class: "tag" }, "需要密钥"));
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
        h("h3", {}, "翻译方式"),
        h(
          "div",
          { class: "desc" },
          method === "llm"
            ? "由大模型翻译，译文更通顺但更慢；大模型出错时自动退回下面的翻译引擎。"
            : "由传统翻译引擎翻译，速度快、免费引擎无需配置。",
        ),
      ),
      segmented(
        [
          ["engine", "翻译引擎"],
          ["llm", "大模型"],
        ],
        method,
        (m) => saveConfig((x) => applyMethod(x, m), m === "llm" ? "已切换为大模型翻译" : "已切换为翻译引擎"),
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
            }, `主用引擎已改为「${e.name}」`);
          },
        },
        h("span", { class: "radio" }),
        h(
          "div",
          { class: "grow" },
          h("div", { class: "engine-name" }, e.name, ...tags(id), fbIndex >= 0 && h("span", { class: "tag" }, `回退第 ${fbIndex + 1}`)),
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
    h("h3", {}, method === "llm" ? "大模型出错时依次尝试" : "主用引擎出错时依次尝试"),
    h("div", { class: "desc" }, "按顺序逐个尝试，直到有一个成功。"),
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
        h("button", { class: "icon-btn plain", title: "上移", disabled: i === 0, onclick: () => move(-1) }, icon("up")),
        h("button", { class: "icon-btn plain", title: "下移", disabled: i === order.length - 1, onclick: () => move(1) }, icon("down")),
        // 大模型方式下第一个是兜底的主力传统引擎，至少保留一个
        !(method === "llm" && order.length === 1) && h("button", { class: "icon-btn", title: "移除", onclick: remove }, icon("x")),
      ),
    );
  });
  if (order.length === 0) fbList.append(h("div", { class: "desc", style: "padding:6px 0" }, "未设置回退，主用引擎出错时直接报错。"));
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
          { placeholder: "添加回退引擎…", width: "260px" },
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
      h("div", { class: "grow" }, h("h3", {}, "大模型"), h("div", { class: "desc" }, "任意 OpenAI 兼容服务（官方、各家云服务或本地部署）。")),
      testButton(lid),
    ),
    h("div", { style: "margin-top:16px" }, paramFields(lid, ["base_url", "api_key", "model", "proxy"], true)),
    testNote(lid),
  );

  root.replaceChildren(
    pageHead("翻译引擎", "选择翻译方式、主用引擎和出错时的回退顺序。修改后立即生效，命中译文库的文字不受影响。"),
    methodCard,
    section("翻译引擎", method === "llm" && h("span", { class: "count" }, "作为大模型的后备")),
    engineList,
    fbCard,
    section("大模型"),
    llmCard,
  );
}
