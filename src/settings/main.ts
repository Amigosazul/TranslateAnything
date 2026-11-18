// 设置窗口：沉浸模式应用管理、译文库、术语表、常规设置
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// ───────────────────────── 类型 ─────────────────────────

interface ImmersiveTarget {
  name: string;
  enabled: boolean;
  exe: string;
  args: string;
  port: number;
  url_prefixes: string[];
  skip_selectors: string[];
}

interface Config {
  hotkey: string;
  target_lang: string;
  persist_cache: boolean;
  builtin_glossary: boolean;
  glossary: Record<string, string>;
  immersive: { enabled: boolean; hotkey: string; targets: ImmersiveTarget[] };
  [key: string]: unknown;
}

interface SettingsView {
  config: Config;
  config_path: string;
  db_path: string;
  builtin_glossary: Record<string, string>;
}

type StateName = "off" | "disabled" | "not_running" | "no_port" | "conflict" | "waiting" | "attached";

interface TargetStatus {
  port: number;
  name: string;
  enabled: boolean;
  state: StateName;
  message: string;
  pages: { id: string; url: string; title: string }[];
  show: boolean;
  translated: number;
  engine_batches: number;
}

interface DetectedApp {
  name: string;
  exe: string;
  port: number | null;
  configured: boolean;
}

interface Entry {
  lang: string;
  source: string;
  target: string;
  engine: string;
  manual: boolean;
  hits: number;
  used_at: number;
}

// ───────────────────────── 小工具 ─────────────────────────

type Child = Node | string | null | undefined | false;

/** 创建元素；文字一律走 textContent，页面标题等外部内容不会被当成 HTML */
function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, unknown> = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === null || v === false) continue;
    if (k.startsWith("on") && typeof v === "function") {
      el.addEventListener(k.slice(2), v as EventListener);
    } else if (k === "class") {
      el.className = String(v);
    } else if (k in el && typeof v !== "string") {
      (el as unknown as Record<string, unknown>)[k] = v;
    } else {
      el.setAttribute(k, v === true ? "" : String(v));
    }
  }
  for (const c of children) if (c !== null && c !== undefined && c !== false) el.append(c);
  return el;
}

function switchEl(checked: boolean, onChange: (v: boolean) => void, disabled = false): HTMLLabelElement {
  const input = h("input", { type: "checkbox", checked, disabled });
  input.addEventListener("change", () => onChange(input.checked));
  return h("label", { class: "switch" }, input, h("i"));
}

function field(label: string, control: HTMLElement, cls = ""): HTMLLabelElement {
  return h("label", { class: `field ${cls}` }, h("span", {}, label), control);
}

let toastTimer = 0;
function toast(msg: string, bad = false) {
  const el = document.getElementById("toast")!;
  el.textContent = msg;
  el.className = bad ? "bad" : "";
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (el.hidden = true), bad ? 6000 : 2500);
}

/** 确认对话框；返回用户是否确认 */
function confirmDialog(title: string, body: Child, okText: string, danger = false): Promise<boolean> {
  const modal = document.getElementById("modal")!;
  return new Promise((resolve) => {
    const close = (v: boolean) => {
      modal.hidden = true;
      modal.replaceChildren();
      resolve(v);
    };
    modal.replaceChildren(
      h(
        "div",
        { class: "dialog" },
        h("h3", {}, title),
        h("div", { style: "margin-top:10px" }, body),
        h(
          "div",
          { class: "actions" },
          h("button", { class: "btn", onclick: () => close(false) }, "取消"),
          h("button", { class: `btn ${danger ? "danger" : "primary"}`, onclick: () => close(true) }, okText),
        ),
      ),
    );
    modal.hidden = false;
  });
}

function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

function relTime(sec: number): string {
  const d = Date.now() / 1000 - sec;
  if (d < 60) return "刚刚";
  if (d < 3600) return `${Math.floor(d / 60)} 分钟前`;
  if (d < 86400) return `${Math.floor(d / 3600)} 小时前`;
  if (d < 86400 * 30) return `${Math.floor(d / 86400)} 天前`;
  return new Date(sec * 1000).toLocaleDateString("zh-CN");
}

