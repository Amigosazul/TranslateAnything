// 沉浸模式：总开关、快捷键、应用列表；添加 / 编辑 / 识别应用都在对话框里完成
import { invoke } from "@tauri-apps/api/core";
import { S, cfg, loadView, refresh, saveConfig, type ImmersiveTarget, type StateName, type TargetStatus } from "../state";
import {
  avatar,
  confirmDialog,
  emptyState,
  errText,
  field,
  fmt,
  h,
  icon,
  keys,
  openDialog,
  pageHead,
  section,
  switchEl,
  toast,
  type DialogHandle,
} from "../ui";

interface DetectedApp {
  name: string;
  exe: string;
  port: number | null;
  configured: boolean;
}

export const STATE_TEXT: Record<StateName, [string, string]> = {
  off: ["总开关已关", ""],
  disabled: ["已停用", ""],
  not_running: ["未运行", ""],
  no_port: ["未以沉浸模式启动", "warn"],
  conflict: ["端口冲突", "bad"],
  waiting: ["等待退出", "warn"],
  attached: ["已接管", "ok"],
};

const launching = new Set<number>();

const statusOf = (port: number): TargetStatus | undefined => S.statuses.find((s) => s.port === port);
const attachedList = () => S.statuses.filter((s) => s.state === "attached");

export function badgeOf(port: number): HTMLElement {
  const s = statusOf(port);
  const [label, tone] = s ? STATE_TEXT[s.state] : ["检测中", ""];
  return h("span", { class: `badge ${tone}` }, label);
}

/** 全部显示原文 / 译文 */
export function toggleAllButton(cls = "btn"): HTMLElement {
  const attached = attachedList();
  const anyShown = attached.some((s) => s.show);
  return h(
    "button",
    { class: cls, disabled: attached.length === 0, onclick: () => invoke("immersive_set_show", { port: null, show: !anyShown }) },
    icon("swap"),
    anyShown ? "全部显示原文" : "全部显示译文",
  );
}

export function setMasterSwitch(v: boolean) {
  return saveConfig((c) => (c.immersive.enabled = v), v ? "沉浸模式已开启" : "沉浸模式已关闭，页面已恢复原文");
}

async function setTargetEnabled(t: ImmersiveTarget, v: boolean) {
  try {
    await invoke("immersive_set_enabled", { port: t.port, enabled: v });
    await loadView();
    refresh();
    toast(v ? `已启用「${t.name}」` : `已停用「${t.name}」，页面已恢复原文`);
  } catch (e) {
    toast(errText(e), true);
  }
}

// ───────────────────────── 页面 ─────────────────────────

export function renderImmersive(root: HTMLElement) {
  const im = cfg().immersive;
  const attached = attachedList();
  const translated = attached.reduce((n, s) => n + s.translated, 0);
  const pages = attached.reduce((n, s) => n + s.pages.length, 0);

  const hero = h(
    "div",
    { class: `card hero${im.enabled ? "" : " off"}` },
    h(
      "div",
      { class: "row", style: "gap:16px" },
      h("div", { class: "hero-icon" }, icon("apps")),
      h(
        "div",
        { class: "grow" },
        h("h3", {}, im.enabled ? "沉浸模式已开启" : "沉浸模式已关闭"),
        h("div", { class: "desc" }, "切换原文 / 译文：", keys(im.hotkey)),
      ),
      toggleAllButton(),
      switchEl(im.enabled, setMasterSwitch),
    ),
    h(
      "div",
      { class: "metrics" },
      h("div", { class: "metric" }, h("b", {}, `${attached.length} / ${im.targets.length}`), h("span", {}, "已接管应用")),
      h("div", { class: "metric" }, h("b", {}, String(pages)), h("span", {}, "已接管页面")),
      h("div", { class: "metric" }, h("b", {}, fmt(translated)), h("span", {}, "本次回填译文")),
    ),
  );

  const hotkey = h("input", { type: "text", value: im.hotkey, "data-k": "im-hotkey" });
  hotkey.addEventListener("change", () => {
    const v = hotkey.value.trim();
    if (v && v !== im.hotkey) saveConfig((c) => (c.immersive.hotkey = v), "已保存，重启随译后生效");
  });

  const list = h("div", { class: "list" });
  for (const t of im.targets) list.append(targetCard(t));
  if (im.targets.length === 0)
    list.append(
      h(
        "div",
        { class: "card" },
        emptyState("还没有添加应用", "apps"),
        h("div", { class: "row", style: "justify-content:center; margin-top:-16px; padding-bottom:12px" }, h("button", { class: "btn primary", onclick: detectDialog }, icon("scan"), "识别正在运行的应用")),
      ),
    );

  root.replaceChildren(
    pageHead("沉浸模式", "不改软件本体，直接在应用内部把界面文字原地换成译文。只接管列在这里、并以沉浸模式启动的应用。"),
    hero,
    h(
      "div",
      { class: "card flush" },
      h(
        "div",
        { class: "setting-row" },
        h("div", { class: "label" }, h("div", {}, "原文 / 译文切换快捷键"), h("div", { class: "desc" }, "在已接管的应用里随时切换显示，重启随译后生效。")),
        h("div", { class: "control" }, hotkey),
      ),
    ),
    section(
      "应用",
      h("span", { class: "count" }, String(im.targets.length)),
      h("span", { class: "spacer" }),
      h("button", { class: "btn sm", onclick: detectDialog }, icon("scan"), "识别正在运行的应用"),
      h("button", { class: "btn ghost sm", onclick: () => appDialog(blankTarget(), null) }, icon("plus"), "手动添加"),
    ),
    list,
    h("p", { class: "desc", style: "margin:8px 2px 0" }, "目前支持 Electron / Chromium 内核的应用。端口背后的程序与配置不符时会拒绝接管。"),
  );
}

