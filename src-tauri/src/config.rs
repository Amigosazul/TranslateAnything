//! 配置读写：`~/.translate-anything/config.json`
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
    /// 译文写入本地库（`~/.translate-anything/translations.db`），相同原文下次直接查库不再请求引擎
    pub persist_cache: bool,
    /// 译文库所在文件夹；留空 = 数据目录。请在设置窗口里修改（会连同文件一起搬过去）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub store_dir: String,
    /// 使用内置的界面常用词术语表（如 New → 新建），避免引擎翻单个词时没有上下文而翻错
    pub builtin_glossary: bool,
    /// 自定义术语表：原文 → 译文，整条精确匹配，优先于内置术语表、译文库和引擎。
    /// 译文写成与原文相同即"保持原样不翻"
    pub glossary: BTreeMap<String, String>,
    /// 沉浸模式：直接改写目标应用界面里的文字（目前支持 Electron/Chromium 内核应用）
    pub immersive: ImmersiveConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImmersiveConfig {
    /// 总开关
    pub enabled: bool,
    /// 在"译文 / 原文"之间切换的全局快捷键（启动时读取）
    pub hotkey: String,
    /// 要接管的应用；应用需要带 `--remote-debugging-port=<port>` 启动
    pub targets: Vec<ImmersiveTarget>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImmersiveTarget {
    /// 显示名
    pub name: String,
    /// 是否接管这个应用（单独开关）
    pub enabled: bool,
    /// 程序完整路径。用于身份校验（调试端口背后必须是这个程序才接管）和"以沉浸模式启动"
    pub exe: String,
    /// 启动参数（不含调试端口）。"以沉浸模式启动"时使用；应用在运行时会自动从它的命令行读取并记下
    pub args: String,
    /// 远程调试端口（只监听本机 127.0.0.1），各应用不能重复
    pub port: u16,
    /// 只处理地址以这些前缀开头的页面；留空 = 该应用的所有页面
    pub url_prefixes: Vec<String>,
    /// 额外不翻译的元素（CSS 选择器），如聊天正文、代码区
    pub skip_selectors: Vec<String>,
}

impl Default for ImmersiveTarget {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            exe: String::new(),
            args: String::new(),
            port: 0,
            url_prefixes: Vec::new(),
            skip_selectors: Vec::new(),
        }
    }
}

impl Default for ImmersiveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hotkey: "Alt+Shift+T".into(),
            targets: vec![ImmersiveTarget {
                name: "reclaude 客户端".into(),
                exe: dirs::home_dir()
                    .map(|h| h.join(".rec").join("claude-app").join("app").join("claude.exe").to_string_lossy().into_owned())
                    .unwrap_or_default(),
                port: 39222,
                ..Default::default()
            }],
        }
    }
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
            persist_cache: true,
            store_dir: String::new(),
            builtin_glossary: true,
            glossary: BTreeMap::new(),
            immersive: ImmersiveConfig::default(),
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

/// 用户数据目录名（配置、译文库、沉浸模式启动参数备份都放这里）
const DATA_DIR: &str = ".translate-anything";
/// 改名前（translate-filter 时期）的数据目录名，启动时自动迁移
const LEGACY_DATA_DIR: &str = ".translate-filter";

/// 用户数据目录 `~/.translate-anything`
///
/// 首次调用时：新目录不存在而旧目录 `~/.translate-filter` 存在，就把旧目录整个改名过来
/// （配置、译文库、术语表等原样保留）。改名失败（如旧版程序还开着、文件被占用）时继续用旧目录，
/// 下次启动再试。
pub fn data_dir() -> Option<PathBuf> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let home = dirs::home_dir()?;
        let dir = home.join(DATA_DIR);
        let legacy = home.join(LEGACY_DATA_DIR);
        if !dir.exists() && legacy.is_dir() {
            match std::fs::rename(&legacy, &dir) {
                Ok(()) => println!("已将旧数据目录 {} 迁移到 {}", legacy.display(), dir.display()),
                Err(e) => {
                    eprintln!("迁移旧数据目录 {} 失败，本次继续使用旧目录：{e}", legacy.display());
                    return Some(legacy);
                }
            }
        }
        Some(dir)
    })
    .clone()
}

pub fn config_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("config.json"))
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

/// 检查配置是否可以保存（端口重复等）
pub fn validate(cfg: &Config) -> Result<(), String> {
    let mut ports = std::collections::HashSet::new();
    for t in &cfg.immersive.targets {
        if t.name.trim().is_empty() {
            return Err("沉浸模式：应用名称不能为空".into());
        }
        if t.port < 1024 {
            return Err(format!("沉浸模式：「{}」的调试端口必须在 1024 到 65535 之间", t.name));
        }
        if !ports.insert(t.port) {
            return Err(format!("沉浸模式：调试端口 {} 被多个应用重复使用", t.port));
        }
    }
    if cfg.target_lang.trim().is_empty() {
        return Err("目标语言不能为空".into());
    }
    Ok(())
}

/// 保存配置：先写临时文件再替换，写到一半出错不会损坏原文件
pub fn save(cfg: &Config) -> Result<(), String> {
    validate(cfg)?;
    let path = config_path().ok_or("无法定位用户目录")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败：{e}"))?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("写入配置失败：{e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("替换配置文件失败：{e}"))
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
    fn 校验端口重复() {
        let mut cfg = Config::default();
        assert!(validate(&cfg).is_ok());
        let mut t = cfg.immersive.targets[0].clone();
        t.name = "另一个".into();
        cfg.immersive.targets.push(t);
        assert!(validate(&cfg).unwrap_err().contains("重复"));
    }

    #[test]
    fn 旧配置补齐新字段() {
        let cfg: Config = serde_json::from_str(r#"{"immersive":{"targets":[{"name":"a","port":40000}]}}"#).unwrap();
        assert!(cfg.immersive.targets[0].enabled, "缺省为启用");
        assert!(cfg.immersive.enabled);
    }

    #[test]
    fn 空配置默认edge() {
        let chain = Config::default().engine_chain();
        assert_eq!(chain[0].0, "edge");
    }
}
