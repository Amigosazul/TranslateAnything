//! DeepLX：用户自建的 DeepL 免费翻译中转（github.com/OwO-Network/DeepLX）
//!
//! POST `{base_url}`，请求体 `{text, source_lang, target_lang}`，译文在 `data` 字段。

use super::{TranslateError, Translator, build_client, merge_lines, parse_err, primary, send_json};
use crate::config::ProviderConfig;
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct DeepLX {
    client: reqwest::Client,
    url: String,
    token: String,
}

impl DeepLX {
    pub fn new(cfg: &ProviderConfig) -> Result<Self, TranslateError> {
        let url = cfg.base_url.trim().to_string();
        if url.is_empty() {
            return Err(TranslateError::Config(
                "deeplx 需要在 provider.base_url 填写自建服务地址，如 http://127.0.0.1:1188/translate".into(),
            ));
        }
        Ok(Self {
            client: build_client(cfg)?,
            url,
            token: cfg.api_key.trim().to_string(),
        })
    }
}

#[async_trait]
impl Translator for DeepLX {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError> {
        let mut req = self.client.post(&self.url).json(&json!({
            "text": merge_lines(text),
            "source_lang": "auto",
            "target_lang": primary(target_lang).to_ascii_uppercase(),
        }));
        // DeepLX 可选的访问令牌
        if !self.token.is_empty() {
            req = req.bearer_auth(&self.token);
        }
        let v = send_json(req, "/message").await?;
        v.get("data")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .ok_or_else(|| parse_err(&v))
    }
}
