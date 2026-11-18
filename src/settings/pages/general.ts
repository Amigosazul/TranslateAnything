// 常规：目标语言、译文库开关、文件位置（显示大小，译文库可搬家）
import { invoke } from "@tauri-apps/api/core";
import { LANGS, S, cfg, loadStats, loadView, refresh, saveConfig } from "../state";
import { confirmDialog, dropdown, errText, fileSize, h, icon, pageHead, section, switchEl, toast } from "../ui";

let moving = false;

async function reveal(which: "config" | "store") {
  try {
    await invoke("reveal_file", { which });
  } catch (e) {
    toast(errText(e), true);
  }
}

async function relocate(reset: boolean) {
  if (reset) {
    const ok = await confirmDialog(
      "把译文库搬回默认位置？",
      h("p", { class: "muted" }, "译文库文件会移动到数据目录，内容不变。"),
      "搬回去",
    );
    if (!ok) return;
  }
  moving = true;
  refresh();
  try {
    const path = reset ? await invoke<string>("store_reset_location") : await invoke<string | null>("store_pick_and_relocate");
    if (path) toast("译文库已移动到新位置");
  } catch (e) {
    toast(errText(e), true);
  } finally {
    moving = false;
    await loadView();
    await loadStats();
    refresh();
  }
}

export function renderGeneral(root: HTMLElement) {
  const c = cfg();
  const v = S.view;
  const row = (label: string, desc: string, control: HTMLElement, auto = false) =>
    h(
      "div",
      { class: "setting-row" },
      h("div", { class: "label" }, h("div", {}, label), desc && h("div", { class: "desc" }, desc)),
      h("div", { class: `control${auto ? " auto" : ""}` }, control),
    );

  const fileRow = (label: string, path: string, size: number, note: string, ...actions: HTMLElement[]) =>
    h(
      "div",
      { class: "file-row" },
      h("span", { class: "file-icon" }, icon(label === "译文库" ? "db" : "sliders")),
      h(
        "div",
        { class: "grow" },
        h("div", { class: "row", style: "gap:8px" }, h("span", { class: "file-name" }, label), h("span", { class: "tag" }, fileSize(size))),
        h("div", { class: "path clip", title: path }, path),
        note && h("div", { class: "desc" }, note),
      ),
      ...actions,
    );

  root.replaceChildren(
    pageHead("常规", "悬浮窗模式和沉浸模式共用这些设置。"),
    h(
      "div",
      { class: "card flush" },
      row(
        "目标语言",
        "译文统一翻译成这种语言。",
        dropdown(
          LANGS.map(([val, n]) => ({ value: val, label: n, hint: val })),
          c.target_lang,
          (val) => saveConfig((x) => (x.target_lang = val), "目标语言已更改"),
          { width: "100%" },
        ),
      ),
      row(
        "使用译文库",
        "翻译过的文字存到本地，相同原文不再请求翻译引擎。",
        switchEl(c.persist_cache, (on) => saveConfig((x) => (x.persist_cache = on), on ? "已开启译文库" : "已关闭译文库")),
        true,
      ),
    ),
    section("文件位置"),
    h(
      "div",
      { class: "card flush" },
      fileRow(
        "配置文件",
        v.config_path,
        v.config_size,
        "固定放在用户目录，随译启动时从这里读取。",
        h("button", { class: "btn ghost sm", onclick: () => reveal("config") }, icon("folder"), "打开位置"),
      ),
      fileRow(
        "译文库",
        v.db_path,
        v.db_size,
        v.db_custom ? "已放在自定义位置。" : "",
        h("button", { class: "btn sm", disabled: moving, onclick: () => relocate(false) }, moving ? "移动中…" : "更改位置…"),
        v.db_custom ? h("button", { class: "btn ghost sm", disabled: moving, onclick: () => relocate(true) }, "恢复默认") : h("span"),
        h("button", { class: "btn ghost sm", onclick: () => reveal("store") }, icon("folder"), "打开位置"),
      ),
    ),
    h("p", { class: "desc", style: "margin:8px 2px 0" }, "翻译引擎和大模型请在“翻译引擎”页面设置。"),
  );
}