const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v));

// ───────────────────────── 全局状态 ─────────────────────────

let view: SettingsView;
let statuses: TargetStatus[] = [];
/** 正在编辑的应用（按端口；新增未保存的用 -1） */
let editing: number | null = null;
let draft: ImmersiveTarget | null = null;
let detecting = false;
let detected: DetectedApp[] | null = null;
const launching = new Set<number>();

async function loadView() {
  view = await invoke<SettingsView>("settings_get");
}

/** 修改配置并保存；成功后重新读取并刷新界面 */
async function saveConfig(mutate: (c: Config) => void, okMsg = "已保存"): Promise<boolean> {
  const cfg = clone(view.config);
  mutate(cfg);
  try {
    await invoke("settings_save", { config: cfg });
    await loadView();
    renderAll();
    if (okMsg) toast(okMsg);
    return true;
  } catch (e) {
    toast(errText(e), true);
    return false;
  }
}

// ───────────────────────── 沉浸模式 ─────────────────────────

const STATE_TEXT: Record<StateName, [string, string]> = {
  off: ["总开关已关", ""],
  disabled: ["已停用", ""],
  not_running: ["未运行", ""],
  no_port: ["未以沉浸模式启动", "warn"],
  conflict: ["端口冲突", "bad"],
  waiting: ["等待退出", "warn"],
  attached: ["已接管", "ok"],
};

function statusOf(port: number): TargetStatus | undefined {
  return statuses.find((s) => s.port === port);
}

function nextFreePort(): number {
  const used = new Set(view.config.immersive.targets.map((t) => t.port));
  let p = 39222;
  while (used.has(p)) p++;
  return p;
}

function renderImmersive() {
  const root = document.getElementById("tab-immersive")!;
  const cfg = view.config.immersive;
  const attached = statuses.filter((s) => s.state === "attached");
  const anyShown = attached.some((s) => s.show);

  const head = h(
    "div",
    { class: "card" },
    h(
      "div",
      { class: "row spread" },
      h(
        "div",
        { class: "grow" },
        h("h3", {}, "沉浸模式"),
        h("div", { class: "muted small" }, `在应用内部原地改写界面文字。切换原文/译文快捷键：${cfg.hotkey}`),
      ),
      h(
        "button",
        {
          class: "btn",
          disabled: attached.length === 0,
          onclick: () => invoke("immersive_set_show", { port: null, show: !anyShown }),
        },
        anyShown ? "全部显示原文" : "全部显示译文",
      ),
      switchEl(cfg.enabled, (v) =>
        saveConfig((c) => (c.immersive.enabled = v), v ? "沉浸模式已开启" : "沉浸模式已关闭，页面已恢复原文"),
      ),
    ),
  );

  const list = h("div", {});
  for (const t of cfg.targets) list.append(targetCard(t));
  if (editing === -1 && draft) list.append(editCard(draft, true));
  if (cfg.targets.length === 0 && editing !== -1) {
    list.append(h("div", { class: "card empty" }, "还没有添加应用"));
  }

  root.replaceChildren(
    h("h2", {}, "沉浸模式"),
    h("p", { class: "lead" }, "管理要接管的应用。只有列在这里、并以沉浸模式（带调试端口）启动的应用才会被翻译；端口背后的程序与配置不符时拒绝接管。"),
    head,
    list,
    addPanel(),
  );
}

