use std::error::Error;
use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

pub const CURRENT_SCHEMA_VERSION: i64 = 2;

const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    email TEXT NOT NULL DEFAULT '',
    website TEXT NOT NULL DEFAULT '',
    content TEXT NOT NULL,
    ref_title TEXT NOT NULL DEFAULT '',
    ref_url TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_created_at ON messages(created_at DESC, id DESC);

CREATE TABLE IF NOT EXISTS page_views (
    path TEXT PRIMARY KEY,
    count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS reactions (
    path TEXT PRIMARY KEY,
    count INTEGER NOT NULL DEFAULT 0
);";

// 留言分页实际按 id（INTEGER PRIMARY KEY，rowid 别名，SQLite 自带隐式索引）
// 排序，从没用上按 created_at 排序的 idx_messages_created_at，纯粹是死索引。
const SCHEMA_V2: &str = "DROP INDEX IF EXISTS idx_messages_created_at;";

pub type BoxError = Box<dyn Error + Send + Sync>;

/// 打开（必要时创建）SQLite 库，并幂等地建好留言、阅读数、喜欢三张表。
/// 单连接串行访问（对应旧实现的 SetMaxOpenConns(1)），由调用方包一层 Mutex。
pub fn open_database(path: &Path) -> Result<Connection, BoxError> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|e| format!("create database directory: {e}"))?;
        }
    }

    let mut conn = Connection::open(path).map_err(|e| format!("open database: {e}"))?;
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(|e| format!("set busy timeout: {e}"))?;
    // journal_mode 会返回结果行，不能走 pragma_update。
    conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
        .map_err(|e| format!("enable WAL: {e}"))?;
    // WAL 下 NORMAL 足够安全（只在系统级掉电时可能丢最后一笔事务，进程崩溃不受影响），
    // 省掉每次写的一次 fsync。
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| format!("set synchronous mode: {e}"))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| format!("enable foreign keys: {e}"))?;

    migrate_database(&mut conn).map_err(|e| format!("initialize database: {e}"))?;
    Ok(conn)
}

/// 用 SQLite 的 user_version 记录结构版本；以后新增字段时按版本追加迁移，
/// 避免只靠 CREATE TABLE IF NOT EXISTS 而无法升级已有数据库。
fn migrate_database(conn: &mut Connection) -> Result<(), BoxError> {
    let tx = conn.transaction()?;

    let version: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "database schema version {version} is newer than supported version {CURRENT_SCHEMA_VERSION}"
        )
        .into());
    }

    if version < 1 {
        tx.execute_batch(SCHEMA_V1)?;
        tx.execute_batch("PRAGMA user_version = 1;")?;
    }
    if version < 2 {
        tx.execute_batch(SCHEMA_V2)?;
        tx.execute_batch("PRAGMA user_version = 2;")?;
    }

    tx.commit()?;
    Ok(())
}
