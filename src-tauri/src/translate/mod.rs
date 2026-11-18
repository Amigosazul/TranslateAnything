//! 翻译 Provider 抽象。配置决定一切，代码里不写死任何服务商的 key。
//!
//! 引擎清单见 [`CATALOG`]（设置界面也从这里读取可选引擎和所需字段）。

mod deepl;
mod deeplx;
mod edge;
mod google;
mod google_free;
mod microsoft;
mod openai;

use crate::config::{Config, ProviderConfig};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;
use std::time::Duration;

#[derive(Debug)]
pub enum TranslateError {
    /// 配置缺失或不合法
    Config(String),
    /// 网络错误（连不上、超时等）
    Network(String),
    /// 服务端返回了错误
    Api(String),
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(m) => f.write_str(&t!("err.engine.config", m = m)),
            Self::Network(m) => f.write_str(&t!("err.engine.network", m = m)),
            Self::Api(m) => f.write_str(&t!("err.engine.api", m = m)),
        }
    }
}

#[async_trait]
pub trait Translator: Send + Sync {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError>;

    /// 批量翻译；默认逐条调用，支持批量的接口应覆盖以减少请求次数
    async fn translate_batch(&self, texts: &[String], target_lang: &str) -> Result<Vec<String>, TranslateError> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.translate(t, target_lang).await?);
        }
        Ok(out)
    }

    // 扩展点（P2 视觉直译）：translate_image(&self, png: &[u8], target_lang)，默认返回"不支持"
}

// ───────────────────────── 引擎清单 ─────────────────────────

/// 引擎元信息，供设置界面渲染（M3）。
/// `name` / `note` 存的是文案键，序列化时按当前界面语言换成文字
#[derive(Debug, Serialize)]
pub struct EngineInfo {
    /// 即配置里的 `type`
    pub id: &'static str,
    #[serde(serialize_with = "localized")]
    pub name: &'static str,
    /// 是否必须填写 api_key
    pub needs_key: bool,
    /// 是否为官方公开 API（否则是非官方免费接口，随时可能失效）
    pub official: bool,
    /// 该引擎会用到的配置字段
    pub fields: &'static [&'static str],
    #[serde(serialize_with = "localized")]
    pub note: &'static str,
}

fn localized<S: serde::Serializer>(key: &&'static str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&t!(key))
}

pub const CATALOG: &[EngineInfo] = &[
    EngineInfo {
        id: "edge",
        name: "engine.edge.name",
        needs_key: false,
        official: false,
        fields: &["proxy"],
        note: "engine.edge.note",
    },
    EngineInfo {
        id: "bing",
        name: "engine.bing.name",
        needs_key: false,
        official: false,
        fields: &["proxy"],
        note: "engine.bing.note",
    },
    EngineInfo {
        id: "google_free",
        name: "engine.google_free.name",
        needs_key: false,
        official: false,
        fields: &["base_url", "proxy"],
        note: "engine.google_free.note",
    },
    EngineInfo {
        id: "microsoft",
        name: "engine.microsoft.name",
        needs_key: true,
        official: true,
        fields: &["api_key", "region", "base_url", "proxy"],
        note: "engine.microsoft.note",
    },
    EngineInfo {
        id: "deepl",
        name: "engine.deepl.name",
        needs_key: true,
        official: true,
        fields: &["api_key", "base_url", "proxy"],
        note: "engine.deepl.note",
    },
    EngineInfo {
        id: "deeplx",
        name: "engine.deeplx.name",
        needs_key: false,
        official: false,
        fields: &["base_url", "api_key", "proxy"],
        note: "engine.deeplx.note",
    },
    EngineInfo {
        id: "google",
        name: "engine.google.name",
        needs_key: true,
        official: true,
        fields: &["api_key", "base_url", "proxy"],
        note: "engine.google.note",
    },
    EngineInfo {
        id: "openai",
        name: "engine.openai.name",
        needs_key: true,
        official: true,
        fields: &["base_url", "api_key", "model", "proxy"],
        note: "engine.openai.note",
    },
];