function targetCard(t: ImmersiveTarget): HTMLElement {
  if (editing === t.port && draft) return editCard(draft, false);
  const s = statusOf(t.port);
  const [label, tone] = s ? STATE_TEXT[s.state] : ["检测中", ""];
  const showLabel = s && s.state === "attached" ? (s.show ? "显示译文中" : "显示原文中") : "";

  const actions = h("div", { class: "row", style: "margin-top:12px; flex-wrap:wrap" });
  if (s && (s.state === "not_running" || s.state === "no_port")) {
    actions.append(
      h(
        "button",
        { class: "btn primary", disabled: launching.has(t.port), onclick: () => launch(t) },
        launching.has(t.port) ? "等待中…" : s.state === "no_port" ? "以沉浸模式重启" : "以沉浸模式启动",
      ),
    );
  }
  if (s && s.state === "attached") {
    actions.append(
      h(
        "button",
        { class: "btn", onclick: () => invoke("immersive_set_show", { port: t.port, show: !s.show }) },
        s.show ? "显示原文" : "显示译文",
      ),
    );
  }
  actions.append(
    h("button", { class: "btn", onclick: () => startEdit(t) }, "编辑"),
    h("button", { class: "btn danger", onclick: () => removeTarget(t) }, "删除"),
  );

  const stats =
    s && s.state === "attached"
      ? `端口 ${t.port} · ${s.pages.length} 个页面 · 本次回填 ${s.translated} 条（请求引擎 ${s.engine_batches} 次）`
      : `端口 ${t.port}`;

  const noteTone = s && (s.state === "conflict" ? "bad" : s.state === "no_port" || s.state === "waiting" ? "warn" : "");
  return h(
    "div",
    { class: "card" },
    h(
      "div",
      { class: "row spread" },
      h(
        "div",
        { class: "row grow" },
        h("h3", {}, t.name),
        h("span", { class: `badge ${tone}` }, label),
        showLabel && h("span", { class: "muted small" }, showLabel),
      ),
      switchEl(
        t.enabled,
        async (v) => {
          try {
            await invoke("immersive_set_enabled", { port: t.port, enabled: v });
            await loadView();
            renderAll();
            toast(v ? `已启用「${t.name}」` : `已停用「${t.name}」，页面已恢复原文`);
          } catch (e) {
            toast(errText(e), true);
          }
        },
        !view.config.immersive.enabled,
      ),
    ),
    h("div", { class: "path", style: "margin-top:6px" }, t.exe || "（未填写程序路径，无法做身份校验）"),
    h("div", { class: "muted small", style: "margin-top:4px" }, stats),
    s && s.message && s.state !== "attached" && h("div", { class: `note ${noteTone}`, style: "margin-top:8px" }, s.message),
    s &&
      s.pages.length > 0 &&
      h(
        "details",
        { class: "pages", style: "margin-top:8px" },
        h("summary", {}, `已接管的页面（${s.pages.length}）`),
        h("ul", {}, ...s.pages.map((p) => h("li", {}, p.title ? `${p.title} — ${p.url}` : p.url))),
      ),
    actions,
  );
}

function startEdit(t: ImmersiveTarget) {
  editing = t.port;
  draft = clone(t);
  renderImmersive();
}

function cancelEdit() {
  editing = null;
  draft = null;
  renderImmersive();
}

