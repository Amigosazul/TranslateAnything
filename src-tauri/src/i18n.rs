//! 界面多语言
//!
//! 文案统一放在仓库的 `src/i18n/<语言>.json`（扁平键值，前端与 Rust 共用同一份），编译时嵌入程序。
//! 以 `zh-CN` 为准，`en` 必须同步补齐；其他语言缺的键自动回退到英文。
//!
//! 界面语言按 设置（`ui_lang`）→ 系统界面语言 → 同语种（如 `zh-TW` → `zh-CN`）→ `en` 的顺序确定。
//! 前端通过 `ui_i18n` 命令一次拿到合并好回退的整份文案，不在前端重复打包语言文件。
//!
//! 只翻面向用户的文案；控制台日志仍用中文，不进语言文件。

use serde::Serialize;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// 内置语言：（语言标签，文案 JSON）。新增语言在这里加一行并放一个同名 JSON 文件
const LOCALES: &[(&str, &str)] = &[
    ("zh-CN", include_str!("../../src/i18n/zh-CN.json")),
    ("en", include_str!("../../src/i18n/en.json")),
];

/// 全部文案都缺失时的兜底语言
const FALLBACK: &str = "en";

type Messages = HashMap<String, String>;

fn parsed() -> &'static HashMap<&'static str, Messages> {
    static P: OnceLock<HashMap<&'static str, Messages>> = OnceLock::new();
    P.get_or_init(|| {
        LOCALES
            .iter()
            .map(|(tag, json)| {
                let m: Messages = serde_json::from_str(json).unwrap_or_else(|e| panic!("语言文件 {tag}.json 格式错误：{e}"));
                (*tag, m)
            })
            .collect()
    })
}

/// 当前生效的界面语言及其（已合并英文回退的）文案
struct Current {
    locale: &'static str,
    messages: Messages,
}

fn current() -> &'static RwLock<Current> {
    static C: OnceLock<RwLock<Current>> = OnceLock::new();
    C.get_or_init(|| RwLock::new(build(resolve(""))))
}

fn build(locale: &'static str) -> Current {
    let p = parsed();
    let mut messages = p.get(FALLBACK).cloned().unwrap_or_default();
    if locale != FALLBACK {
        if let Some(m) = p.get(locale) {
            messages.extend(m.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    Current { locale, messages }
}

/// 按用户设置切换界面语言（空或 `auto` = 跟随系统）；返回是否发生了变化
pub fn set_pref(pref: &str) -> bool {
    let locale = resolve(pref);
    let mut c = current().write().unwrap();
    if c.locale == locale {
        return false;
    }
    *c = build(locale);
    true
}

/// 当前界面语言标签，如 `zh-CN`
pub fn locale() -> &'static str {
    current().read().unwrap().locale
}

/// 取文案；键不存在时原样返回键名（方便发现漏翻）
pub fn get(key: &str) -> String {
    current().read().unwrap().messages.get(key).cloned().unwrap_or_else(|| key.to_string())
}

/// 把文案里的 `{name}` 占位符替换成参数
pub fn fill(mut s: String, args: &[(&str, String)]) -> String {
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// 取文案：`t!("tray.quit")`、`t!("err.port_range", name = t.name, port = 80)`
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::get($key)
    };
    ($key:expr, $($name:ident = $val:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::get($key), &[$((stringify!($name), ($val).to_string())),+])
    };
}

/// 按 设置 → 系统语言 → 同语种 → 英文 确定界面语言
fn resolve(pref: &str) -> &'static str {
    let pref = pref.trim();
    let mut candidates = Vec::new();
    if !pref.is_empty() && pref != "auto" {
        candidates.push(pref.to_string());
    }
    candidates.push(system_locale());
    for want in &candidates {
        if let Some(tag) = match_locale(want) {
            return tag;
        }
    }
    FALLBACK
}

