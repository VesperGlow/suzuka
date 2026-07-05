use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::json;

/// 请求体上限，留言与计数写接口共用。
pub const MAX_REQUEST_BYTES: usize = 16 << 10;

/// ClientIp 提取请求来源 IP。仅当直连方来自回环或私有网络（即常见的本机/容器反代）时，
/// 才信任 X-Forwarded-For；公网直连客户端不能靠伪造请求头绕过限流。
/// 上游反代仍须覆盖而不是透传客户端自带的 X-Forwarded-For。
pub struct ClientIp(pub String);

impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0);
        Ok(ClientIp(client_ip(peer, &parts.headers)))
    }
}

pub fn client_ip(peer: Option<SocketAddr>, headers: &HeaderMap) -> String {
    let Some(remote) = peer.map(|addr| canonical(addr.ip())) else {
        return "unknown".to_string();
    };

    if remote.is_loopback() || is_private(remote) {
        if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            for value in forwarded.split(',') {
                if let Ok(ip) = value.trim().parse::<IpAddr>() {
                    return canonical(ip).to_string();
                }
            }
        }
    }
    remote.to_string()
}

/// 把 IPv4-mapped IPv6 地址还原为 IPv4，保证限流 key 的一致性。
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

/// RFC 1918 私有网段与 IPv6 ULA（fc00::/7），对应 Go net/netip 的 IsPrivate。
fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00,
    }
}

/// 校验传入路径是否为站内 /posts/ 的相对路径，
/// 既用于留言的来源链接，也用于阅读数 / 喜欢的目标文章。
pub fn valid_post_url(value: &str) -> bool {
    if value.contains('?') || value.contains('#') {
        return false;
    }
    // 必须以 /posts/ 开头（顺带排除了带 scheme 或 //host 的绝对形式），
    // 且归一化掉 ".." 后仍留在 /posts/ 下，防目录穿越式的路径伪装。
    if !value.starts_with("/posts/") {
        return false;
    }
    clean_path(value).starts_with("/posts/")
}

/// Go path.Clean 的绝对路径子集：折叠空段与 "."，按栈消解 ".."，去掉尾部斜杠。
fn clean_path(value: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in value.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    format!("/{}", segments.join("/"))
}

pub fn write_json<T: Serialize>(status: StatusCode, value: &T) -> Response {
    (status, Json(value)).into_response()
}

pub fn write_error(status: StatusCode, text: &str) -> Response {
    write_json(status, &json!({ "error": text }))
}

/// 对外隐藏实现细节，同时把真实错误留在服务日志中便于排查。
pub fn internal_error(operation: &str, err: &dyn std::fmt::Display) -> Response {
    eprintln!("{operation}: {err}");
    write_error(StatusCode::INTERNAL_SERVER_ERROR, operation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn client_ip_trusts_only_local_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));

        let public: SocketAddr = "198.51.100.10:1234".parse().unwrap();
        assert_eq!(client_ip(Some(public), &headers), "198.51.100.10");

        let private: SocketAddr = "10.0.0.2:1234".parse().unwrap();
        assert_eq!(client_ip(Some(private), &headers), "203.0.113.7");

        headers.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));
        assert_eq!(client_ip(Some(private), &headers), "10.0.0.2");
    }

    #[test]
    fn post_url_validation() {
        assert!(valid_post_url("/posts/example/"));
        assert!(!valid_post_url("/posts/../about/"));
        assert!(!valid_post_url("/about/"));
        assert!(!valid_post_url("https://example.com/posts/a/"));
        assert!(!valid_post_url("/posts/a/?x=1"));
        assert!(!valid_post_url("/posts/"));
    }
}
