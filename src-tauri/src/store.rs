//! 译文本地持久缓存：`~/.translate-anything/translations.db`（SQLite）
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
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// 单条原文超过这个长度就不缓存（多半是整段正文，复用率低）
const MAX_SOURCE_CHARS: usize = 4000;

/// 数据库文件名
const DB_FILE: &str = "translations.db";

/// 连接状态：第一次用到时才打开；打不开就记成失败，本次运行不再反复尝试（迁移位置后会重置）
enum Slot {
    Unopened,
    Open(Connection),
    Failed,
}

static STORE: Mutex<Slot> = Mutex::new(Slot::Unopened);

/// 译文库所在文件夹：配置了 `store_dir` 用它，否则用数据目录
pub fn db_dir() -> Option<PathBuf> {
    let custom = crate::config::load().map(|c| c.store_dir).unwrap_or_default();
    let custom = custom.trim();
    if custom.is_empty() { crate::config::data_dir() } else { Some(PathBuf::from(custom)) }
}

pub fn db_path() -> Option<PathBuf> {
    db_dir().map(|d| d.join(DB_FILE))
}

/// 数据库文件及其预写日志的总大小（字节）
pub fn db_size() -> u64 {
    let Some(p) = db_path() else { return 0 };
    ["", "-wal"]
        .iter()
        .filter_map(|ext| std::fs::metadata(format!("{}{ext}", p.display())).ok())
        .map(|m| m.len())
        .sum()
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

/// 在译文库连接上执行 f；库打不开时返回 None
fn with_db<T>(f: impl FnOnce(&mut Connection) -> T) -> Option<T> {
    let mut slot = STORE.lock().unwrap();
    if matches!(*slot, Slot::Unopened) {
        *slot = match db_path() {
            Some(path) => match open(&path) {
                Ok(c) => Slot::Open(c),
                Err(e) => {
                    eprintln!("打开译文缓存失败，本次运行不做持久缓存：{}\n{e}", path.display());
                    Slot::Failed
                }
            },
            None => Slot::Failed,
        };
    }
    match &mut *slot {
        Slot::Open(c) => Some(f(c)),
        _ => None,
    }
}

/// 把译文库搬到新文件夹（空字符串 = 搬回默认的数据目录），成功后写入配置。
/// 先合并预写日志、关闭连接，再移动文件；目标处已有译文库时拒绝，免得覆盖。
pub fn relocate(new_dir: &str) -> Result<PathBuf, String> {
    let mut cfg = crate::config::load()?;
    let new_dir = new_dir.trim();
    let target_dir = if new_dir.is_empty() {
        crate::config::data_dir().ok_or_else(|| t!("err.config.no_home"))?
    } else {
        PathBuf::from(new_dir)
    };
    let old = db_path().ok_or_else(|| t!("err.store.no_path"))?;
    let new = target_dir.join(DB_FILE);
    if same_file(&old, &new) {
        return Ok(new);
    }
    if new.exists() {
        return Err(t!("err.store.exists", path = new.display()));
    }
    std::fs::create_dir_all(&target_dir).map_err(|e| t!("err.store.mkdir", e = e))?;

    let mut slot = STORE.lock().unwrap();
    if let Slot::Open(c) = &*slot {
        let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }
    // 关闭连接（Windows 下文件打开时无法移动）；之后按新位置重新打开
    *slot = Slot::Unopened;
    if old.exists() {
        move_file(&old, &new)?;
        for ext in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", old.display()));
        }
    }
    cfg.store_dir = if new_dir.is_empty() { String::new() } else { target_dir.display().to_string() };
    if let Err(e) = crate::config::save(&cfg) {
        // 配置没写进去：把文件搬回原处，保持一致
        let _ = move_file(&new, &old);
        return Err(e);
    }
    Ok(new)
}