/// 按引擎类型创建 Provider
pub fn create(kind: &str, cfg: &ProviderConfig) -> Result<Box<dyn Translator>, TranslateError> {
    Ok(match kind {
        "edge" => Box::new(edge::Edge::new(cfg, edge::Route::EdgeThenBing)?),
        "bing" => Box::new(edge::Edge::new(cfg, edge::Route::BingOnly)?),
        "google_free" => Box::new(google_free::GoogleFree::new(cfg)?),
        "microsoft" => Box::new(microsoft::Microsoft::new(cfg)?),
        "deepl" => Box::new(deepl::DeepL::new(cfg)?),
        "deeplx" => Box::new(deeplx::DeepLX::new(cfg)?),
        "google" => Box::new(google::Google::new(cfg)?),
        "openai" => Box::new(openai::OpenAICompatible::new(cfg)?),
        other => {
            let ids: Vec<_> = CATALOG.iter().map(|e| e.id).collect();
            return Err(TranslateError::Config(t!(
                "err.engine.unsupported",
                kind = other,
                ids = ids.join(" / ")
            )));
        }
    })
}

/// 引擎的显示名（按当前界面语言）；不在清单里的原样返回
fn engine_name(kind: &str) -> String {
    CATALOG.iter().find(|e| e.id == kind).map_or_else(|| kind.to_string(), |e| t!(e.name))
}

/// 本地缓存全部命中时返回的"引擎名"（按当前界面语言）
pub fn cache_engine() -> String {
    t!("engine.cache")
}

/// 术语表：内置界面常用词（`glossary.json`，按目标语言）+ 配置里的 `glossary`（后者优先）。
/// 按整条原文精确匹配（忽略首尾空白），命中的不查库也不请求引擎。
/// 传统翻译引擎翻单个界面词没有上下文，容易出"New → 新款""Auto → 汽车"这类错，靠它兜住。
pub fn glossary(cfg: &Config) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if cfg.builtin_glossary {
        map.extend(builtin_glossary(&cfg.target_lang));
    }
    map.extend(cfg.glossary.iter().map(|(k, v)| (k.trim().to_string(), v.clone())));
    map
}

/// 某个目标语言的内置术语表（设置界面展示用）
pub fn builtin_glossary(target_lang: &str) -> std::collections::BTreeMap<String, String> {
    static BUILTIN: OnceLock<HashMap<String, std::collections::BTreeMap<String, String>>> = OnceLock::new();
    let builtin = BUILTIN.get_or_init(|| serde_json::from_str(include_str!("glossary.json")).unwrap_or_default());
    let key = match primary(target_lang).as_str() {
        "zh" if is_traditional(target_lang) => "zh-TW".to_string(),
        "zh" => "zh-CN".to_string(),
        p => p.to_string(),
    };
    builtin.get(&key).cloned().unwrap_or_default()
}

/// 批量翻译：术语表 → 本地译文库 → 翻译引擎（主引擎 + 回退），引擎结果写回译文库。
/// 返回（与 `texts` 一一对应的译文，实际使用的引擎名；没有请求引擎时为"本地缓存"）
pub async fn translate_batch_with_fallback(cfg: &Config, texts: &[String]) -> Result<(Vec<String>, String), String> {
    let lang = cfg.target_lang.as_str();
    let terms = glossary(cfg);
    let mut found: HashMap<String, String> = texts
        .iter()
        .filter_map(|t| terms.get(t.trim()).map(|g| (t.clone(), g.clone())))
        .collect();
    let rest: Vec<String> = texts.iter().filter(|t| !found.contains_key(*t)).cloned().collect();
    if cfg.persist_cache && !rest.is_empty() {
        found.extend(crate::store::get_many(lang, &rest));
    }
    let mut missing: Vec<String> = Vec::new();
    for t in rest {
        if !found.contains_key(&t) && !missing.contains(&t) {
            missing.push(t);
        }
    }
    let engine = if missing.is_empty() {
        cache_engine()
    } else {
        let (translated, kind) = translate_by_engines(cfg, &missing).await?;
        let pairs: Vec<(String, String)> = missing.into_iter().zip(translated).collect();
        if cfg.persist_cache {
            // 译文库里记引擎 id（与界面语言无关），显示时再换成当前语言的名称
            crate::store::put_many(lang, &pairs, &kind);
        }
        found.extend(pairs);
        engine_name(&kind)
    };
    Ok((texts.iter().map(|t| found[t].clone()).collect(), engine))
}

/// 按配置的主引擎 + 回退顺序依次尝试批量翻译（不查缓存），返回（译文，实际使用的引擎类型 id）
async fn translate_by_engines(cfg: &Config, texts: &[String]) -> Result<(Vec<String>, String), String> {
    let mut errors = Vec::new();
    for (id, provider) in cfg.engine_chain() {
        let kind = provider.kind_or(&id);
        let result = match create(&kind, &provider) {
            Ok(t) => t.translate_batch(texts, &cfg.target_lang).await,
            Err(e) => Err(e),
        };
        match result {
            Ok(t) if t.len() == texts.len() => return Ok((t, kind)),
            Ok(_) => errors.push(t!("err.engine.count_mismatch", name = engine_name(&kind))),
            Err(e) => errors.push(t!("err.engine.prefixed", name = engine_name(&kind), e = e)),
        }
    }
    Err(errors.join("\n"))
}

