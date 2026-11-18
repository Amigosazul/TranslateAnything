//! 微软翻译官方 API（Azure AI Translator v3）
//!
//! 免费层 F0：每月 200 万字符。在 Azure 门户创建 Translator 资源后获得 key 与区域。

use super::{TranslateError, Translator, build_client, ms_lang, merge_lines, parse_err, require_key, send_json};
use crate::config::ProviderConfig;
use async_trait::async_trait;
use serde_json::{Value, json};

pub(crate) const DEFAULT_BASE: &str = "https://api.cognitive.microsofttranslator.com";

pub struct Microsoft {
    client: reqwest::Client,
    base: String,
    key: String,
    region: String,
}

impl Microsoft {
    pub fn new(cfg: &ProviderConfig) -> Result<Self, TranslateError> {
        let key = require_key(
            cfg,
            &t!("err.engine.microsoft_key_hint"),
        )?;
        let base = cfg.base_url.trim().trim_end_matches('/');
        Ok(Self {
            client: build_client(cfg)?,
            base: if base.is_empty() { DEFAULT_BASE.into() } else { base.into() },
            key,
            region: cfg.region.trim().to_string(),
        })
    }
}


#[async_trait]
impl Translator for Microsoft {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError> {
        let url = format!("{}/translate?api-version=3.0&to={}", self.base, ms_lang(target_lang));
        let mut req = self
            .client
            .post(url)
            .header("Ocp-Apim-Subscription-Key", &self.key)
            .json(&json!([{ "Text": merge_lines(text) }]));
        if !self.region.is_empty() {
            req = req.header("Ocp-Apim-Subscription-Region", &self.region);
        }
        let v = send_json(req, "/error/message").await?;
        v.pointer("/0/translations/0/text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| parse_err(&v))
    }
}