fn same_file(a: &PathBuf, b: &PathBuf) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// 移动文件；跨磁盘时改名会失败，退回复制再删除
fn move_file(from: &PathBuf, to: &PathBuf) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| t!("err.store.move", e = e))?;
    std::fs::remove_file(from).map_err(|e| t!("err.store.remove_old", e = e))
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// 批量查询，返回命中的（原文 → 译文），同时累计命中次数
pub fn get_many(lang: &str, sources: &[String]) -> HashMap<String, String> {
    with_db(|c| {
        get_many_in(c, lang, sources).unwrap_or_else(|e| {
            eprintln!("查询译文缓存失败：{e}");
            HashMap::new()
        })
    })
    .unwrap_or_default()
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
    with_db(|c| {
        if let Err(e) = put_many_in(c, lang, pairs, engine) {
            eprintln!("写入译文缓存失败：{e}");
        }
    });
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
    with_db(|c| {
        let r = (|| -> rusqlite::Result<Vec<(String, String)>> {
            let mut stmt = c.prepare_cached(
                "SELECT source, target FROM translations WHERE lang = ?1 ORDER BY used_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![lang, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })();
        r.unwrap_or_default()
    })
    .unwrap_or_default()
}

// ───────────────────────── 管理（设置界面用） ─────────────────────────

/// 译文库里的一条记录
#[derive(Debug, Clone, serde::Serialize)]
pub struct Entry {
    pub lang: String,
    pub source: String,
    pub target: String,
    pub engine: String,
    /// 用户手工改过（自动翻译不会覆盖）
    pub manual: bool,
    pub hits: i64,
    pub used_at: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct Page {
    pub total: i64,
    pub rows: Vec<Entry>,
}

fn with_conn<T>(f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    with_db(f).ok_or_else(|| t!("err.store.open"))?.map_err(|e| t!("err.store.op", e = e))
}

/// 分页查询：按最近使用排序；`keyword` 同时匹配原文和译文；`manual_only` 只看手工译文
pub fn query(lang: &str, keyword: &str, manual_only: bool, offset: i64, limit: i64) -> Result<Page, String> {
    with_conn(|c| query_in(c, lang, keyword, manual_only, offset, limit))
}

fn query_in(c: &Connection, lang: &str, keyword: &str, manual_only: bool, offset: i64, limit: i64) -> rusqlite::Result<Page> {
    // LIKE 的通配符要转义，用户搜 "100%" 不能匹配全部
    let kw = format!("%{}%", keyword.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let filter = "lang = ?1 AND (source LIKE ?2 ESCAPE '\\' OR target LIKE ?2 ESCAPE '\\') AND (?3 = 0 OR manual = 1)";
    let total = c.query_row(
        &format!("SELECT COUNT(*) FROM translations WHERE {filter}"),
        params![lang, kw, manual_only as i64],
        |r| r.get(0),
    )?;
    let mut stmt = c.prepare(&format!(
        "SELECT lang, source, target, engine, manual, hits, used_at FROM translations
         WHERE {filter} ORDER BY used_at DESC, source LIMIT ?4 OFFSET ?5"
    ))?;
    let rows = stmt
        .query_map(params![lang, kw, manual_only as i64, limit, offset], |r| {
            Ok(Entry {
                lang: r.get(0)?,
                source: r.get(1)?,
                target: r.get(2)?,
                engine: r.get(3)?,
                manual: r.get::<_, i64>(4)? != 0,
                hits: r.get(5)?,
                used_at: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Page { total, rows })
}

/// 某个目标语言的译文库概况（设置窗口概览页用）
#[derive(Debug, Default, serde::Serialize)]
pub struct Stats {
    /// 条数
    pub total: i64,
    /// 其中手工译文
    pub manual: i64,
    /// 累计命中次数（每命中一次就少请求一次引擎）
    pub hits: i64,
}

pub fn stats(lang: &str) -> Result<Stats, String> {
    with_conn(|c| stats_in(c, lang))
}

fn stats_in(c: &Connection, lang: &str) -> rusqlite::Result<Stats> {
    c.query_row(
        "SELECT COUNT(*), COALESCE(SUM(manual), 0), COALESCE(SUM(hits), 0) FROM translations WHERE lang = ?1",
        params![lang],
        |r| Ok(Stats { total: r.get(0)?, manual: r.get(1)?, hits: r.get(2)? }),
    )
}

/// 手工改译（不存在则新增），标记为手工译文
pub fn set_manual(lang: &str, source: &str, target: &str) -> Result<(), String> {
    with_conn(|c| set_manual_in(c, lang, source, target))
}

fn set_manual_in(c: &Connection, lang: &str, source: &str, target: &str) -> rusqlite::Result<()> {
    let t = now();
    c.execute(
        "INSERT INTO translations (lang, source, target, engine, manual, created_at, used_at)
         VALUES (?1, ?2, ?3, '手工', 1, ?4, ?4)
         ON CONFLICT (lang, source) DO UPDATE SET target = excluded.target, manual = 1",
        params![lang, source, target, t],
    )?;
    Ok(())
}

/// 删除若干条，返回删除条数
pub fn delete(lang: &str, sources: &[String]) -> Result<usize, String> {
    with_conn(|c| {
        let tx = c.transaction()?;
        let mut n = 0;
        {
            let mut stmt = tx.prepare("DELETE FROM translations WHERE lang = ?1 AND source = ?2")?;
            for s in sources {
                n += stmt.execute(params![lang, s])?;
            }
        }
        tx.commit()?;
        Ok(n)
    })
}

/// 清空某个目标语言的译文；`keep_manual` 为真时保留手工译文。返回删除条数
pub fn clear(lang: &str, keep_manual: bool) -> Result<usize, String> {
    with_conn(|c| {
        c.execute(
            "DELETE FROM translations WHERE lang = ?1 AND (?2 = 0 OR manual = 0)",
            params![lang, keep_manual as i64],
        )
    })
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
    fn 查询与改译() {
        let mut c = mem();
        let pairs = vec![
            ("Open file".to_string(), "打开文件".to_string()),
            ("100% done".to_string(), "完成".to_string()),
            ("Close".to_string(), "关闭".to_string()),
        ];
        put_many_in(&mut c, "zh-CN", &pairs, "edge").unwrap();
        assert_eq!(query_in(&c, "zh-CN", "", false, 0, 10).unwrap().total, 3);
        assert_eq!(query_in(&c, "zh-CN", "文件", false, 0, 10).unwrap().total, 1, "按译文搜");
        assert_eq!(query_in(&c, "zh-CN", "%", false, 0, 10).unwrap().total, 1, "百分号按字面匹配");
        set_manual_in(&c, "zh-CN", "Close", "关掉").unwrap();
        set_manual_in(&c, "zh-CN", "New", "新建").unwrap();
        let manual = query_in(&c, "zh-CN", "", true, 0, 10).unwrap();
        assert_eq!(manual.total, 2);
        assert!(manual.rows.iter().all(|r| r.manual));
        let s = stats_in(&c, "zh-CN").unwrap();
        assert_eq!((s.total, s.manual), (4, 2));
        assert_eq!(stats_in(&c, "ja").unwrap().total, 0, "空库也能统计");
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