function targetCard(t: ImmersiveTarget): HTMLElement {
  const s = statusOf(t.port);
  const showLabel = s && s.state === "attached" ? (s.show ? "显示译文中" : "显示原文中") : "";

  const actions = h("div", { class: "card-foot" });
  if (s && (s.state === "not_running" || s.state === "no_port")) {
    actions.append(
      h(
        "button",
        { class: "btn primary sm", disabled: launching.has(t.port), onclick: () => launch(t) },
        icon("play"),
        launching.has(t.port) ? "等待中…" : s.state === "no_port" ? "以沉浸模式重启" : "以沉浸模式启动",
      ),
    );
  }
  if (s && s.state === "attached") {
    actions.append(
      h(
        "button",
        { class: "btn sm", onclick: () => invoke("immersive_set_show", { port: t.port, show: !s.show }) },
        icon("swap"),
        s.show ? "显示原文" : "显示译文",
      ),
    );
  }
  actions.append(
    h("div", { class: "spacer" }),
    h("button", { class: "btn ghost sm", onclick: () => appDialog({ ...t }, t.port) }, icon("edit"), "编辑"),
    h("button", { class: "btn ghost danger sm", onclick: () => removeTarget(t) }, icon("trash"), "删除"),
  );

  const meta = h("div", { class: "meta" }, h("span", { class: "chip" }, `端口 ${t.port}`));
  if (s && s.state === "attached") {
    meta.append(
      h("span", {}, h("b", {}, String(s.pages.length)), " 个页面"),
      h("span", {}, "本次回填 ", h("b", {}, fmt(s.translated)), " 条"),
      h("span", {}, "请求引擎 ", h("b", {}, String(s.engine_batches)), " 次"),
    );
  }

  const noteTone = s && (s.state === "conflict" ? "bad" : s.state === "no_port" || s.state === "waiting" ? "warn" : "");
  return h(
    "div",
    { class: `card app-card${t.enabled && cfg().immersive.enabled ? "" : " dim"}` },
    h(
      "div",
      { class: "row", style: "gap:14px; align-items:flex-start" },
      avatar(t.name),
      h(
        "div",
        { class: "grow" },
        h("div", { class: "app-title" }, h("h3", {}, t.name), badgeOf(t.port), showLabel && h("span", { class: "muted small" }, showLabel)),
        h("div", { class: "path clip", style: "margin-top:2px", title: t.exe }, t.exe || "未填写程序路径，无法做身份校验"),
      ),
      switchEl(t.enabled, (v) => setTargetEnabled(t, v), !cfg().immersive.enabled),
    ),
    meta,
    s && s.message && s.state !== "attached" && h("div", { class: `note ${noteTone}`, style: "margin-top:12px" }, s.message),
    s &&
      s.pages.length > 0 &&
      h(
        "details",
        { class: "pages", style: "margin-top:12px" },
        h("summary", {}, `已接管的页面（${s.pages.length}）`),
        h("ul", {}, ...s.pages.map((p) => h("li", {}, p.title && h("b", {}, p.title), p.url))),
      ),
    actions,
  );
}

async function removeTarget(t: ImmersiveTarget) {
  const ok = await confirmDialog(
    `删除「${t.name}」？`,
    h("p", { class: "muted" }, "删除后不再接管这个应用，已翻译的页面会恢复原文。应用本身不受影响。"),
    "删除",
    true,
  );
  if (ok) await saveConfig((c) => (c.immersive.targets = c.immersive.targets.filter((x) => x.port !== t.port)), `已删除「${t.name}」`);
}

async function launch(t: ImmersiveTarget) {
  launching.add(t.port);
  refresh();
  try {
    toast(await invoke<string>("immersive_launch", { port: t.port }));
  } catch (e) {
    toast(errText(e), true);
  } finally {
    launching.delete(t.port);
    await loadView();
    refresh();
  }
}

// ───────────────────────── 对话框 ─────────────────────────

function nextFreePort(): number {
  const used = new Set(cfg().immersive.targets.map((t) => t.port));
  let p = 39222;
  while (used.has(p)) p++;
  return p;
}

const blankTarget = (): ImmersiveTarget => ({ name: "", enabled: true, exe: "", args: "", port: nextFreePort(), url_prefixes: [], skip_selectors: [] });

