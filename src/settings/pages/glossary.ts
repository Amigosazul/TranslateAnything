// 术语表：自定义术语增删改、内置术语表开关与查看
import { S, cfg, saveConfig } from "../state";
import { fmtNum, t } from "../../i18n";
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
    const src = h("input", { type: "text", value: row[0], placeholder: t("glossary.sourcePh"), "data-k": `gl-s-${i}` });
    const dst = h("input", { type: "text", value: row[1], placeholder: t("glossary.targetPh"), "data-k": `gl-t-${i}` });
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
          h("button", { class: "icon-btn", title: t("common.delete"), "aria-label": t("common.delete"), onclick: () => (rows.splice(i, 1), (dirty = true), rerender()) }, icon("trash")),
        ),
      ),
    );
  });
  if (rows.length === 0) tbody.append(h("tr", {}, h("td", { colspan: 3 }, emptyState(t("glossary.empty")))));

  const save = async () => {
    const map: Record<string, string> = {};
    for (const [k, v] of rows) {
      if (!k.trim()) continue;
      if (!v.trim()) {
        toast(t("glossary.toast.emptyTarget", { term: k }), true);
        return;
      }
      map[k.trim()] = v;
    }
    dirty = false;
    draft = null;
    const ok = await saveConfig((c) => (c.glossary = map), t("glossary.toast.saved"));
    if (!ok) {
      dirty = true;
      draft = rows;
      rerender();
    }
  };

  const builtin = Object.entries(S.view.builtin_glossary);
  const filterInput = h("input", { type: "search", placeholder: t("glossary.filter"), value: builtinFilter });
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
    pageHead(t("glossary.title"), t("glossary.lead")),
    h(
      "div",
      { class: "card flush" },
      h(
        "div",
        { class: "toolbar" },
        h("div", { class: "grow" }, h("h3", {}, t("glossary.custom.title")), h("div", { class: "desc" }, t("glossary.custom.desc", { n: fmtNum(rows.length) }))),
        h("button", { class: "btn ghost", onclick: () => (rows.push(["", ""]), (dirty = true), rerender()) }, icon("plus"), t("glossary.addRow")),
        h("button", { class: "btn primary", onclick: save }, t("common.save")),
      ),
      h("table", {}, h("thead", {}, h("tr", {}, h("th", {}, t("glossary.col.source")), h("th", {}, t("glossary.col.target")), h("th", { style: "width:52px" }, ""))), tbody),
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
          h("h3", {}, t("glossary.builtin.title")),
          h("div", { class: "desc" }, t("glossary.builtin.desc", { n: fmtNum(builtin.length), lang: cfg().target_lang })),
        ),
        switchEl(cfg().builtin_glossary, (v) => saveConfig((c) => (c.builtin_glossary = v), v ? t("glossary.toast.builtinOn") : t("glossary.toast.builtinOff"))),
      ),
      builtin.length > 0 &&
        h(
          "details",
          { class: "pages", style: "margin-top:14px" },
          h("summary", {}, t("glossary.builtin.view")),
          h("div", { class: "search", style: "margin:10px 0 8px" }, icon("search"), filterInput),
          h("div", { class: "inner-table" }, h("table", {}, h("thead", {}, h("tr", {}, h("th", {}, t("glossary.col.source")), h("th", {}, t("glossary.col.target")))), builtinBody)),
        ),
    ),
  );
}