function editCard(d: ImmersiveTarget, isNew: boolean): HTMLElement {
  const name = h("input", { type: "text", value: d.name });
  const exe = h("input", { type: "text", value: d.exe, placeholder: "例如 C:\\Program Files\\App\\App.exe" });
  const port = h("input", { type: "number", value: String(d.port), min: "1024", max: "65535" });
  const args = h("textarea", { rows: 2, placeholder: "留空即可：应用运行时点“以沉浸模式启动”会自动读取" });
  args.value = d.args;
  const prefixes = h("textarea", { rows: 2, placeholder: "每行一个，例如 https://claude.ai/" });
  prefixes.value = d.url_prefixes.join("\n");
  const skips = h("textarea", { rows: 2, placeholder: "每行一个 CSS 选择器，例如 .chat-message" });
  skips.value = d.skip_selectors.join("\n");
  const lines = (v: string) => v.split("\n").map((s) => s.trim()).filter(Boolean);

  const save = async () => {
    const next: ImmersiveTarget = {
      ...d,
      name: name.value.trim(),
      exe: exe.value.trim(),
      port: Number(port.value),
      args: args.value.trim(),
      url_prefixes: lines(prefixes.value),
      skip_selectors: lines(skips.value),
    };
    const ok = await saveConfig((c) => {
      if (isNew) c.immersive.targets.push(next);
      else {
        const i = c.immersive.targets.findIndex((t) => t.port === editing);
        if (i >= 0) c.immersive.targets[i] = next;
      }
    }, isNew ? `已添加「${next.name}」` : "已保存");
    if (ok) cancelEdit();
  };

  return h(
    "div",
    { class: "card" },
    h("h3", { style: "margin-bottom:12px" }, isNew ? "添加应用" : `编辑「${d.name}」`),
    h(
      "div",
      { class: "form-grid" },
      field("名称", name),
      field("调试端口", port),
      field("程序路径（用于身份校验和启动）", exe, "wide"),
      field("启动参数（不含调试端口）", args, "wide"),
      field("只处理这些地址开头的页面（留空 = 全部）", prefixes, "wide"),
      field("不翻译的区域（CSS 选择器）", skips, "wide"),
    ),
    h(
      "div",
      { class: "row", style: "margin-top:14px; justify-content:flex-end" },
      h("button", { class: "btn", onclick: cancelEdit }, "取消"),
      h("button", { class: "btn primary", onclick: save }, "保存"),
    ),
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
  renderImmersive();
  try {
    toast(await invoke<string>("immersive_launch", { port: t.port }));
  } catch (e) {
    toast(errText(e), true);
  } finally {
    launching.delete(t.port);
    await loadView();
    renderImmersive();
  }
}

function addPanel(): HTMLElement {
  const card = h("div", { class: "card" });
  const header = h(
    "div",
    { class: "row spread" },
    h("div", { class: "grow" }, h("h3", {}, "添加应用"), h("div", { class: "muted small" }, "目前支持 Electron / Chromium 内核的应用。可以从正在运行的程序里识别，也可以手动填写。")),
    h("button", { class: "btn", disabled: detecting, onclick: detect }, detecting ? "识别中…" : detected ? "重新识别" : "识别正在运行的应用"),
    h(
      "button",
      {
        class: "btn",
        disabled: editing === -1,
        onclick: () => {
          editing = -1;
          draft = { name: "", enabled: true, exe: "", args: "", port: nextFreePort(), url_prefixes: [], skip_selectors: [] };
          renderImmersive();
        },
      },
      "手动添加",
    ),
  );
  card.append(header);
  if (detected) {
    if (detected.length === 0) {
      card.append(h("div", { class: "empty" }, "没有找到正在运行的 Electron 应用"));
    } else {
      const table = h(
        "table",
        { style: "margin-top:12px" },
        h("thead", {}, h("tr", {}, h("th", { style: "width:22%" }, "应用"), h("th", {}, "程序路径"), h("th", { style: "width:120px" }, ""))),
      );
      const tbody = h("tbody");
      for (const a of detected) {
        tbody.append(
          h(
            "tr",
            {},
            h("td", {}, a.name, a.port ? h("div", { class: "muted small" }, `已开调试端口 ${a.port}`) : null),
            h("td", { class: "path" }, a.exe),
            h(
              "td",
              {},
              a.configured
                ? h("span", { class: "muted small" }, "已在列表中")
                : h(
                    "button",
                    {
                      class: "btn",
                      onclick: () => {
                        editing = -1;
                        draft = { name: a.name, enabled: true, exe: a.exe, args: "", port: a.port ?? nextFreePort(), url_prefixes: [], skip_selectors: [] };
                        renderImmersive();
                      },
                    },
                    "添加",
                  ),
            ),
          ),
        );
      }
      table.append(tbody);
      card.append(table);
    }
  }
  return card;
}

async function detect() {
  detecting = true;
  renderImmersive();
  try {
    detected = await invoke<DetectedApp[]>("immersive_detect");
  } catch (e) {
    toast(errText(e), true);
  } finally {
    detecting = false;
    renderImmersive();
  }
}

// ───────────────────────── 译文库 ─────────────────────────

const PAGE_SIZE = 50;
const lib = { keyword: "", manualOnly: false, offset: 0, total: 0, rows: [] as Entry[] };
let libBuilt = false;
let searchTimer = 0;

function buildLibrary() {
  const root = document.getElementById("tab-library")!;
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
    h("h2", {}, "译文库"),
    h("p", { class: "lead" }, "翻译过的文字都存在这里，下次遇到相同原文直接使用、不再请求翻译引擎。改过的译文标记为手工译文，自动翻译不会覆盖；改动会立即推送到已接管的页面。"),
    h(
      "div",
      { class: "card" },
      h(
        "div",
        { class: "row" },
        h("div", { class: "grow" }, search),
        h("label", { class: "row small", style: "gap:4px; white-space:nowrap" }, manual, "只看手工译文"),
        h("button", { class: "btn danger", onclick: clearLibrary }, "清空…"),
      ),
      h("div", { id: "lib-table", style: "margin-top:12px" }),
    ),
  );
  libBuilt = true;
}

