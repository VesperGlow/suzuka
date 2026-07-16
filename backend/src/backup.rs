//! 数据库的每日一致性快照。
//!
//! WAL 模式下服务运行时直接复制 guestbook.db（连同 -wal/-shm）可能拷到缺
//! 数据甚至打不开的半成品；`VACUUM INTO` 产出的快照则是一个完整独立的
//! 数据库文件。快照落在数据目录内（默认 `<db 目录>/backups/`），之后宿主
//! 侧直接复制 / rsync 整个数据目录，拿到的快照永远可用。
//!
//! 快照文件名形如 `guestbook-20260716-120000.db`（时间戳按字典序即时间序），
//! 只保留最近 KEEP_SNAPSHOTS 份。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::Connection;
use time::macros::format_description;

use crate::httputil::log_timestamp;
use crate::server::App;

const KEEP_SNAPSHOTS: usize = 7;
/// 略小于 24 小时：每天固定时刻重启或检查时，不因几分钟的误差跳过当天快照。
const MIN_SNAPSHOT_AGE: Duration = Duration::from_secs(23 * 60 * 60);
const CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// 启动后台快照任务：立即检查一次（首次部署即落第一份快照），此后每小时
/// 检查最新快照是否已满一天。失败只写日志，下个周期重试。
pub fn spawn(app: Arc<App>, dir: PathBuf) {
    tokio::spawn(async move {
        loop {
            if let Err(err) = run_once(&app, &dir) {
                eprintln!("{} backup: {err}", log_timestamp());
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

fn run_once(app: &App, dir: &Path) -> Result<(), String> {
    if newest_snapshot_age(dir)?.is_some_and(|age| age < MIN_SNAPSHOT_AGE) {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    remove_leftover_tmp(dir);

    let format = format_description!("[year][month][day]-[hour][minute][second]");
    let ts = (app.now)()
        .format(&format)
        .map_err(|e| format!("format timestamp: {e}"))?;
    let name = format!("guestbook-{ts}.db");
    // 先写临时名再改名：进程在 VACUUM 中途死掉时，残留的半成品不会顶着
    // 合法快照的名字混进备份序列（.tmp 由下次运行清理）。
    let tmp = dir.join(format!("{name}.tmp"));
    let path = dir.join(&name);
    snapshot(&app.conn(), &tmp)?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename {}: {e}", path.display()))?;
    prune(dir, KEEP_SNAPSHOTS)?;
    println!("{} backup: wrote {}", log_timestamp(), path.display());
    Ok(())
}

fn snapshot(conn: &Connection, target: &Path) -> Result<(), String> {
    let target = target
        .to_str()
        .ok_or_else(|| format!("backup path is not valid UTF-8: {}", target.display()))?;
    conn.execute("VACUUM INTO ?1", [target])
        .map_err(|e| format!("VACUUM INTO {target}: {e}"))?;
    Ok(())
}

fn is_snapshot(name: &str) -> bool {
    name.starts_with("guestbook-") && name.ends_with(".db")
}

/// 目录里现有快照的名字列表，按文件名（即时间）升序。
fn snapshots(dir: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(names),
        Err(err) => return Err(format!("read {}: {err}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("read {}: {e}", dir.display()))?;
        if let Some(name) = entry.file_name().to_str() {
            if is_snapshot(name) {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

fn newest_snapshot_age(dir: &Path) -> Result<Option<Duration>, String> {
    let Some(newest) = snapshots(dir)?.pop() else {
        return Ok(None);
    };
    let path = dir.join(newest);
    let modified = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map_err(|e| format!("stat {}: {e}", path.display()))?;
    // 时钟回拨等异常时按"刚写过"处理，宁可跳过一天也不连续重写。
    Ok(Some(modified.elapsed().unwrap_or(Duration::ZERO)))
}

fn prune(dir: &Path, keep: usize) -> Result<(), String> {
    let names = snapshots(dir)?;
    for name in names.iter().rev().skip(keep) {
        let path = dir.join(name);
        if let Err(err) = std::fs::remove_file(&path) {
            eprintln!("{} backup: prune {}: {err}", log_timestamp(), path.display());
        }
    }
    Ok(())
}

/// 清掉上次崩溃可能残留的 .tmp 半成品（同一时刻只有本任务在写这个目录）。
fn remove_leftover_tmp(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_str().is_some_and(|n| n.ends_with(".tmp")) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_a_complete_database() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = Connection::open(tmp.path().join("live.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (id INTEGER PRIMARY KEY, content TEXT);
             INSERT INTO messages (content) VALUES ('hello'), ('world');",
        )
        .unwrap();

        let target = tmp.path().join("snap.db");
        snapshot(&conn, &target).unwrap();

        let copy = Connection::open(&target).unwrap();
        let count: i64 = copy
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn prune_keeps_the_newest_snapshots() {
        let tmp = tempfile::TempDir::new().unwrap();
        for ts in [
            "20260701-120000",
            "20260702-120000",
            "20260703-120000",
            "20260704-120000",
        ] {
            std::fs::write(tmp.path().join(format!("guestbook-{ts}.db")), b"x").unwrap();
        }
        // 无关文件与 .tmp 半成品不参与保留名额，也不该被 prune 碰。
        std::fs::write(tmp.path().join("guestbook.db"), b"live").unwrap();

        prune(tmp.path(), 2).unwrap();

        let kept = snapshots(tmp.path()).unwrap();
        assert_eq!(
            kept,
            vec![
                "guestbook-20260703-120000.db".to_string(),
                "guestbook-20260704-120000.db".to_string(),
            ]
        );
        assert!(tmp.path().join("guestbook.db").exists());
    }
}
