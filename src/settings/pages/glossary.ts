// 术语表：自定义术语增删改、内置术语表开关与查看
import { S, cfg, saveConfig } from "../state";
import { emptyState, h, icon, pageHead, switchEl, toast } from "../ui";

let draft: [string, string][] | null = null;
/** 自定义术语有未保存的修改（此时配置变动不覆盖正在编辑的内容） */
let dirty = false;
let builtinFilter = "";

export const glossaryDirty = () => dirty;
export function resetGlossaryDraft() {
  if (!dirty) draft = null;
}

export function renderGlossary(root: HTMLElement) {
  if (!draft) draft = Object.entries(cfg().glossary);
  const rows = draft;
  const rerender = () => renderGlossary(root);

  const tbody = h("tbody");
  rows.forEach((row, i) => {
    const src = h("input", { type: "text", value: row[0], placeholder: "原文（整条精确匹配）", "data-k": `gl-s-${i}` });
    const dst = h("input", { type: "text", value: row[1], placeholder: "译文（与原文相同即保持不翻）", "data-k": `gl-t-${i}` });
    src.addEventListener("input", () => ((row[0] = src.value), (dirty = true)));
    dst.addEventListener("input", () => ((row[1] = dst.value), (dirty = true)));
    tbody.append(
      h(
        "tr",
        {},
        h("td", {}, src),
        h("td", {}, dst),
        h(
          "td",
          {},
          h("button", { class: "icon-btn", title: "删除", "aria-label": "删除", onclick: () => (rows.splice(i, 1), (dirty = true), rerender()) }, icon("trash")),
        ),
      ),
    );
  });
  if (rows.length === 0) tbody.append(h("tr", {}, h("td", { colspan: 3 }, emptyState("还没有自定义术语"))));

  const save = async () => {
    const map: Record<string, string> = {};
    for (const [k, v] of rows) {
      if (!k.trim()) continue;
      if (!v.trim()) {
        toast(`「${k}」的译文不能为空`, true);
        return;
      }
      map[k.trim()] = v;
    }
    dirty = false;
    draft = null;
    const ok = await saveConfig((c) => (c.glossary = map), "术语表已保存，已接管的页面会立即更新");
    if (!ok) {
      dirty = true;
      draft = rows;
      rerender();
    }
  };

  const builtin = Object.entries(S.view.builtin_glossary);
  const filterInput = h("input", { type: "search", placeholder: "筛选", value: builtinFilter });
  const builtinBody = h("tbody");
  const fillBuiltin = () => {
    const f = builtinFilter.toLowerCase();
    builtinBody.replaceChildren(
      ...builtin
        .filter(([k, v]) => !f || k.toLowerCase().includes(f) || v.includes(builtinFilter))
        .map(([k, v]) => h("tr", {}, h("td", {}, k), h("td", {}, v))),
    );
  };
  filterInput.addEventListener("input", () => {
    builtinFilter = filterInput.value;
    fillBuiltin();
  });
  fillBuiltin();

  root.replaceChildren(
    pageHead("术语表", "按整条原文精确匹配（忽略首尾空格），命中的既不查译文库也不请求翻译引擎。优先级：自定义术语 > 内置术语 > 译文库 > 翻译引擎。"),
    h(
      "div",
      { class: "card flush" },
      h(
        "div",
        { class: "toolbar" },
        h("div", { class: "grow" }, h("h3", {}, "自定义术语"), h("div", { class: "desc" }, `${rows.length} 条 · 适合修正单个词的错译，例如 New → 新建`)),
        h("button", { class: "btn ghost", onclick: () => (rows.push(["", ""]), (dirty = true), rerender()) }, icon("plus"), "添加一条"),
        h("button", { class: "btn primary", onclick: save }, "保存"),
      ),
      h("table", {}, h("thead", {}, h("tr", {}, h("th", {}, "原文"), h("th", {}, "译文"), h("th", { style: "width:52px" }, ""))), tbody),
    ),
    h(
      "div",
      { class: "card" },
      h(
        "div",
        { class: "row spread", style: "gap:16px" },
        h(
          "div",
          { class: "grow" },
          h("h3", {}, "内置术语表"),
          h("div", { class: "desc" }, `${builtin.length} 条常见界面词的固定译法（${cfg().target_lang}）。自定义术语里写同样的原文即可覆盖。`),
        ),
        switchEl(cfg().builtin_glossary, (v) => saveConfig((c) => (c.builtin_glossary = v), v ? "已启用内置术语表" : "已停用内置术语表")),
      ),
      builtin.length > 0 &&
        h(
          "details",
          { class: "pages", style: "margin-top:14px" },
          h("summary", {}, "查看内置术语"),
          h("div", { class: "search", style: "margin:10px 0 8px" }, icon("search"), filterInput),
          h("div", { class: "inner-table" }, h("table", {}, h("thead", {}, h("tr", {}, h("th", {}, "原文"), h("th", {}, "译文"))), builtinBody)),
        ),
    ),
  );
}
