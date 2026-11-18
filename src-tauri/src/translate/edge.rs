//! 免 key 的微软翻译（非官方接口，随时可能失效）
//!
//! - 主路：Edge 浏览器内置翻译 `edge.microsoft.com/translate/translatetext`，无需鉴权
//!   （2026-08 起 `translate/auth` 已 404，kiss-translator 改用此端点）
//! - 回退：Bing 网页版 `/ttranslatev3`，先从 `/translator` 页面抓 IG / IID / key / token
//!   （参考 Easydict、translate-shell）。token 用到有效期一半即刷新，返回 205 视为失效重取。

use super::{
    BROWSER_UA, TranslateError, Translator, build_client, form_encode, merge_lines, ms_lang,
    network_err, parse_err, send_json,
};
use crate::config::ProviderConfig;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const EDGE_URL: &str = "https://edge.microsoft.com/translate/translatetext";
const BING_HOME: &str = "https://cn.bing.com/translator";

/// 走哪条路
#[derive(Clone, Copy, PartialEq)]
pub enum Route {
    /// Edge 接口优先，失败回退 Bing 网页
    EdgeThenBing,
    /// 只用 Bing 网页
    BingOnly,
}

pub struct Edge {
    client: reqwest::Client,
    route: Route,
}

impl Edge {
    pub fn new(cfg: &ProviderConfig, route: Route) -> Result<Self, TranslateError> {
        Ok(Self {
            client: build_client(cfg)?,
            route,
        })
    }

    async fn edge(&self, texts: &[String], to: &str) -> Result<Vec<String>, TranslateError> {
        let url = format!("{EDGE_URL}?from=&to={to}&isEnterpriseClient=false");
        let req = self
            .client
            .post(url)
            .header("User-Agent", BROWSER_UA)
            .json(&json!(texts));
        let v = send_json(req, "/error/message").await?;
        let items = v.as_array().ok_or_else(|| parse_err(&v))?;
        items
            .iter()
            .map(|item| {
                item.pointer("/translations/0/text")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .ok_or_else(|| parse_err(&v))
            })
            .collect()
    }

    async fn bing(&self, text: &str, to: &str) -> Result<String, TranslateError> {
        // 缓存的 token 被判失效时重取一次
        for attempt in 0..2 {
            let token = self.bing_token(attempt > 0).await?;
            let url = format!(
                "https://{}/ttranslatev3?isVertical=1&IG={}&IID={}",
                token.host, token.ig, token.iid
            );
            let body = form_encode(&[
                ("fromLang", "auto-detect"),
                ("to", to),
                ("text", text),
                ("token", &token.token),
                ("key", &token.key),
            ]);
            let resp = self
                .client
                .post(url)
                .header("User-Agent", BROWSER_UA)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await
                .map_err(network_err)?;
            let status = resp.status();
            let raw = resp.text().await.map_err(network_err)?;
            let v: Value = serde_json::from_str(&raw).map_err(|_| {
                TranslateError::Api(format!("Bing 返回 HTTP {status}：{}", raw.chars().take(200).collect::<String>()))
            })?;
            if v.get("statusCode").and_then(Value::as_i64) == Some(205) {
                *BING_TOKEN.lock().unwrap() = None;
                continue;
            }
            if !status.is_success() {
                return Err(TranslateError::Api(format!("Bing 返回 HTTP {status}")));
            }
            return join_translations(&v);
        }
        Err(TranslateError::Api("Bing token 反复失效".into()))
    }