/** 应用表单：返回表单元素和读取结果的函数 */
function appForm(d: ImmersiveTarget) {
  const name = h("input", { type: "text", value: d.name, placeholder: "例如 reclaude" });
  const exe = h("input", { type: "text", value: d.exe, placeholder: "例如 C:\\Program Files\\App\\App.exe", spellcheck: false });
  const port = h("input", { type: "number", value: String(d.port), min: "1024", max: "65535" });
  const args = h("textarea", { rows: 2, placeholder: "留空即可：应用运行时点“以沉浸模式启动”会自动读取" });
  args.value = d.args;
  const prefixes = h("textarea", { rows: 2, placeholder: "每行一个，例如 https://claude.ai/" });
  prefixes.value = d.url_prefixes.join("\n");
  const skips = h("textarea", { rows: 2, placeholder: "每行一个 CSS 选择器，例如 .chat-message" });
  skips.value = d.skip_selectors.join("\n");
  const lines = (v: string) => v.split("\n").map((s) => s.trim()).filter(Boolean);
  const form = h(
    "div",
    { class: "form-grid" },
    field("名称", name),
    field("调试端口", port),
    field("程序路径（用于身份校验和启动）", exe, "wide"),
    h(
      "details",
      { class: "pages wide" },
      h("summary", {}, "高级选项"),
      h(
        "div",
        { class: "form-grid", style: "margin-top:12px" },
        field("启动参数（不含调试端口）", args, "wide"),
        field("只处理这些地址开头的页面（留空 = 全部）", prefixes, "wide"),
        field("不翻译的区域（CSS 选择器）", skips, "wide"),
      ),
    ),
  );
  const read = (): ImmersiveTarget => ({
    ...d,
    name: name.value.trim(),
    exe: exe.value.trim(),
    port: Number(port.value),
    args: args.value.trim(),
    url_prefixes: lines(prefixes.value),
    skip_selectors: lines(skips.value),
  });
  return { form, read };
}

/** 添加（origPort 为 null）或编辑应用的对话框；可传入已打开的对话框直接切换内容 */
function appDialog(d: ImmersiveTarget, origPort: number | null, existing?: DialogHandle) {
  const isNew = origPort === null;
  const { form, read } = appForm(d);
  let dlg: DialogHandle;
  const saveBtn = h("button", { class: "btn primary" }, isNew ? "添加" : "保存");
  saveBtn.onclick = async () => {
    const next = read();
    saveBtn.disabled = true;
    const ok = await saveConfig(
      (c) => {
        if (isNew) c.immersive.targets.push(next);
        else {
          const i = c.immersive.targets.findIndex((t) => t.port === origPort);
          if (i >= 0) c.immersive.targets[i] = next;
        }
      },
      isNew ? `已添加「${next.name}」` : "已保存",
    );
    saveBtn.disabled = false;
    if (ok) dlg.close();
  };
  const title = isNew ? "添加应用" : `编辑「${d.name}」`;
  const actions = [h("button", { class: "btn", onclick: () => dlg.close() }, "取消"), saveBtn];
  if (existing) {
    dlg = existing;
    dlg.set(title, form, actions);
    requestAnimationFrame(() => form.querySelector<HTMLInputElement>("input")?.focus());
  } else dlg = openDialog(title, form, actions, { wide: true });
}

/** 识别正在运行的 Electron 应用，选一个后在同一个对话框里补全信息 */
function detectDialog() {
  const body = h("div", { class: "detect" }, h("div", { class: "loading" }, h("span", { class: "spinner" }), "正在查找正在运行的应用…"));
  const manual = h("button", { class: "btn ghost", onclick: () => appDialog(blankTarget(), null, dlg) }, icon("plus"), "手动添加");
  const dlg = openDialog("识别正在运行的应用", body, [manual, h("span", { class: "spacer" }), h("button", { class: "btn", onclick: () => dlg.close() }, "关闭")], { wide: true });

  invoke<DetectedApp[]>("immersive_detect")
    .then((apps) => {
      if (apps.length === 0) {
        body.replaceChildren(emptyState("没有找到正在运行的 Electron 应用。先把要翻译的应用打开，或者手动添加。", "scan"));
        return;
      }
      body.replaceChildren(
        h("p", { class: "desc", style: "margin:0 0 10px" }, `找到 ${apps.length} 个应用，选择要接管的：`),
        h(
          "div",
          { class: "pick-list" },
          ...apps.map((a) =>
            h(
              "div",
              { class: `pick-item${a.configured ? " done" : ""}` },
              avatar(a.name),
              h(
                "div",
                { class: "grow" },
                h("div", { class: "row", style: "gap:8px" }, h("b", {}, a.name), a.port ? h("span", { class: "tag" }, `调试端口 ${a.port}`) : null),
                h("div", { class: "path clip", title: a.exe }, a.exe),
              ),
              a.configured
                ? h("span", { class: "muted small" }, "已在列表中")
                : h(
                    "button",
                    { class: "btn sm", onclick: () => appDialog({ ...blankTarget(), name: a.name, exe: a.exe, port: a.port ?? nextFreePort() }, null, dlg) },
                    "添加",
                  ),
            ),
          ),
        ),
      );
    })
    .catch((e) => body.replaceChildren(h("div", { class: "note bad" }, errText(e))));
}