/// 先完全匹配，再按语种（`-` 前的部分）匹配
fn match_locale(want: &str) -> Option<&'static str> {
    let want = want.to_ascii_lowercase();
    if let Some((tag, _)) = LOCALES.iter().find(|(t, _)| t.to_ascii_lowercase() == want) {
        return Some(tag);
    }
    let base = want.split(['-', '_']).next().unwrap_or("");
    LOCALES.iter().find(|(t, _)| t.split('-').next().unwrap_or("").eq_ignore_ascii_case(base)).map(|(t, _)| *t)
}

/// 系统界面语言标签，如 `zh-CN`、`en-US`。可用环境变量 `TA_LOCALE` 覆盖（自测用）
pub fn system_locale() -> String {
    if let Ok(v) = std::env::var("TA_LOCALE") {
        if !v.trim().is_empty() {
            return v.trim().to_string();
        }
    }
    os_locale().unwrap_or_else(|| "en-US".into())
}

#[cfg(windows)]
fn os_locale() -> Option<String> {
    use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
    let mut buf = [0u16; 85];
    // SAFETY: 缓冲区长度足够放下 LOCALE_NAME_MAX_LENGTH（85）个字符
    let n = unsafe { LCIDToLocaleName(GetUserDefaultUILanguage() as u32, Some(&mut buf), 0) };
    (n > 1).then(|| String::from_utf16_lossy(&buf[..n as usize - 1]))
}

#[cfg(not(windows))]
fn os_locale() -> Option<String> {
    std::env::var("LC_ALL").or_else(|_| std::env::var("LANG")).ok().map(|v| v.split('.').next().unwrap_or("").replace('_', "-"))
}

/// 首次启动时默认的目标语言：按系统语言换算成翻译引擎认的语言标签
pub fn default_target_lang() -> String {
    let sys = system_locale().to_ascii_lowercase();
    let base = sys.split(['-', '_']).next().unwrap_or("").to_string();
    match base.as_str() {
        // 繁体：台湾、香港、澳门，或显式带 Hant 的
        "zh" if sys.contains("hant") || sys.ends_with("-tw") || sys.ends_with("-hk") || sys.ends_with("-mo") => "zh-TW".into(),
        "zh" => "zh-CN".into(),
        "pt" if sys.ends_with("-pt") => "pt-PT".into(),
        "pt" => "pt".into(),
        "" => "en".into(),
        _ => base,
    }
}

#[derive(Serialize)]
pub struct I18nView {
    /// 当前界面语言
    locale: &'static str,
    /// 已合并英文回退的整份文案
    messages: Messages,
    /// 可选的界面语言：（标签，该语言自己的名称）
    locales: Vec<(&'static str, String)>,
}

/// 前端启动时取一次文案
#[tauri::command]
pub fn ui_i18n() -> I18nView {
    let c = current().read().unwrap();
    let locales = LOCALES
        .iter()
        .map(|(tag, _)| (*tag, parsed()[tag].get("meta.name").cloned().unwrap_or_else(|| tag.to_string())))
        .collect();
    I18nView { locale: c.locale, messages: c.messages.clone(), locales }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 语种匹配与回退() {
        assert_eq!(match_locale("zh-CN"), Some("zh-CN"));
        assert_eq!(match_locale("zh-TW"), Some("zh-CN"));
        assert_eq!(match_locale("en-GB"), Some("en"));
        assert_eq!(match_locale("ja-JP"), None);
    }

    #[test]
    fn 占位符() {
        assert_eq!(fill("端口 {port} 被 {name} 占用".into(), &[("port", "80".into()), ("name", "a".into())]), "端口 80 被 a 占用");
    }

    /// 各语言文件的键必须与 zh-CN 一致（en 必须齐全；其他语言可缺，但不能多出 zh-CN 没有的键）
    #[test]
    fn 语言文件键一致() {
        let p = parsed();
        let base = &p["zh-CN"];
        for (tag, m) in p {
            for k in m.keys() {
                assert!(base.contains_key(k), "{tag}.json 多出了 zh-CN.json 没有的键：{k}");
            }
            if *tag == "en" {
                for k in base.keys() {
                    assert!(m.contains_key(k), "en.json 缺少键：{k}");
                }
            }
        }
    }
}
