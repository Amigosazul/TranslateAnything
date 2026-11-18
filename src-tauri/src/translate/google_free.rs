//! 免 key 的 Google 翻译（`translate_a/single?client=gtx`，非官方接口）
//!
//! 国内需配置代理；频繁调用会被限流（429 / 人机验证页）。

use super::{BROWSER_UA, TranslateError, Translator, build_client, google_lang, merge_lines, network_err};
use crate::config::ProviderConfig;
use async_trait::async_trait;
use serde_json::Value;

pub(crate) const DEFAULT_BASE: &str = "https://translate.googleapis.com";
/// GET 请求的 URL 不宜过长
const MAX_CHARS: usize = 5000;

pub struct GoogleFree {
    client: reqwest::Client,
    base: String,
}

impl GoogleFree {
    pub fn new(cfg: &ProviderConfig) -> Result<Self, TranslateError> {
        // base_url 可改成镜像地址（参考 pot-desktop）
        let base = cfg.base_url.trim().trim_end_matches('/');
        let base = match base {
            "" => DEFAULT_BASE.to_string(),
            b if b.starts_with("http") => b.to_string(),
            b => format!("https://{b}"),
        };
        Ok(Self {
            client: build_client(cfg)?,
            base,
        })
    }
}

fn rate_limited() -> TranslateError {
    TranslateError::Api(t!("err.engine.google_rate_limited"))
}

#[async_trait]
impl Translator for GoogleFree {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError> {
        let text = merge_lines(text);
        if text.chars().count() > MAX_CHARS {
            return Err(TranslateError::Api(t!("err.engine.too_long", max = MAX_CHARS)));
        }
        let tl = google_lang(target_lang);
        let url = reqwest::Url::parse_with_params(
            &format!("{}/translate_a/single", self.base),
            [
                ("client", "gtx"),
                ("dt", "t"),
                ("dj", "1"),
                ("ie", "UTF-8"),
                ("oe", "UTF-8"),
                ("sl", "auto"),
                ("tl", tl.as_str()),
                ("q", text.as_str()),
            ],
        )
        .map_err(|e| TranslateError::Config(t!("err.engine.bad_base_url", e = e)))?;

        let resp = self
            .client
            .get(url)
            .header("User-Agent", BROWSER_UA)
            .send()
            .await
            .map_err(network_err)?;
        let status = resp.status();
        if status.as_u16() == 429 {
            return Err(rate_limited());
        }
        let raw = resp.text().await.map_err(network_err)?;
        let v: Value = serde_json::from_str(&raw).map_err(|_| {
            if raw.contains("sorry") || raw.contains("captcha") {
                rate_limited()
            } else {
                TranslateError::Api(format!("HTTP {status}：{}", raw.chars().take(200).collect::<String>()))
            }
        })?;

        // dj=1 时响应为 {"sentences":[{"trans":"..."}, ...]}，逐句拼接
        let sentences = v
            .get("sentences")
            .and_then(Value::as_array)
            .ok_or_else(|| super::parse_err(&v))?;
        let out: String = sentences
            .iter()
            .filter_map(|s| s.get("trans").and_then(Value::as_str))
            .collect();
        Ok(out.trim().to_string())
    }
}
