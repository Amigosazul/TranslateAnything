//! 译文本地持久缓存：`~/.translate-filter/translations.db`（SQLite）
//!
//! 翻译过的文字存进本地库，下次遇到相同原文（同一目标语言）直接查库，不再请求翻译引擎。
//! 悬浮窗模式和沉浸模式共用这一份缓存。
//!
//! - 键是（目标语言，原文），不区分引擎：换引擎后旧译文仍然有效，想重翻可以清空缓存。
//! - `manual` 标记用户手工修改过的译文，为后续设置界面的"词典/改译"功能预留，自动翻译不会覆盖它。
//! - 打不开数据库时（磁盘只读、文件损坏等）静默降级为"不缓存"，不影响翻译本身。

use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 单条原文超过这个长度就不缓存（多半是整段正文，复用率低）
const MAX_SOURCE_CHARS: usize = 4000;

static STORE: OnceLock<Option<Mutex<Connection>>> = OnceLock::new();

pub fn db_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".translate-filter").join("translations.db"))
}

fn open(path: &PathBuf) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    init(&conn)?;
    Ok(conn)
}

fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS translations (
             lang       TEXT    NOT NULL,
             source     TEXT    NOT NULL,
             target     TEXT    NOT NULL,
             engine     TEXT    NOT NULL DEFAULT '',
             manual     INTEGER NOT NULL DEFAULT 0,
             hits       INTEGER NOT NULL DEFAULT 0,
             created_at INTEGER NOT NULL,
             used_at    INTEGER NOT NULL,
             PRIMARY KEY (lang, source)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS idx_translations_used ON translations (lang, used_at DESC);",
    )
}

fn conn() -> Option<&'static Mutex<Connection>> {
    STORE
        .get_or_init(|| {
            let path = db_path()?;
            match open(&path) {
                Ok(c) => Some(Mutex::new(c)),
                Err(e) => {
                    eprintln!("打开译文缓存失败，本次运行不做持久缓存：{}\n{e}", path.display());
                    None
                }
            }
        })
        .as_ref()
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// 批量查询，返回命中的（原文 → 译文），同时累计命中次数
pub fn get_many(lang: &str, sources: &[String]) -> HashMap<String, String> {
    let Some(c) = conn() else { return HashMap::new() };
    let c = c.lock().unwrap();
    get_many_in(&c, lang, sources).unwrap_or_else(|e| {
        eprintln!("查询译文缓存失败：{e}");
        HashMap::new()
    })
}

fn get_many_in(c: &Connection, lang: &str, sources: &[String]) -> rusqlite::Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    let mut get = c.prepare_cached("SELECT target FROM translations WHERE lang = ?1 AND source = ?2")?;
    let mut touch =
        c.prepare_cached("UPDATE translations SET hits = hits + 1, used_at = ?3 WHERE lang = ?1 AND source = ?2")?;
    let t = now();
    for s in sources {
        if out.contains_key(s) {
            continue;
        }
        if let Some(target) = get.query_row(params![lang, s], |r| r.get::<_, String>(0)).optional()? {
            touch.execute(params![lang, s, t])?;
            out.insert(s.clone(), target);
        }
    }
    Ok(out)
}

/// 批量写入自动翻译结果；用户手工改过的条目不覆盖
pub fn put_many(lang: &str, pairs: &[(String, String)], engine: &str) {
    let Some(c) = conn() else { return };
    let mut c = c.lock().unwrap();
    if let Err(e) = put_many_in(&mut c, lang, pairs, engine) {
        eprintln!("写入译文缓存失败：{e}");
    }
}

fn put_many_in(c: &mut Connection, lang: &str, pairs: &[(String, String)], engine: &str) -> rusqlite::Result<()> {
    let tx = c.transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO translations (lang, source, target, engine, created_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT (lang, source) DO UPDATE
                 SET target = excluded.target, engine = excluded.engine, used_at = excluded.used_at
                 WHERE manual = 0",
        )?;
        let t = now();
        for (s, tr) in pairs {
            // 空译文不存；与原文相同的照存（产品名、专有名词），免得每次都去问引擎
            if s.chars().count() > MAX_SOURCE_CHARS || tr.trim().is_empty() {
                continue;
            }
            stmt.execute(params![lang, s, tr, engine, t])?;
        }
    }
    tx.commit()
}

/// 最近常用的若干条译文，供沉浸模式注入页面时预加载（避免首屏先闪一下原文）
pub fn recent(lang: &str, limit: usize) -> Vec<(String, String)> {
    let Some(c) = conn() else { return Vec::new() };
    let c = c.lock().unwrap();
    let r = (|| -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = c.prepare_cached(
            "SELECT source, target FROM translations WHERE lang = ?1 ORDER BY used_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![lang, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    })();
    r.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        c
    }

    #[test]
    fn 写入后命中() {
        let mut c = mem();
        let pairs = vec![
            ("Settings".to_string(), "设置".to_string()),
            ("Claude".to_string(), "Claude".to_string()),
            ("Empty".to_string(), " ".to_string()),
        ];
        put_many_in(&mut c, "zh-CN", &pairs, "edge").unwrap();
        let hit = get_many_in(&c, "zh-CN", &["Settings".into(), "Claude".into(), "Empty".into(), "Other".into()]).unwrap();
        assert_eq!(hit.len(), 2, "空译文不入库，原样返回的照存");
        assert_eq!(hit["Settings"], "设置");
        // 不同目标语言互不干扰
        assert!(get_many_in(&c, "ja", &["Settings".into()]).unwrap().is_empty());
    }

    #[test]
    fn 手工译文不被覆盖() {
        let mut c = mem();
        put_many_in(&mut c, "zh-CN", &[("Run".into(), "运行".into())], "edge").unwrap();
        c.execute("UPDATE translations SET target = '执行', manual = 1", []).unwrap();
        put_many_in(&mut c, "zh-CN", &[("Run".into(), "跑".into())], "edge").unwrap();
        assert_eq!(get_many_in(&c, "zh-CN", &["Run".into()]).unwrap()["Run"], "执行");
    }
}
