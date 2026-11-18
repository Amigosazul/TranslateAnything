//! OpenAI 兼容协议：POST `{base_url}/chat/completions`

use super::{TranslateError, Translator, build_client, parse_err, send_json};
use crate::config::ProviderConfig;
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct OpenAICompatible {
    client: reqwest::Client,
    url: String,
    api_key: String,
    model: String,
}

impl OpenAICompatible {
    pub fn new(cfg: &ProviderConfig) -> Result<Self, TranslateError> {
        let base = cfg.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err(TranslateError::Config(t!("err.engine.no_base_url")));
        }
        if cfg.model.trim().is_empty() {
            return Err(TranslateError::Config(t!("err.engine.no_model")));
        }
        Ok(Self {
            client: build_client(cfg)?,
            url: format!("{base}/chat/completions"),
            api_key: cfg.api_key.trim().to_string(),
            model: cfg.model.trim().to_string(),
        })
    }
}

fn system_prompt(target_lang: &str) -> String {
    format!(
        "你是专业的翻译引擎。请把用户发来的文本翻译成目标语言（语言代码：{target_lang}）。\
         文本来自屏幕截图的 OCR 结果，可能存在被硬换行打断的句子和个别识别错误：\
         请按语义合并被打断的句子、保留原有的段落和列表结构、修正明显的识别错误。\
         如果文本本身已经是目标语言，原样输出。\
         只输出译文，不要任何解释、前后缀或引号。"
    )
}

#[async_trait]
impl Translator for OpenAICompatible {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError> {
        // 大模型自己能处理断行，直接传原始 OCR 文本
        let mut req = self.client.post(&self.url).json(&json!({
            "model": self.model,
            "temperature": 0.2,
            "messages": [
                { "role": "system", "content": system_prompt(target_lang) },
                { "role": "user", "content": text },
            ],
        }));
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let v = send_json(req, "/error/message").await?;
        v.pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .ok_or_else(|| parse_err(&v))
    }
}
