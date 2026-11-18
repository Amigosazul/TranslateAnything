//! Google Cloud Translation 官方 API（Basic v2）
//!
//! 需要在 Google Cloud 控制台启用 Cloud Translation API 并创建 API key；国内需配置代理。

use super::{TranslateError, Translator, build_client, google_lang, merge_lines, parse_err, require_key, send_json};
use crate::config::ProviderConfig;
use async_trait::async_trait;
use serde_json::{Value, json};

const DEFAULT_BASE: &str = "https://translation.googleapis.com";

pub struct Google {
    client: reqwest::Client,
    url: String,
    key: String,
}

impl Google {
    pub fn new(cfg: &ProviderConfig) -> Result<Self, TranslateError> {
        let key = require_key(cfg, &t!("err.engine.google_key_hint"))?;
        let base = cfg.base_url.trim().trim_end_matches('/');
        let base = if base.is_empty() { DEFAULT_BASE } else { base };
        Ok(Self {
            client: build_client(cfg)?,
            url: format!("{base}/language/translate/v2"),
            key,
        })
    }
}


#[async_trait]
impl Translator for Google {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError> {
        let req = self
            .client
            .post(&self.url)
            // key 放请求头，避免出现在 URL 里
            .header("X-goog-api-key", &self.key)
            .json(&json!({
                "q": merge_lines(text),
                "target": google_lang(target_lang),
                "format": "text",
            }));
        let v = send_json(req, "/error/message").await?;
        v.pointer("/data/translations/0/translatedText")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| parse_err(&v))
    }
}
