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
use std::fmt;
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
            Self::Config(m) => write!(f, "配置错误：{m}"),
            Self::Network(m) => write!(f, "网络错误：{m}"),
            Self::Api(m) => write!(f, "服务返回错误：{m}"),
        }
    }
}

#[async_trait]
pub trait Translator: Send + Sync {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError>;
    // 扩展点（P2 视觉直译）：translate_image(&self, png: &[u8], target_lang)，默认返回"不支持"
}

// ───────────────────────── 引擎清单 ─────────────────────────

/// 引擎元信息，供设置界面渲染（M3）
#[derive(Debug, Serialize)]
pub struct EngineInfo {
    /// 即配置里的 `type`
    pub id: &'static str,
    pub name: &'static str,
    /// 是否必须填写 api_key
    pub needs_key: bool,
    /// 是否为官方公开 API（否则是非官方免费接口，随时可能失效）
    pub official: bool,
    /// 该引擎会用到的配置字段
    pub fields: &'static [&'static str],
    pub note: &'static str,
}

pub const CATALOG: &[EngineInfo] = &[
    EngineInfo {
        id: "edge",
        name: "微软翻译（免费）",
        needs_key: false,
        official: false,
        fields: &["proxy"],
        note: "Edge 浏览器内置翻译接口，失败自动回退 Bing 网页版；国内可直连",
    },
    EngineInfo {
        id: "bing",
        name: "Bing 网页翻译（免费）",
        needs_key: false,
        official: false,
        fields: &["proxy"],
        note: "Bing 网页版接口；国内可直连",
    },
    EngineInfo {
        id: "google_free",
        name: "Google 翻译（免费）",
        needs_key: false,
        official: false,
        fields: &["base_url", "proxy"],
        note: "国内需配置代理；调用频繁会被限流。base_url 可改为镜像",
    },
    EngineInfo {
        id: "microsoft",
        name: "微软翻译（官方 API）",
        needs_key: true,
        official: true,
        fields: &["api_key", "region", "base_url", "proxy"],
        note: "Azure Translator 资源，免费层每月 200 万字符",
    },
    EngineInfo {
        id: "deepl",
        name: "DeepL（官方 API）",
        needs_key: true,
        official: true,
        fields: &["api_key", "base_url", "proxy"],
        note: "免费版每月 50 万字符，:fx 结尾的 key 自动走免费端点",
    },
    EngineInfo {
        id: "deeplx",
        name: "DeepLX（自建）",
        needs_key: false,
        official: false,
        fields: &["base_url", "api_key", "proxy"],
        note: "自建 DeepLX 服务地址，api_key 为可选的访问令牌",
    },
    EngineInfo {
        id: "google",
        name: "Google Cloud 翻译（官方 API）",
        needs_key: true,
        official: true,
        fields: &["api_key", "base_url", "proxy"],
        note: "需启用 Cloud Translation API；国内需配置代理",
    },
    EngineInfo {
        id: "openai",
        name: "大模型（OpenAI 兼容）",
        needs_key: true,
        official: true,
        fields: &["base_url", "api_key", "model", "proxy"],
        note: "任意 OpenAI 兼容服务，译文更通顺但较慢",
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
            return Err(TranslateError::Config(format!(
                "不支持的引擎类型：{other}（可选 {}）",
                ids.join(" / ")
            )));
        }
    })
}

fn engine_name(kind: &str) -> &str {
    CATALOG.iter().find(|e| e.id == kind).map_or(kind, |e| e.name)
}

/// 按配置的主引擎 + 回退顺序依次尝试，返回（译文，实际使用的引擎名）
pub async fn translate_with_fallback(cfg: &Config, text: &str) -> Result<(String, String), String> {
    let mut errors = Vec::new();
    for (id, provider) in cfg.engine_chain() {
        let kind = provider.kind_or(&id);
        let result = match create(&kind, &provider) {
            Ok(t) => t.translate(text, &cfg.target_lang).await,
            Err(e) => Err(e),
        };
        match result {
            Ok(t) => return Ok((t, engine_name(&kind).to_string())),
            Err(e) => errors.push(format!("【{}】{e}", engine_name(&kind))),
        }
    }
    Err(errors.join("\n"))
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
            reqwest::Proxy::all(p).map_err(|e| TranslateError::Config(format!("代理地址无效：{e}")))?,
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
        return Err(TranslateError::Config(format!("未填写 api_key。{hint}")));
    }
    Ok(key.to_string())
}

fn network_err(e: reqwest::Error) -> TranslateError {
    let host = e.url().and_then(|u| u.host_str()).unwrap_or("").to_string();
    let mut msg = if e.is_timeout() { "请求超时".to_string() } else { e.to_string() };
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
    value.ok_or_else(|| TranslateError::Api(format!("响应不是 JSON：{}", raw.chars().take(300).collect::<String>())))
}

fn parse_err(v: &Value) -> TranslateError {
    TranslateError::Api(format!("无法解析响应：{}", v.to_string().chars().take(300).collect::<String>()))
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
    fn 清单覆盖所有引擎() {
        let cfg = ProviderConfig::default();
        for e in CATALOG {
            // 只要不是"不支持的类型"即可（缺 key 之类的配置错误是预期的）
            if let Err(TranslateError::Config(m)) = create(e.id, &cfg) {
                assert!(!m.contains("不支持"), "{}: {m}", e.id);
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
