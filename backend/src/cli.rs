//! 留言管理子命令：不经 HTTP，直接操作 SQLite。服务进程与本命令可以并存——
//! 数据库开着 WAL 且双方都设了 busy_timeout，短暂的写入互不阻塞。
//!
//! 本机：`backend list` / `backend delete <id>...`
//! 容器（scratch 镜像无 shell，但 exec 二进制不需要 shell）：
//! `podman exec suzuka /backend list`

use rusqlite::{params, Connection};

/// 每条留言两行：首行是 id、时间与来源信息，次行缩进展示正文（压平换行）。
pub fn list_messages(conn: &Connection) -> Result<String, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, created_at, name, email, website, content, ref_url
FROM messages
ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| {
        let id: i64 = row.get(0)?;
        let created_at: String = row.get(1)?;
        let name: String = row.get(2)?;
        let email: String = row.get(3)?;
        let website: String = row.get(4)?;
        let content: String = row.get(5)?;
        let ref_url: String = row.get(6)?;
        let mut line = format!("#{id}  {created_at}  {name}");
        if !email.is_empty() {
            line.push_str(&format!(" <{email}>"));
        }
        if !website.is_empty() {
            line.push_str(&format!("  {website}"));
        }
        if !ref_url.is_empty() {
            line.push_str(&format!("  re: {ref_url}"));
        }
        line.push_str(&format!("\n    {}\n", content.replace('\n', " ")));
        Ok(line)
    })?;

    let mut out = String::new();
    let mut count = 0i64;
    for line in rows {
        out.push_str(&line?);
        count += 1;
    }
    out.push_str(&format!("{count} message(s)\n"));
    Ok(out)
}

/// 逐条删除并报告结果；只要有一个 id 不存在就返回 false，让调用方以非零码退出，
/// 避免脚本里删错 id 而不自知。
pub fn delete_messages(conn: &Connection, ids: &[i64]) -> Result<bool, rusqlite::Error> {
    let mut all_found = true;
    for id in ids {
        match conn.execute("DELETE FROM messages WHERE id = ?1", params![id])? {
            0 => {
                eprintln!("#{id}: not found");
                all_found = false;
            }
            _ => println!("#{id}: deleted"),
        }
    }
    Ok(all_found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_database;

    fn seeded_db() -> (Connection, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = open_database(&tmp.path().join("guestbook.db")).unwrap();
        conn.execute_batch(
            "INSERT INTO messages (name, email, website, content, ref_title, ref_url, created_at)
VALUES ('Suzuka', 'a@example.com', '', '你好' || char(10) || '世界', '一篇文章', '/posts/example/', '2026-07-16T12:00:00Z'),
       ('spammer', '', 'https://spam.example', 'buy stuff', '', '', '2026-07-17T00:00:00Z');",
        )
        .unwrap();
        (conn, tmp)
    }

    #[test]
    fn list_shows_ids_and_flattens_newlines() {
        let (conn, _tmp) = seeded_db();
        let out = list_messages(&conn).unwrap();
        assert!(out.contains("#1  2026-07-16T12:00:00Z  Suzuka <a@example.com>"));
        assert!(out.contains("re: /posts/example/"));
        assert!(out.contains("你好 世界"), "newlines flattened: {out}");
        assert!(out.contains("#2  2026-07-17T00:00:00Z  spammer  https://spam.example"));
        assert!(out.ends_with("2 message(s)\n"));
    }

    #[test]
    fn delete_removes_rows_and_flags_missing_ids() {
        let (conn, _tmp) = seeded_db();
        assert!(delete_messages(&conn, &[2]).unwrap());
        assert!(!delete_messages(&conn, &[2]).unwrap(), "already gone");

        let out = list_messages(&conn).unwrap();
        assert!(!out.contains("spammer"));
        assert!(out.ends_with("1 message(s)\n"));
    }
}
