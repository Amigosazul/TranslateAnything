// 译文库：搜索、改译、删除、清空
import { invoke } from "@tauri-apps/api/core";
import { cfg, loadStats } from "../state";
import { confirmDialog, emptyState, errText, fmt, h, icon, pageHead, relTime, toast } from "../ui";

interface Entry {
  lang: string;
  source: string;
  target: string;
  engine: string;
  manual: boolean;
  hits: number;
  used_at: number;
}

const PAGE_SIZE = 50;
const lib = { keyword: "", manualOnly: false, offset: 0, total: 0, rows: [] as Entry[] };
let built = false;
let searchTimer = 0;

/** 译文库页只搭一次骨架（保住搜索框里正在输入的内容），之后只刷新表格 */
export function buildLibrary(root: HTMLElement) {
  if (built) return;
  const search = h("input", { type: "search", placeholder: "搜索原文或译文", value: lib.keyword });
  search.addEventListener("input", () => {
    clearTimeout(searchTimer);
    searchTimer = window.setTimeout(() => {
      lib.keyword = search.value;
      lib.offset = 0;
      queryLibrary();
    }, 300);
  });
  const manual = h("input", { type: "checkbox", checked: lib.manualOnly });
  manual.addEventListener("change", () => {
    lib.manualOnly = manual.checked;
    lib.offset = 0;
    queryLibrary();
  });
  root.replaceChildren(
    pageHead("译文库", "翻译过的文字都存在这里，下次遇到相同原文直接使用、不再请求翻译引擎。改过的译文标记为手工译文，自动翻译不会覆盖；改动会立即推送到已接管的页面。"),
    h(
      "div",
      { class: "card flush" },
      h(
        "div",
        { class: "toolbar" },
        h("div", { class: "grow search" }, icon("search"), search),
        h("label", { class: "check" }, manual, "只看手工译文"),
        h("button", { class: "btn ghost danger", onclick: clearLibrary }, icon("trash"), "清空…"),
      ),
      h("div", { id: "lib-table" }),
    ),
  );
  built = true;
}

export async function queryLibrary() {
  try {
    const page = await invoke<{ total: number; rows: Entry[] }>("store_query", {
      keyword: lib.keyword,
      manualOnly: lib.manualOnly,
      offset: lib.offset,
      limit: PAGE_SIZE,
    });
    lib.total = page.total;
    lib.rows = page.rows;
    renderTable();
  } catch (e) {
    toast(errText(e), true);
  }
}

function renderTable() {
  const box = document.getElementById("lib-table");
  if (!box) return;
  if (lib.rows.length === 0) {
    box.replaceChildren(emptyState(lib.keyword || lib.manualOnly ? "没有符合条件的译文" : "译文库还是空的", lib.keyword ? "search" : "box"));
    return;
  }
  const tbody = h("tbody");
  for (const r of lib.rows) {
    const input = h("textarea", { class: "cell", rows: 1, title: "修改后按回车或移开焦点保存" });
    input.value = r.target;
    const commit = async () => {
      const v = input.value.trim();
      if (!v || v === r.target) {
        input.value = r.target;
        return;
      }
      try {
        await invoke("store_set", { source: r.source, target: v });
        r.target = v;
        r.manual = true;
        r.engine = "手工";
        toast("已改译，已接管的页面会立即更新");
        renderTable();
        loadStats();
      } catch (e) {
        toast(errText(e), true);
      }
    };
    input.addEventListener("change", commit);
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        input.blur();
      }
    });
    tbody.append(
      h(
        "tr",
        { class: r.manual ? "manual" : "" },
        h("td", { class: "src-text" }, r.source),
        h("td", {}, input),
        h("td", {}, h("span", { class: `tag${r.manual ? " accent" : ""}` }, r.manual ? "手工" : r.engine)),
        h("td", { class: "small muted num" }, String(r.hits)),
        h("td", { class: "small muted" }, relTime(r.used_at)),
        h(
          "td",
          {},
          h(
            "button",
            {
              class: "icon-btn",
              title: "删除（已接管的页面会重新翻译这段文字）",
              "aria-label": "删除",
              onclick: async () => {
                try {
                  await invoke("store_delete", { sources: [r.source] });
                  toast("已删除，页面会重新翻译这段文字");
                  queryLibrary();
                  loadStats();
                } catch (e) {
                  toast(errText(e), true);
                }
              },
            },
            icon("trash"),
          ),
        ),
      ),
    );
  }
  const pages = Math.max(1, Math.ceil(lib.total / PAGE_SIZE));
  const cur = Math.floor(lib.offset / PAGE_SIZE) + 1;
  box.replaceChildren(
    h(
      "table",
      {},
      h(
        "thead",
        {},
        h(
          "tr",
          {},
          h("th", { style: "width:36%" }, "原文"),
          h("th", {}, "译文"),
          h("th", { style: "width:92px" }, "来源"),
          h("th", { style: "width:60px" }, "命中"),
          h("th", { style: "width:92px" }, "最近使用"),
          h("th", { style: "width:52px" }, ""),
        ),
      ),
      tbody,
    ),
    h(
      "div",
      { class: "row spread pager" },
      h("span", { class: "muted small" }, `共 ${fmt(lib.total)} 条（目标语言 ${cfg().target_lang}）`),
      h(
        "div",
        { class: "row" },
        h("button", { class: "btn sm", disabled: cur <= 1, onclick: () => ((lib.offset -= PAGE_SIZE), queryLibrary()) }, icon("chevL"), "上一页"),
        h("span", { class: "small muted num" }, `${cur} / ${pages}`),
        h("button", { class: "btn sm", disabled: cur >= pages, onclick: () => ((lib.offset += PAGE_SIZE), queryLibrary()) }, "下一页", icon("chevR")),
      ),
    ),
  );
}

async function clearLibrary() {
  const keep = h("input", { type: "checkbox", checked: true });
  const ok = await confirmDialog(
    "清空译文库？",
    h(
      "div",
      {},
      h("p", { class: "muted", style: "margin-top:0" }, `将删除目标语言 ${cfg().target_lang} 的译文，之后遇到这些文字会重新请求翻译引擎。已显示在页面上的译文要等页面刷新后才会更新。`),
      h("label", { class: "check" }, keep, "保留手工译文"),
    ),
    "清空",
    true,
  );
  if (!ok) return;
  try {
    const n = await invoke<number>("store_clear", { keepManual: keep.checked });
    toast(`已删除 ${n} 条`);
    lib.offset = 0;
    queryLibrary();
    loadStats();
  } catch (e) {
    toast(errText(e), true);
  }
}
