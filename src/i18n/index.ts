// 界面多语言（前端）
//
// 文案在 src/i18n/<语言>.json，由后端按“设置 → 系统语言 → 同语种 → 英文”选好语言、合并好回退后
// 通过 ui_i18n 命令整份下发，前端不重复打包语言文件。各窗口入口先 await initI18n() 再渲染。
//
// 用法：t("nav.dashboard")、t("lib.count", { n: 3 })；静态 HTML 里用 data-i18n / data-i18n-title /
// data-i18n-aria / data-i18n-placeholder 标注，initI18n 时自动替换。
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface I18nView {
  locale: string;
  messages: Record<string, string>;
  locales: [string, string][];
}

let messages: Record<string, string> = {};
let current = "en";
let available: [string, string][] = [];

/** 当前界面语言，如 zh-CN、en */
export const locale = () => current;
/** 当前界面语言是否为中文 */
export const isZh = () => current.startsWith("zh");
/** 可选的界面语言：[标签, 该语言自己的名称] */
export const uiLocales = () => available;

/** 取文案；`{name}` 占位符用 vars 替换。键不存在时返回键名，方便发现漏翻 */
export function t(key: string, vars?: Record<string, string | number>): string {
  let s = messages[key] ?? key;
  if (vars) for (const [k, v] of Object.entries(vars)) s = s.split(`{${k}}`).join(String(v));
  return s;
}

/** 按当前语言格式化数字 */
export const fmtNum = (n: number) => n.toLocaleString(current);
/** 按当前语言格式化日期 */
export const fmtDate = (d: Date) => d.toLocaleDateString(current);

/** 用当前界面语言显示某个语言标签的名称，如 ja → 日语 / Japanese */
export function langLabel(tag: string): string {
  try {
    return new Intl.DisplayNames([current], { type: "language" }).of(tag) ?? tag;
  } catch {
    return tag;
  }
}

/** 替换静态 HTML 里带 data-i18n* 标注的文字 */
export function applyStatic(root: ParentNode = document) {
  for (const el of root.querySelectorAll<HTMLElement>("[data-i18n]")) el.textContent = t(el.dataset.i18n!);
  for (const el of root.querySelectorAll<HTMLElement>("[data-i18n-title]")) el.title = t(el.dataset.i18nTitle!);
  for (const el of root.querySelectorAll<HTMLElement>("[data-i18n-aria]")) el.setAttribute("aria-label", t(el.dataset.i18nAria!));
  for (const el of root.querySelectorAll<HTMLInputElement>("[data-i18n-placeholder]")) el.placeholder = t(el.dataset.i18nPlaceholder!);
  const title = document.querySelector<HTMLElement>("title[data-i18n]");
  if (title) document.title = title.textContent ?? "";
  document.documentElement.lang = current;
}

/**
 * 载入文案并替换静态文字。`onChange` 省略时，界面语言改变后整页重新载入。
 */
export async function initI18n(onChange?: () => void) {
  const v = await invoke<I18nView>("ui_i18n");
  messages = v.messages;
  current = v.locale;
  available = v.locales;
  applyStatic();
  await listen("locale-changed", () => (onChange ? onChange() : location.reload()));
}