async function queryLibrary() {
  try {
    const page = await invoke<{ total: number; rows: Entry[] }>("store_query", {
      keyword: lib.keyword,
      manualOnly: lib.manualOnly,
      offset: lib.offset,
      limit: PAGE_SIZE,
    });
    lib.total = page.total;
    lib.rows = page.rows;
    renderLibraryTable();
  } catch (e) {
    toast(errText(e), true);
  }
}

function renderLibraryTable() {
  const box = document.getElementById("lib-table");
  if (!box) return;
  if (lib.rows.length === 0) {
    box.replaceChildren(h("div", { class: "empty" }, lib.keyword || lib.manualOnly ? "没有符合条件的译文" : "译文库还是空的"));
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
        renderLibraryTable();
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
        h("td", {}, r.source),
        h("td", {}, input),
        h("td", { class: "small muted" }, r.manual ? "手工" : r.engine),
        h("td", { class: "small muted" }, String(r.hits)),
        h("td", { class: "small muted" }, relTime(r.used_at)),
        h(
          "td",
          {},
          h(
            "button",
            {
              class: "link",
              title: "删除后，已接管的页面会重新翻译这段文字",
              onclick: async () => {
                try {
                  await invoke("store_delete", { sources: [r.source] });
                  toast("已删除，页面会重新翻译这段文字");
                  queryLibrary();
                } catch (e) {
                  toast(errText(e), true);
                }
              },
            },
            "删除",
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
          h("th", { style: "width:96px" }, "来源"),
          h("th", { style: "width:44px" }, "命中"),
          h("th", { style: "width:72px" }, "最近使用"),
          h("th", { style: "width:48px" }, ""),
        ),
      ),
      tbody,
    ),
    h(
      "div",
      { class: "row spread pager" },
      h("span", { class: "muted small" }, `共 ${lib.total} 条（目标语言 ${view.config.target_lang}）`),
      h(
        "div",
        { class: "row" },
        h("button", { class: "btn", disabled: cur <= 1, onclick: () => ((lib.offset -= PAGE_SIZE), queryLibrary()) }, "上一页"),
        h("span", { class: "small" }, `${cur} / ${pages}`),
        h("button", { class: "btn", disabled: cur >= pages, onclick: () => ((lib.offset += PAGE_SIZE), queryLibrary()) }, "下一页"),
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
      h("p", { class: "muted", style: "margin-top:0" }, `将删除目标语言 ${view.config.target_lang} 的译文，之后遇到这些文字会重新请求翻译引擎。已显示在页面上的译文要等页面刷新后才会更新。`),
      h("label", { class: "row small" }, keep, "保留手工译文"),
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
  } catch (e) {
    toast(errText(e), true);
  }
}

// ───────────────────────── 术语表 ─────────────────────────

let glossaryDraft: [string, string][] | null = null;
/** 自定义术语有未保存的修改 */
let glossaryDirty = false;
let builtinFilter = "";

function renderGlossary() {
  const root = document.getElementById("tab-glossary")!;
  if (!glossaryDraft) glossaryDraft = Object.entries(view.config.glossary);
  const rows = glossaryDraft;

  const tbody = h("tbody");
  rows.forEach((row, i) => {
    const src = h("input", { type: "text", value: row[0], placeholder: "原文（整条精确匹配）" });
    const dst = h("input", { type: "text", value: row[1], placeholder: "译文（与原文相同即保持不翻）" });
    src.addEventListener("input", () => ((row[0] = src.value), (glossaryDirty = true)));
    dst.addEventListener("input", () => ((row[1] = dst.value), (glossaryDirty = true)));
    tbody.append(
      h(
        "tr",
        {},
        h("td", {}, src),
        h("td", {}, dst),
        h("td", {}, h("button", { class: "link", onclick: () => (rows.splice(i, 1), (glossaryDirty = true), renderGlossary()) }, "删除")),
      ),
    );
  });
  if (rows.length === 0) tbody.append(h("tr", {}, h("td", { colspan: 3, class: "empty" }, "还没有自定义术语")));

  const saveGlossary = async () => {
    const map: Record<string, string> = {};
    for (const [k, v] of rows) {
      if (!k.trim()) continue;
      if (!v.trim()) {
        toast(`「${k}」的译文不能为空`, true);
        return;
      }
      map[k.trim()] = v;
    }
    const ok = await saveConfig((c) => (c.glossary = map), "术语表已保存，已接管的页面会立即更新");
    if (ok) {
      glossaryDraft = null;
      glossaryDirty = false;
      renderGlossary();
    }
  };

  const builtin = Object.entries(view.builtin_glossary);
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
    h("h2", {}, "术语表"),
    h("p", { class: "lead" }, "按整条原文精确匹配（忽略首尾空格），命中的既不查译文库也不请求翻译引擎。优先级：自定义术语 > 内置术语 > 译文库 > 翻译引擎。适合修正界面上单个词的错译，例如 New → 新建。"),
    h(
      "div",
      { class: "card" },
      h("div", { class: "row spread" }, h("h3", {}, `自定义术语（${rows.length}）`), h("div", { class: "row" },
        h("button", { class: "btn", onclick: () => (rows.push(["", ""]), (glossaryDirty = true), renderGlossary()) }, "添加一条"),
        h("button", { class: "btn primary", onclick: saveGlossary }, "保存术语表"),
      )),
      h(
        "table",
        { style: "margin-top:10px" },
        h("thead", {}, h("tr", {}, h("th", {}, "原文"), h("th", {}, "译文"), h("th", { style: "width:48px" }, ""))),
        tbody,
      ),
    ),
    h(
      "div",
      { class: "card" },
      h(
        "div",
        { class: "row spread" },
        h("div", { class: "grow" }, h("h3", {}, `内置术语表（${builtin.length} 条，目标语言 ${view.config.target_lang}）`), h("div", { class: "muted small" }, "常见界面词的固定译法。自定义术语里写同样的原文即可覆盖。")),
        switchEl(view.config.builtin_glossary, (v) => saveConfig((c) => (c.builtin_glossary = v), v ? "已启用内置术语表" : "已停用内置术语表")),
      ),
      builtin.length > 0 &&
        h(
          "details",
          { class: "pages", style: "margin-top:10px" },
          h("summary", {}, "查看内置术语"),
          h("div", { style: "margin:8px 0" }, filterInput),
          h("table", {}, h("thead", {}, h("tr", {}, h("th", {}, "原文"), h("th", {}, "译文"))), builtinBody),
        ),
    ),
  );
}

