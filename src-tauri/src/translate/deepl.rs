//! DeepL 官方 API（v2）
//!
//! 免费版 key 以 `:fx` 结尾，走 api-free.deepl.com；付费版走 api.deepl.com。

use super::{TranslateError, Translator, build_client, is_traditional, merge_lines, parse_err, primary, require_key, send_json};
use crate::config::ProviderConfig;
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct DeepL {
    client: reqwest::Client,
    url: String,
    key: String,
}

impl DeepL {
    pub fn new(cfg: &ProviderConfig) -> Result<Self, TranslateError> {
        let key = require_key(cfg, "DeepL 需要注册 DeepL API（免费版每月 50 万字符）获取 key。")?;
        let base = cfg.base_url.trim().trim_end_matches('/');
        let base = if !base.is_empty() {
            base.to_string()
        } else if key.ends_with(":fx") {
            "https://api-free.deepl.com".into()
        } else {
            "https://api.deepl.com".into()
        };
        Ok(Self {
            client: build_client(cfg)?,
            url: format!("{base}/v2/translate"),
            key,
        })
    }
}

/// DeepL 目标语言代码：中文要区分简繁，英文/葡萄牙文必须带地区
fn to_deepl_lang(lang: &str) -> String {
    match primary(lang).as_str() {
        "zh" if is_traditional(lang) => "ZH-HANT".into(),
        "zh" => "ZH-HANS".into(),
        "en" if lang.to_ascii_lowercase().contains("gb") => "EN-GB".into(),
        "en" => "EN-US".into(),
        "pt" if lang.to_ascii_lowercase().contains("pt-pt") => "PT-PT".into(),
        "pt" => "PT-BR".into(),
        p => p.to_ascii_uppercase(),
    }
}

#[async_trait]
impl Translator for DeepL {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError> {
        let req = self
            .client
            .post(&self.url)
            .header("Authorization", format!("DeepL-Auth-Key {}", self.key))
            .json(&json!({
                "text": [merge_lines(text)],
                "target_lang": to_deepl_lang(target_lang),
            }));
        let v = send_json(req, "/message").await?;
        v.pointer("/translations/0/text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| parse_err(&v))
    }
}
