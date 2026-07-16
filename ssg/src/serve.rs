//! `ssg serve`：本地开发用的"构建 + 监听变更重建 + 静态文件服务"。
//!
//! 刻意零新依赖：变更检测用 500ms 轮询扫 mtime/大小（站点规模小，扫一遍
//! 毫秒级），HTTP 是手写的极简 GET/HEAD 静态文件服务，只监听回环地址。
//! 每次变更做全量重建——本站全量构建本来就是秒级，不值得做增量。
//!
//! 与生产的差异：不跑 Pagefind（本地搜索不可用），不 minify；
//! /api/guestbook/ 下的动态接口这里没有（返回 404），联调动态功能仍按
//! README 用后端一体托管静态产物的方式。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::build;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

pub fn serve(source: &Path, dest: &Path, addr: &str) -> Result<()> {
    // 初次构建失败直接退出（没有可服务的产物）；此后的重建失败只报错，
    // 继续服务上一次的产物，改错了内容不至于把预览一起弄死。
    build::build(source, dest, false)?;

    let listener = TcpListener::bind(addr).with_context(|| format!("监听 {addr}"))?;
    println!("serving {} at http://{addr}/", dest.display());
    println!("watching for changes (Ctrl-C to stop)");

    let serve_root = dest.to_path_buf();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let root = serve_root.clone();
            std::thread::spawn(move || {
                let _ = handle_connection(stream, &root);
            });
        }
    });

    let mut fingerprint = source_fingerprint(source);
    loop {
        std::thread::sleep(POLL_INTERVAL);
        let current = source_fingerprint(source);
        if current == fingerprint {
            continue;
        }
        // 等到连续两次扫描一致再重建：避开保存到一半的文件。
        fingerprint = current;
        loop {
            std::thread::sleep(POLL_INTERVAL);
            let settled = source_fingerprint(source);
            if settled == fingerprint {
                break;
            }
            fingerprint = settled;
        }
        let started = std::time::Instant::now();
        match build::build(source, dest, false) {
            Ok(()) => println!("rebuilt in {:.1}s", started.elapsed().as_secs_f32()),
            Err(err) => eprintln!("rebuild failed (still serving previous output): {err:#}"),
        }
    }
}

/// 站点输入的指纹：所有被 build 读取的路径的 (相对路径, mtime, 大小) 散列。
/// 任何新增/删除/修改都会改变它。
fn source_fingerprint(source: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    let roots = [
        source.join("content"),
        source.join("assets"),
        source.join("static"),
        source.join("i18n"),
        source.join("ssg").join("templates"),
        source.join("site.toml"),
    ];
    for root in roots {
        for entry in walkdir::WalkDir::new(&root)
            .sort_by_file_name()
            .into_iter()
            .flatten()
        {
            if !entry.file_type().is_file() {
                continue;
            }
            entry.path().hash(&mut hasher);
            if let Ok(meta) = entry.metadata() {
                meta.len().hash(&mut hasher);
                if let Ok(modified) = meta.modified() {
                    modified.hash(&mut hasher);
                }
            }
        }
    }
    hasher.finish()
}

fn handle_connection(mut stream: TcpStream, root: &Path) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let request_line = read_request_line(&mut stream)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();

    if method != "GET" && method != "HEAD" {
        return write_response(&mut stream, 405, "text/plain; charset=utf-8", b"", false);
    }
    let head_only = method == "HEAD";

    let Some(rel) = decode_path(&target) else {
        return write_response(
            &mut stream,
            400,
            "text/plain; charset=utf-8",
            b"bad request",
            head_only,
        );
    };

    // 目录 URL 落到 index.html；没有扩展名的裸路径按目录 URL 的习惯跳转，
    // 与生产环境（后端 ServeDir / CDN）的行为一致。
    let mut file = root.join(&rel);
    if target.ends_with('/') || rel.is_empty() {
        file = file.join("index.html");
    }
    if !file.is_file() && !target.ends_with('/') && root.join(&rel).join("index.html").is_file() {
        let location = format!("{}/", target.split('?').next().unwrap_or(&target));
        let head = format!(
            "HTTP/1.1 301 Moved Permanently\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return stream.write_all(head.as_bytes());
    }

    let (status, body_path) = if file.is_file() {
        (200, file)
    } else {
        (404, root.join("404.html"))
    };
    let body = std::fs::read(&body_path).unwrap_or_default();
    let mime = content_type(&body_path);
    write_response(&mut stream, status, mime, &body, head_only)
}

/// 读到请求头结束（空行）为止，返回请求行。GET/HEAD 没有请求体；必须把
/// 头读完再响应——带着未读数据关连接会触发 TCP RST，浏览器可能丢弃响应。
fn read_request_line(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while buf.len() < 16384 && !buf.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 {
            break;
        }
        buf.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&buf);
    Ok(text.lines().next().unwrap_or_default().to_string())
}

/// URL path → dest 内的相对路径。percent-decode（非 ASCII slug 的目录名在
/// 磁盘上是原始 UTF-8），去掉查询串，拒绝 `..` 与反斜杠。
fn decode_path(target: &str) -> Option<String> {
    let path = target.split(['?', '#']).next().unwrap_or(target);
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .ok()?;
    if decoded.contains("..") || decoded.contains('\\') {
        return None;
    }
    Some(decoded.trim_matches('/').to_string())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "xml" => "application/xml",
        "webp" => "image/webp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Method Not Allowed",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    if !head_only {
        stream.write_all(body)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_path_normalizes_and_rejects_traversal() {
        assert_eq!(decode_path("/"), Some(String::new()));
        assert_eq!(decode_path("/posts/foo/"), Some("posts/foo".to_string()));
        assert_eq!(
            decode_path("/tags/%E8%A7%86%E8%A7%89%E5%B0%8F%E8%AF%B4/?x=1"),
            Some("tags/视觉小说".to_string())
        );
        assert_eq!(decode_path("/../etc/passwd"), None);
        assert_eq!(decode_path("/a/%2E%2E/b"), None);
        assert_eq!(decode_path("/a\\b"), None);
    }
}