// ───────────────────────── 常规 ─────────────────────────

const LANGS: [string, string][] = [
  ["zh-CN", "简体中文"],
  ["zh-TW", "繁体中文"],
  ["en", "英语"],
  ["ja", "日语"],
  ["ko", "韩语"],
  ["fr", "法语"],
  ["de", "德语"],
  ["es", "西班牙语"],
  ["ru", "俄语"],
];

function renderGeneral() {
  const root = document.getElementById("tab-general")!;
  const c = view.config;
  const lang = h("select", {}, ...LANGS.map(([v, n]) => h("option", { value: v, selected: v === c.target_lang }, `${n}（${v}）`)));
  if (!LANGS.some(([v]) => v === c.target_lang)) lang.append(h("option", { value: c.target_lang, selected: true }, c.target_lang));
  const hotkey = h("input", { type: "text", value: c.hotkey });
  const imHotkey = h("input", { type: "text", value: c.immersive.hotkey });
  let persist = c.persist_cache;

  root.replaceChildren(
    h("h2", {}, "常规"),
    h("p", { class: "lead" }, "悬浮窗模式和沉浸模式共用这些设置。"),
    h(
      "div",
      { class: "card stack" },
      field("目标语言", lang),
      field("框选翻译快捷键（重启后生效）", hotkey),
      field("沉浸模式原文/译文切换快捷键（重启后生效）", imHotkey),
      h(
        "div",
        { class: "row spread" },
        h("div", {}, h("div", {}, "使用译文库"), h("div", { class: "muted small" }, "翻译过的文字存到本地，相同原文不再请求翻译引擎。")),
        switchEl(persist, (v) => (persist = v)),
      ),
      h(
        "div",
        { class: "row", style: "justify-content:flex-end" },
        h(
          "button",
          {
            class: "btn primary",
            onclick: () =>
              saveConfig((cfg) => {
                cfg.target_lang = lang.value;
                cfg.hotkey = hotkey.value.trim();
                cfg.immersive.hotkey = imHotkey.value.trim();
                cfg.persist_cache = persist;
              }),
          },
          "保存",
        ),
      ),
    ),
    h(
      "div",
      { class: "card stack" },
      h("h3", {}, "文件位置"),
      field("配置文件", h("div", { class: "path" }, view.config_path)),
      field("译文库", h("div", { class: "path" }, view.db_path)),
      h("div", { class: "muted small" }, "翻译引擎的设置暂时请直接编辑配置文件（参考仓库里的 config.example.json）。"),
    ),
  );
}

