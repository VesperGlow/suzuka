use std::collections::HashSet;
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

/// 在根路径托管 ssg 产物。ServeDir 自带目录 index.html 回退与
/// Last-Modified / Range / 条件请求；MIME 表由 mime_guess 内置，
/// 不依赖运行镜像里的 /etc/mime.types。未命中时返回 ssg 的 404.html。
pub fn static_router(dir: &str) -> Router {
    let not_found_page = Path::new(dir).join("404.html");
    let fallback = service_fn(move |request: Request<Body>| {
        let page = not_found_page.clone();
        async move {
            Ok::<_, Infallible>(serve_not_found(&page, request.method() == Method::HEAD).await)
        }
    });

    // CSP 只需在启动时组装一次：内联脚本哈希清单随静态产物一起发布，
    // 产物不换，清单不变。
    let csp = load_csp_header(Path::new(dir));
    Router::new()
        .fallback_service(ServeDir::new(dir).not_found_service(fallback))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let csp = csp.clone();
            async move { static_headers(request, next, csp).await }
        }))
}

/// 扫描 ssg 静态产物里真实存在的文章路径（posts/ 下含 index.html 的目录），
/// 作为阅读数/点赞与留言引用的白名单——不然任何以 /posts/ 开头的伪造路径
/// 都能往数据库里无限造行。posts/ 顶层的列表页（posts/index.html 自身）
/// 不在结果里：遍历从子目录开始。
pub fn scan_post_paths(dir: &Path) -> std::io::Result<HashSet<String>> {
    let mut paths = HashSet::new();
    let posts_root = dir.join("posts");
    collect_post_dirs(&posts_root, &posts_root, &mut paths)?;
    Ok(paths)
}

fn collect_post_dirs(root: &Path, dir: &Path, paths: &mut HashSet<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("index.html").is_file() {
            // strip_prefix 不会失败：path 由 root 逐层 read_dir 而来。
            let rel = path
                .strip_prefix(root)
                .expect("directory came from walking root")
                .to_string_lossy()
                .replace('\\', "/");
            paths.insert(format!("/posts/{rel}/"));
        }
        collect_post_dirs(root, &path, paths)?;
    }
    Ok(())
}

/// 组装静态站的 Content-Security-Policy。内联脚本哈希由 ssg 构建时写进
/// 产物根部的 csp-hashes.txt（对最终落盘字节算的 sha256，见 ssg 的
/// build.rs）；启动时读一次。清单缺失/不可读时干脆不下发 CSP——宁可少
/// 一层防护，也不能对着旧产物把里面的内联脚本（主题闪烁抑制等）全拦死。
fn load_csp_header(dir: &Path) -> Option<HeaderValue> {
    let path = dir.join("csp-hashes.txt");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!(
                "warning: read {} failed ({err}); serving without Content-Security-Policy",
                path.display()
            );
            return None;
        }
    };
    // 'wasm-unsafe-eval'：英文搜索的 pagefind 要实例化 WebAssembly
    let mut script_src = String::from("'self' 'wasm-unsafe-eval'");
    for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !line.starts_with("sha256-") || line.contains(['\'', ';']) {
            eprintln!(
                "warning: {} 里有格式不对的行（{line}），已跳过",
                path.display()
            );
            continue;
        }
        script_src.push_str(" '");
        script_src.push_str(line);
        script_src.push('\'');
    }
    let value = format!(
        "default-src 'self'; script-src {script_src}; style-src 'self'; \
         img-src 'self' data:; object-src 'none'; base-uri 'self'; \
         form-action 'self'; frame-ancestors 'none'"
    );
    match HeaderValue::from_str(&value) {
        Ok(header) => Some(header),
        Err(err) => {
            eprintln!("warning: CSP 头构造失败（{err}），本次不下发");
            None
        }
    }
}

/// 给静态响应补上 nosniff / 反嵌入 / Referrer / HSTS / CSP 策略与按路径
/// 计算的 Cache-Control。
async fn static_headers(request: Request, next: Next, csp: Option<HeaderValue>) -> Response {
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    let status = response.status();
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    // TLS 由 Cloudflare 终止，这个头随响应透传给浏览器。纯 HTTP 环境
    // （本地开发）下浏览器会忽略它，无需条件判断。不带 includeSubDomains：
    // 子域名是否全走 HTTPS 不是这个服务能替域名主人决定的事。
    headers.insert(
        header::STRICT_TRANSPORT_SECURITY,
        HeaderValue::from_static("max-age=31536000"),
    );
    if let Some(csp) = csp {
        headers.insert(header::CONTENT_SECURITY_POLICY, csp);
    }
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

    // ssg 生成的搜索回退索引：固定名，每次发布都变。
    if base == "index.json" {
        return Some(REVALIDATE_CACHE);
    }

    // ssg 指纹资源（name.min.<hash>.ext）：改了内容即换名字，可 immutable。
    if is_fingerprinted(base) {
        return Some(IMMUTABLE_CACHE);
    }

    // 其余（HTML、favicon、manifest 等）沿用默认协商缓存，不显式声明。
    None
}

/// 判断是否为 ssg 指纹文件：倒数第二段为一段长十六进制内容散列。
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
    fn scans_only_post_dirs_with_index() {
        let tmp = tempfile::TempDir::new().unwrap();
        let make_page = |rel: &str| {
            let dir = tmp.path().join(rel);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("index.html"), "<html></html>").unwrap();
        };
        make_page("posts");
        make_page("posts/hello-world");
        make_page("posts/nested/deep-post");
        make_page("about");
        // 只有图片资源、没有 index.html 的目录不算文章
        std::fs::create_dir_all(tmp.path().join("posts/images-only")).unwrap();

        let paths = scan_post_paths(tmp.path()).unwrap();
        assert_eq!(
            paths,
            std::collections::HashSet::from([
                "/posts/hello-world/".to_string(),
                "/posts/nested/deep-post/".to_string(),
            ])
        );
    }

    #[test]
    fn csp_from_manifest() {
        let tmp = tempfile::TempDir::new().unwrap();
        // 清单缺失：不下发 CSP，而不是下发一个拦死内联脚本的
        assert_eq!(load_csp_header(tmp.path()), None);

        std::fs::write(
            tmp.path().join("csp-hashes.txt"),
            "sha256-2kpHhkcU4p5OoreyhonSFcII/0Gef6Q6W97spxuJaWg=\n\nbogus line\n",
        )
        .unwrap();
        let header = load_csp_header(tmp.path()).unwrap();
        let value = header.to_str().unwrap();
        assert!(value
            .contains("script-src 'self' 'wasm-unsafe-eval' 'sha256-2kpHhkcU4p5OoreyhonSFcII/0Gef6Q6W97spxuJaWg='"));
        // 格式不对的行只跳过，不进头、也不让启动失败
        assert!(!value.contains("bogus"));
        assert!(value.starts_with("default-src 'self'; "));
        assert!(value.contains("frame-ancestors 'none'"));
    }

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