/// 引擎测试结果（设置窗口"测试"按钮）
#[derive(Debug, Serialize)]
pub struct EngineTest {
    pub engine: String,
    /// 测试用原文与译文
    pub source: String,
    pub text: String,
    /// 耗时（毫秒），含建立连接
    pub ms: u64,
}

/// 用一句固定的英文测试某个已配置的引擎（不查缓存、不写译文库、不回退）
pub async fn test_engine(cfg: &Config, id: &str) -> Result<EngineTest, String> {
    const SAMPLE: &str = "Hello! Screen text, translated anywhere.";
    let provider = cfg.engines.get(id).cloned().unwrap_or_default();
    let kind = provider.kind_or(id);
    let name = engine_name(&kind);
    let start = std::time::Instant::now();
    let text = create(&kind, &provider)
        .map_err(|e| e.to_string())?
        .translate(SAMPLE, &cfg.target_lang)
        .await
        .map_err(|e| e.to_string())?;
    Ok(EngineTest { engine: name, source: SAMPLE.into(), text, ms: start.elapsed().as_millis() as u64 })
}

// ───────────────────────── 公共工具 ─────────────────────────

/// 免 key 接口伪装成浏览器时使用的 UA
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0";

/// 按配置构造 HTTP 客户端
///
/// 代理：留空 = 直连（本机环境变量里的代理曾导致 HTTPS 握手失败）；
/// `system` = 跟随系统环境变量；其他 = 代理地址
fn build_client(cfg: &ProviderConfig) -> Result<reqwest::Client, TranslateError> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60));
    builder = match cfg.proxy.as_deref().map(str::trim) {
        Some("system") => builder,
        Some(p) if !p.is_empty() => builder.proxy(
            reqwest::Proxy::all(p).map_err(|e| TranslateError::Config(t!("err.engine.bad_proxy", e = e)))?,
        ),
        _ => builder.no_proxy(),
    };
    builder
        .build()
        .map_err(|e| TranslateError::Config(e.to_string()))
}

fn require_key(cfg: &ProviderConfig, hint: &str) -> Result<String, TranslateError> {
    let key = cfg.api_key.trim();
    if key.is_empty() || key == "sk-xxx" || key.contains("在此填写") {
        return Err(TranslateError::Config(t!("err.engine.no_key", hint = hint)));
    }
    Ok(key.to_string())
}

fn network_err(e: reqwest::Error) -> TranslateError {
    let host = e.url().and_then(|u| u.host_str()).unwrap_or("").to_string();
    let mut msg = if e.is_timeout() { t!("err.engine.timeout") } else { e.to_string() };
    // 带上底层原因，方便排查（TLS、DNS 等）
    let mut src = std::error::Error::source(&e);
    while let Some(s) = src {
        msg.push_str(&format!("：{s}"));
        src = s.source();
    }
    if !host.is_empty() {
        msg.push_str(&format!("（{host}）"));
    }
    TranslateError::Network(msg)
}

/// 发送请求并解析 JSON；非 2xx 时用 `err_pointer` 指向的字段作为错误信息
async fn send_json(req: reqwest::RequestBuilder, err_pointer: &str) -> Result<Value, TranslateError> {
    let resp = req.send().await.map_err(network_err)?;
    let status = resp.status();
    let raw = resp.text().await.map_err(network_err)?;
    let value: Option<Value> = serde_json::from_str(&raw).ok();
    if !status.is_success() {
        let msg = value
            .as_ref()
            .and_then(|v| v.pointer(err_pointer))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| raw.chars().take(300).collect());
        return Err(TranslateError::Api(format!("HTTP {status}：{msg}")));
    }
    value.ok_or_else(|| TranslateError::Api(t!("err.engine.not_json", body = raw.chars().take(300).collect::<String>())))
}

fn parse_err(v: &Value) -> TranslateError {
    TranslateError::Api(t!("err.engine.bad_response", body = v.to_string().chars().take(300).collect::<String>()))
}

/// 表单编码（application/x-www-form-urlencoded）
fn form_encode(pairs: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse("http://localhost/").unwrap();
    url.query_pairs_mut().extend_pairs(pairs);
    url.query().unwrap_or("").to_string()
}

