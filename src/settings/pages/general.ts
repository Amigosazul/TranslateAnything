// 常规：界面语言、目标语言、网络代理、译文库开关、文件位置（显示大小，译文库可搬家）
import { invoke } from "@tauri-apps/api/core";
import { S, cfg, langs, loadStats, loadView, refresh, saveConfig } from "../state";
import { confirmDialog, dropdown, errText, fileSize, h, icon, pageHead, section, switchEl, toast, type IconName } from "../ui";
import { t, uiLocales } from "../../i18n";
import { proxyField } from "./engines";

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
      t("general.resetConfirmTitle"),
      h("p", { class: "muted" }, t("general.resetConfirmBody")),
      t("general.resetConfirmOk"),
    );
    if (!ok) return;
  }
  moving = true;
  refresh();
  try {
    const path = reset ? await invoke<string>("store_reset_location") : await invoke<string | null>("store_pick_and_relocate");
    if (path) toast(t("general.storeMoved"));
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

  const fileRow = (ic: IconName, label: string, path: string, size: number, note: string, ...actions: HTMLElement[]) =>
    h(
      "div",
      { class: "file-row" },
      h("span", { class: "file-icon" }, icon(ic)),
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
    pageHead(t("general.lead")),
    h(
      "div",
      { class: "card flush" },
      row(
        t("general.uiLang"),
        t("general.uiLangDesc"),
        dropdown(
          [{ value: "", label: t("general.uiLangSystem") }, ...uiLocales().map(([tag, name]) => ({ value: tag, label: name, hint: tag }))],
          c.ui_lang ?? "",
          // 保存后后端会发 locale-changed，窗口随之重新载入，所以不提示
          (val) => saveConfig((x) => (x.ui_lang = val), ""),
          { width: "100%" },
        ),
      ),
      row(
        t("general.targetLang"),
        t("general.targetLangDesc"),
        dropdown(
          langs().map(([val, n]) => ({ value: val, label: n, hint: val })),
          c.target_lang,
          (val) => saveConfig((x) => (x.target_lang = val), t("general.targetLangChanged")),
          { width: "100%" },
        ),
      ),
      row(
        t("general.proxy"),
        t("general.proxyDesc"),
        proxyField(c.proxy ?? "", null, (val) => saveConfig((x) => (x.proxy = val))),
      ),
      row(
        t("general.useStore"),
        t("general.useStoreDesc"),
        switchEl(c.persist_cache, (on) => saveConfig((x) => (x.persist_cache = on), on ? t("general.storeOn") : t("general.storeOff"))),
        true,
      ),
    ),
    section(t("general.files")),
    h(
      "div",
      { class: "card flush" },
      fileRow(
        "sliders",
        t("general.configFile"),
        v.config_path,
        v.config_size,
        t("general.configFileDesc", { app: t("app.name") }),
        h("button", { class: "btn ghost sm", onclick: () => reveal("config") }, icon("folder"), t("general.openLocation")),
      ),
      fileRow(
        "db",
        t("nav.library"),
        v.db_path,
        v.db_size,
        v.db_custom ? t("general.storeCustom") : "",
        h("button", { class: "btn sm", disabled: moving, onclick: () => relocate(false) }, moving ? t("general.moving") : t("general.changeLocation")),
        v.db_custom ? h("button", { class: "btn ghost sm", disabled: moving, onclick: () => relocate(true) }, t("common.restoreDefault")) : h("span"),
        h("button", { class: "btn ghost sm", onclick: () => reveal("store") }, icon("folder"), t("general.openLocation")),
      ),
    ),
    h("p", { class: "desc", style: "margin:8px 2px 0" }, t("general.enginesHint", { page: t("nav.engines") })),
  );
}