    async fn bing_token(&self, force: bool) -> Result<BingToken, TranslateError> {
        if !force {
            if let Some(t) = BING_TOKEN.lock().unwrap().as_ref() {
                if t.fetched.elapsed() < t.ttl / 2 {
                    return Ok(t.clone());
                }
            }
        }
        let resp = self
            .client
            .get(BING_HOME)
            .header("User-Agent", BROWSER_UA)
            .send()
            .await
            .map_err(network_err)?;
        // 可能被重定向到 www.bing.com 等，后续请求要用最终的 host
        let host = resp.url().host_str().unwrap_or("cn.bing.com").to_string();
        let page = resp.text().await.map_err(network_err)?;
        let token = parse_bing_page(&page, host)
            .ok_or_else(|| TranslateError::Api("无法从 Bing 翻译页面解析 token（页面结构可能已变化）".into()))?;
        *BING_TOKEN.lock().unwrap() = Some(token.clone());
        Ok(token)
    }
}

#[derive(Clone)]
struct BingToken {
    host: String,
    ig: String,
    iid: String,
    key: String,
    token: String,
    fetched: Instant,
    ttl: Duration,
}

static BING_TOKEN: Mutex<Option<BingToken>> = Mutex::new(None);

/// 截取 `start` 之后、`end` 之前的内容
fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let j = s[i..].find(end)? + i;
    Some(&s[i..j])
}

/// 从 `/translator` 页面抓 `IG:"..."`、`data-iid="..."`、`params_AbusePreventionHelper = [key,"token",ttl]`
fn parse_bing_page(page: &str, host: String) -> Option<BingToken> {
    let ig = between(page, "IG:\"", "\"")?.to_string();
    let iid = between(page, "data-iid=\"", "\"")?.to_string();
    let helper = between(page, "params_AbusePreventionHelper = [", "]")?;
    let mut parts = helper.splitn(3, ',');
    let key = parts.next()?.trim().to_string();
    let token = parts.next()?.trim().trim_matches('"').to_string();
    let ttl_ms: u64 = parts.next().and_then(|s| s.trim().parse().ok()).unwrap_or(3_600_000);
    if ig.is_empty() || key.is_empty() || token.is_empty() {
        return None;
    }
    Some(BingToken {
        host,
        ig,
        iid,
        key,
        token,
        fetched: Instant::now(),
        ttl: Duration::from_millis(ttl_ms),
    })
}

/// 两个接口的响应格式相同：`[{"translations":[{"text":...}]}]`
fn join_translations(v: &Value) -> Result<String, TranslateError> {
    v.pointer("/0/translations/0/text")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .ok_or_else(|| parse_err(v))
}

#[async_trait]
impl Translator for Edge {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError> {
        let mut v = self.translate_batch(&[text.to_string()], target_lang).await?;
        Ok(v.pop().unwrap_or_default())
    }

    async fn translate_batch(&self, texts: &[String], target_lang: &str) -> Result<Vec<String>, TranslateError> {
        let texts: Vec<String> = texts.iter().map(|t| merge_lines(t)).collect();
        let to = ms_lang(target_lang);
        if self.route == Route::EdgeThenBing {
            match self.edge(&texts, &to).await {
                Ok(v) if v.len() == texts.len() => return Ok(v),
                Ok(_) => {}
                Err(edge_err) => {
                    return self.bing_all(&texts, &to).await.map_err(|bing_err| {
                        TranslateError::Api(format!("Edge 接口：{edge_err}；Bing 网页回退：{bing_err}"))
                    });
                }
            }
        }
        self.bing_all(&texts, &to).await
    }
}

impl Edge {
    /// Bing 网页接口一次只能翻一条
    async fn bing_all(&self, texts: &[String], to: &str) -> Result<Vec<String>, TranslateError> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.bing(t, to).await?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 解析bing页面() {
        let page = r#"xx IG:"ABC123",y data-iid="translator.5023" z
            var params_AbusePreventionHelper = [1794823013973,"tok_en-1",3600000];"#;
        let t = parse_bing_page(page, "cn.bing.com".into()).unwrap();
        assert_eq!(t.ig, "ABC123");
        assert_eq!(t.iid, "translator.5023");
        assert_eq!(t.key, "1794823013973");
        assert_eq!(t.token, "tok_en-1");
        assert_eq!(t.ttl, Duration::from_secs(3600));
    }
}
