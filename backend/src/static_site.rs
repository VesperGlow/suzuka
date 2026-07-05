use std::convert::Infallible;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tower::service_fn;
use tower_http::services::ServeDir;

/// 在根路径托管 Hugo 产物。ServeDir 自带目录 index.html 回退与
/// Last-Modified / Range / 条件请求；MIME 表由 mime_guess 内置，
/// 不依赖运行镜像里的 /etc/mime.types。未命中时返回 Hugo 的 404.html。
pub fn static_router(dir: &str) -> Router {
    let not_found_page = Path::new(dir).join("404.html");
    let fallback = service_fn(move |request: Request<Body>| {
        let page = not_found_page.clone();
        async move {
            Ok::<_, Infallible>(serve_not_found(&page, request.method() == Method::HEAD).await)
        }
    });

    Router::new()
        .fallback_service(ServeDir::new(dir).not_found_service(fallback))
        .layer(middleware::from_fn(static_headers))
}

/// 给静态响应补上 nosniff 与按路径计算的 Cache-Control。
async fn static_headers(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    let status = response.status();
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // 只给成功命中的文件（含 304 / Range）声明缓存策略，404 页保持默认协商缓存。
    if status.is_success() || status.is_redirection() {
        if let Some(cc) = cache_control_for(&path) {
            headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cc));
        }
    }
    response
}

/// 指纹化资源：文件名内嵌内容 hash，内容变了文件名就变，可放心钉死缓存。
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";
/// 文件名固定但内容会随发布变化的资源：每次回源做条件请求校验，
/// 命中则 304。否则 CDN/浏览器会把旧版本钉死一年（典型受害者是 pagefind 搜索索引）。
const REVALIDATE_CACHE: &str = "no-cache";

/// 决定静态资源的 Cache-Control。让 origin 成为缓存策略的唯一事实源，
/// 这样无论前面的 Cloudflare / 反代「遵循源站头」即可得到正确行为。
///
/// 关键区分：pagefind 的 pagefind.js / pagefind-entry.json 文件名固定、内容却随文章更新，
/// 一旦被当成指纹资源 immutable 缓存，老访客就会一直拿到旧索引、搜不到新文章。
fn cache_control_for(url_path: &str) -> Option<&'static str> {
    let base = url_path.rsplit('/').next().unwrap_or(url_path);

    // pagefind：带 hash 的分片可长期缓存；固定名的入口与运行时必须回源校验。
    if url_path.starts_with("/pagefind/") {
        if url_path.starts_with("/pagefind/index/")
            || url_path.starts_with("/pagefind/fragment/")
            || base.ends_with(".pagefind")
        {
            return Some(IMMUTABLE_CACHE);
        }
        return Some(REVALIDATE_CACHE);
    }

    // Hugo 生成的搜索回退索引：固定名，每次发布都变。
    if base == "index.json" {
        return Some(REVALIDATE_CACHE);
    }

    // Hugo 指纹资源（name.min.<hash>.ext）：改了内容即换名字，可 immutable。
    if is_fingerprinted(base) {
        return Some(IMMUTABLE_CACHE);
    }

    // 其余（HTML、favicon、manifest 等）沿用默认协商缓存，不显式声明。
    None
}

/// 判断是否为 Hugo 指纹文件：倒数第二段为一段长十六进制内容散列。
fn is_fingerprinted(base: &str) -> bool {
    let parts: Vec<&str> = base.split('.').collect();
    if parts.len() < 3 {
        return false;
    }
    let seg = parts[parts.len() - 2];
    seg.len() >= 16
        && seg
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// 输出站点自带的 404 页面并带上 404 状态码；页面缺失时退回纯文本 404。
async fn serve_not_found(page: &PathBuf, is_head: bool) -> Response {
    match tokio::fs::read(page).await {
        Ok(body) => {
            let body = if is_head { Vec::new() } else { body };
            (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                body,
            )
                .into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "404 page not found").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_policy() {
        assert_eq!(
            cache_control_for("/pagefind/pagefind.js"),
            Some(REVALIDATE_CACHE)
        );
        assert_eq!(
            cache_control_for("/pagefind/index/en_1234abcd.pf_index"),
            Some(IMMUTABLE_CACHE)
        );
        assert_eq!(cache_control_for("/index.json"), Some(REVALIDATE_CACHE));
        assert_eq!(
            cache_control_for("/css/main.min.0123456789abcdef0123.css"),
            Some(IMMUTABLE_CACHE)
        );
        assert_eq!(cache_control_for("/posts/example/index.html"), None);
        assert_eq!(cache_control_for("/favicon.ico"), None);
    }
}