// ───────────────────────── 启动 ─────────────────────────

function renderAll() {
  renderImmersive();
  renderGlossary();
  renderGeneral();
  if (!libBuilt) buildLibrary();
}

function switchTab(name: string) {
  for (const b of document.querySelectorAll<HTMLButtonElement>("#nav button")) b.classList.toggle("active", b.dataset.tab === name);
  for (const s of document.querySelectorAll<HTMLElement>(".tab")) s.classList.toggle("active", s.id === `tab-${name}`);
  if (name === "library") queryLibrary();
}

for (const b of document.querySelectorAll<HTMLButtonElement>("#nav button")) {
  b.addEventListener("click", () => switchTab(b.dataset.tab!));
}

(async () => {
  try {
    await loadView();
    statuses = await invoke<TargetStatus[]>("immersive_status");
    renderAll();
  } catch (e) {
    document.getElementById("tab-immersive")!.replaceChildren(h("div", { class: "note bad" }, `读取设置失败：${errText(e)}`));
  }
  await listen("config-changed", async () => {
    await loadView();
    // 正在编辑的表单不打断；其余页面按新配置重绘
    if (editing === null) renderImmersive();
    if (!glossaryDirty) {
      glossaryDraft = null;
      renderGlossary();
    }
    renderGeneral();
  });
  await listen<TargetStatus[]>("immersive-status", (e) => {
    statuses = e.payload;
    // 编辑表单打开时不重绘，免得输入被打断
    if (editing === null) renderImmersive();
  });
})();