/// 把 OCR 的硬换行合并成自然段，传统翻译引擎逐行翻译会把句子拆碎
///
/// 规则：上一行以句末标点结尾才保留换行；英文行尾连字符断词时去掉连字符直接拼接
pub fn merge_lines(text: &str) -> String {
    const ENDS: &[char] = &['.', '!', '?', ':', ';', '。', '！', '？', '：', '；', '…', '"', '”', ')', '）'];
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if out.is_empty() {
            out.push_str(line);
            continue;
        }
        let last = out.chars().last().unwrap();
        let first = line.chars().next().unwrap();
        if ENDS.contains(&last) {
            out.push('\n');
        } else if last == '-' && first.is_lowercase() {
            out.pop();
        } else if last.is_ascii() || first.is_ascii() {
            out.push(' ');
        }
        // 两边都是 CJK 字符时直接拼接
        out.push_str(line);
    }
    out
}

// ───────────────────────── 语言代码映射 ─────────────────────────

/// 语言标签的主子标签（小写），如 `zh-CN` → `zh`
fn primary(lang: &str) -> String {
    lang.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase()
}

/// 是否为繁体中文标签
fn is_traditional(lang: &str) -> bool {
    let l = lang.to_ascii_lowercase();
    l.contains("tw") || l.contains("hk") || l.contains("mo") || l.contains("hant")
}

/// 微软系：zh-CN → zh-Hans，zh-TW → zh-Hant，其余取主子标签
fn ms_lang(lang: &str) -> String {
    match primary(lang).as_str() {
        "zh" if is_traditional(lang) => "zh-Hant".into(),
        "zh" => "zh-Hans".into(),
        p => p.into(),
    }
}

/// Google 系：中文用 zh-CN / zh-TW，其余取主子标签
fn google_lang(lang: &str) -> String {
    match primary(lang).as_str() {
        "zh" if is_traditional(lang) => "zh-TW".into(),
        "zh" => "zh-CN".into(),
        p => p.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 合并英文断行() {
        let s = "This is a sentence that\nwas broken by OCR. And a hyph-\nenated word.\nNew para";
        assert_eq!(
            merge_lines(s),
            "This is a sentence that was broken by OCR. And a hyphenated word.\nNew para"
        );
    }

    #[test]
    fn 合并中文断行() {
        assert_eq!(merge_lines("这是一句被\n打断的话。\n下一段"), "这是一句被打断的话。\n下一段");
    }

    #[test]
    fn 语言映射() {
        assert_eq!(ms_lang("zh-CN"), "zh-Hans");
        assert_eq!(ms_lang("zh-TW"), "zh-Hant");
        assert_eq!(google_lang("zh-Hans"), "zh-CN");
        assert_eq!(ms_lang("ja-JP"), "ja");
    }

    #[test]
    fn 术语表() {
        let mut cfg = Config::default();
        let g = glossary(&cfg);
        assert_eq!(g["New"], "新建");
        cfg.glossary.insert(" New ".into(), "新的".into());
        assert_eq!(glossary(&cfg)["New"], "新的", "用户术语优先");
        cfg.builtin_glossary = false;
        cfg.glossary.clear();
        assert!(glossary(&cfg).is_empty());
        cfg.builtin_glossary = true;
        cfg.target_lang = "ja".into();
        assert!(!glossary(&cfg).contains_key("New"), "其他目标语言不用中文术语");
    }

    #[test]
    fn 清单覆盖所有引擎() {
        let cfg = ProviderConfig::default();
        let ids: Vec<_> = CATALOG.iter().map(|e| e.id).collect();
        for e in CATALOG {
            // 只要不是"不支持的类型"即可（缺 key 之类的配置错误是预期的）
            if let Err(TranslateError::Config(m)) = create(e.id, &cfg) {
                let unsupported = t!("err.engine.unsupported", kind = e.id, ids = ids.join(" / "));
                assert_ne!(m, unsupported, "{}", e.id);
            }
        }
    }

    /// 联网实测：cargo test -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn 联网实测免费引擎() {
        let text = "Hello world, this is a\ntest of the translation filter.";
        for (id, proxy) in [("edge", None), ("bing", None), ("google_free", Some("http://127.0.0.1:7897"))] {
            let cfg = ProviderConfig {
                proxy: proxy.map(str::to_string),
                ..Default::default()
            };
            let r = create(id, &cfg).unwrap().translate(text, "zh-CN").await;
            println!("{id}: {r:?}");
            assert!(r.is_ok(), "{id} 失败");
        }
    }
}
