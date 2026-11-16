//! 配置读写：`~/.translate-filter/config.json`
//!
//! 每次翻译时重新读取，修改配置文件后无需重启即可生效（快捷键除外）。
//!
//! 翻译引擎可同时配置多个（`engines`），用 `engine` 指定主用、`fallback` 指定失败后的回退顺序，
//! 以后的设置界面只需要读写这几个字段。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 全局快捷键，如 `Alt+T`（启动时读取，修改后需重启）
    pub hotkey: String,
    /// 目标语言，如 `zh-CN`
    pub target_lang: String,
    /// 翻译模式：`ocr`（第一版只支持这个）/ `vision`（预留）
    pub mode: String,
    /// OCR 语言标签，如 `en-US`；`auto` 表示自动挑选
    pub ocr_lang: String,
    /// 主用引擎 id（`engines` 里的键）
    pub engine: String,
    /// 主引擎失败后依次尝试的引擎 id
    pub fallback: Vec<String>,
    /// 各引擎参数，键为引擎 id；未出现的引擎按默认参数使用
    pub engines: BTreeMap<String, ProviderConfig>,
    /// 旧版单引擎写法，仅为兼容；有 `engines` 时忽略
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    /// 引擎类型（见 translate::CATALOG）；留空时取 `engines` 里的键名。
    /// 同一类型配置多份时用得上，例如两个不同的 openai 兼容服务。
    #[serde(rename = "type", skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// 接口地址；多数引擎可留空用默认地址
    pub base_url: String,
    pub api_key: String,
    /// 微软翻译官方 API 的资源区域，如 `eastasia`
    pub region: String,
    /// 仅 openai 使用
    pub model: String,
    /// 代理：留空/不填 = 直连；`system` = 跟随系统环境变量；其他 = 代理地址
    pub proxy: Option<String>,
}

impl ProviderConfig {
    pub fn kind_or(&self, id: &str) -> String {
        let k = self.kind.trim();
        if k.is_empty() { id.to_string() } else { k.to_string() }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "Alt+T".into(),
            target_lang: "zh-CN".into(),
            mode: "ocr".into(),
            ocr_lang: "auto".into(),
            engine: "edge".into(),
            fallback: Vec::new(),
            engines: BTreeMap::new(),
            provider: None,
        }
    }
}

impl Config {
    /// 按尝试顺序返回（引擎 id，参数），已去重
    pub fn engine_chain(&self) -> Vec<(String, ProviderConfig)> {
        // 兼容旧版：只写了 provider
        if self.engines.is_empty() {
            if let Some(p) = &self.provider {
                let id = p.kind_or("openai");
                return vec![(id, p.clone())];
            }
        }
        let mut ids: Vec<&str> = vec![self.engine.trim()];
        ids.extend(self.fallback.iter().map(|s| s.trim()));
        let mut seen = Vec::new();
        let mut chain = Vec::new();
        for id in ids.into_iter().filter(|s| !s.is_empty()) {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            chain.push((id.to_string(), self.engines.get(id).cloned().unwrap_or_default()));
        }
        if chain.is_empty() {
            chain.push(("edge".into(), ProviderConfig::default()));
        }
        chain
    }
}

pub fn config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".translate-filter").join("config.json"))
}

/// 读取配置；文件不存在时返回默认配置（默认引擎免 key，开箱即用）
pub fn load() -> Result<Config, String> {
    let path = config_path().ok_or("无法定位用户目录")?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(format!("读取配置文件失败：{}\n{e}", path.display())),
    };
    serde_json::from_str(&text).map_err(|e| format!("配置文件格式错误：{}\n{e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 引擎链与回退() {
        let cfg: Config = serde_json::from_str(
            r#"{"engine":"microsoft","fallback":["edge","microsoft","google_free"],
                "engines":{"microsoft":{"api_key":"k","region":"eastasia"},"my-llm":{"type":"openai"}}}"#,
        )
        .unwrap();
        let chain: Vec<_> = cfg.engine_chain().into_iter().map(|(id, _)| id).collect();
        assert_eq!(chain, ["microsoft", "edge", "google_free"]);
        assert_eq!(cfg.engines["my-llm"].kind_or("my-llm"), "openai");
    }

    #[test]
    fn 兼容旧版provider写法() {
        let cfg: Config = serde_json::from_str(r#"{"provider":{"type":"openai","model":"m"}}"#).unwrap();
        let chain = cfg.engine_chain();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].0, "openai");
    }

    #[test]
    fn 空配置默认edge() {
        let chain = Config::default().engine_chain();
        assert_eq!(chain[0].0, "edge");
    }
}
