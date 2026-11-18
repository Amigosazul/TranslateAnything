// 设置窗口的数据：类型、全局状态、保存配置、翻译方式换算
import { invoke } from "@tauri-apps/api/core";
import { clone, errText, toast } from "./ui";

// ───────────────────────── 类型 ─────────────────────────

export interface ImmersiveTarget {
  name: string;
  enabled: boolean;
  exe: string;
  args: string;
  port: number;
  url_prefixes: string[];
  skip_selectors: string[];
}

export interface ProviderConfig {
  type?: string;
  base_url: string;
  api_key: string;
  region: string;
  model: string;
  proxy: string | null;
}

export interface Config {
  hotkey: string;
  target_lang: string;
  mode: string;
  ocr_lang: string;
  engine: string;
  fallback: string[];
  engines: Record<string, ProviderConfig>;
  persist_cache: boolean;
  builtin_glossary: boolean;
  glossary: Record<string, string>;
  immersive: { enabled: boolean; hotkey: string; targets: ImmersiveTarget[] };
  [key: string]: unknown;
}

export interface SettingsView {
  config: Config;
  config_path: string;
  config_size: number;
  db_path: string;
  db_size: number;
  db_custom: boolean;
  builtin_glossary: Record<string, string>;
}

export type StateName = "off" | "disabled" | "not_running" | "no_port" | "conflict" | "waiting" | "attached";

export interface TargetStatus {
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

export interface EngineInfo {
  id: string;
  name: string;
  needs_key: boolean;
  official: boolean;
  fields: string[];
  note: string;
}

export interface StoreStats {
  total: number;
  manual: number;
  hits: number;
}

export interface EngineTest {
  engine: string;
  source: string;
  text: string;
  ms: number;
}

// ───────────────────────── 全局状态 ─────────────────────────

export const S = {
  view: undefined as unknown as SettingsView,
  statuses: [] as TargetStatus[],
  catalog: [] as EngineInfo[],
  stats: null as StoreStats | null,
  frames: 0,
  ocrLangs: null as { tag: string; name: string }[] | null,
  /** 各引擎最近一次测试结果（按引擎 id） */
  tests: {} as Record<string, { busy: boolean; result?: EngineTest; error?: string }>,
};

export const cfg = () => S.view.config;

let rerender: () => void = () => {};
export function setRerender(fn: () => void) {
  rerender = fn;
}

/** 重绘界面，并尽量把焦点还给重绘前聚焦的那个控件（按 data-k 对应） */
export function refresh() {
  const key = (document.activeElement as HTMLElement | null)?.dataset?.k;
  rerender();
  if (key) document.querySelector<HTMLElement>(`[data-k="${key}"]`)?.focus({ preventScroll: true });
}

export async function loadView() {
  S.view = await invoke<SettingsView>("settings_get");
}

export async function loadStats() {
  try {
    S.stats = await invoke<StoreStats>("store_stats");
  } catch {
    S.stats = null;
  }
}

/** 修改配置并保存；成功后重新读取并刷新界面 */
export async function saveConfig(mutate: (c: Config) => void, okMsg = "已保存"): Promise<boolean> {
  const next = clone(S.view.config);
  mutate(next);
  try {
    await invoke("settings_save", { config: next });
    await loadView();
    refresh();
    if (okMsg) toast(okMsg);
    return true;
  } catch (e) {
    toast(errText(e), true);
    refresh();
    return false;
  }
}

export const LANGS: [string, string][] = [
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

export const langName = (tag: string) => LANGS.find(([v]) => v === tag)?.[1] ?? tag;

// ───────────────────────── 翻译方式 ─────────────────────────
// 配置里只有"主用引擎 + 回退顺序"。界面上的"翻译方式"由主用引擎的类型推出来：
// 主用的是大模型（openai 兼容）就是"大模型"，否则是"翻译引擎"。

export const LLM_KIND = "openai";

export const kindOf = (c: Config, id: string) => (c.engines?.[id]?.type || "").trim() || id;
export const isLLM = (c: Config, id: string) => kindOf(c, id) === LLM_KIND;
export const engineInfo = (kind: string) => S.catalog.find((e) => e.id === kind);
export const engineLabel = (c: Config, id: string) => engineInfo(kindOf(c, id))?.name ?? id;

/** 配置里的大模型引擎 id：已有 openai 类型的配置就用它，否则用 "openai" */
export function llmId(c: Config): string {
  return Object.keys(c.engines).find((id) => isLLM(c, id)) ?? LLM_KIND;
}

export type Method = "engine" | "llm";
export const methodOf = (c: Config): Method => (isLLM(c, c.engine) ? "llm" : "engine");

/** 传统引擎的尝试顺序（主用 + 回退，去掉大模型），没有就用 edge */
export function engineChain(c: Config): string[] {
  const chain = [c.engine, ...c.fallback].map((s) => s.trim()).filter((id, i, a) => id && !isLLM(c, id) && a.indexOf(id) === i);
  return chain.length ? chain : ["edge"];
}

/**
 * 切换翻译方式。
 * - 切到大模型：大模型做主用，原来的传统引擎顺序整体变成回退（大模型出错时自动退回传统引擎）；
 * - 切回翻译引擎：传统引擎顺序恢复为主用 + 回退。
 */
export function applyMethod(c: Config, m: Method) {
  const chain = engineChain(c);
  if (m === "llm") {
    const id = llmId(c);
    c.engines[id] ??= { base_url: "", api_key: "", region: "", model: "", proxy: null };
    c.engine = id;
    c.fallback = chain;
  } else {
    c.engine = chain[0];
    c.fallback = chain.slice(1);
  }
}

/** 大模型是否已填好必填项 */
export function llmReady(c: Config): boolean {
  const p = c.engines[llmId(c)];
  return !!p && !!p.base_url?.trim() && !!p.model?.trim();
}

/** 测试某个引擎（用已保存的配置），结果记到 S.tests 并刷新界面 */
export async function testEngine(id: string) {
  S.tests[id] = { busy: true };
  refresh();
  try {
    S.tests[id] = { busy: false, result: await invoke<EngineTest>("engine_test", { id }) };
  } catch (e) {
    S.tests[id] = { busy: false, error: errText(e) };
  }
  refresh();
}
