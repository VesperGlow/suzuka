use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{header, HeaderValue};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use rusqlite::Connection;
use time::OffsetDateTime;

use crate::guestbook::{self, POST_BURST, POST_WINDOW};
use crate::httputil::MAX_REQUEST_BYTES;
use crate::metrics::{self, COUNTER_BURST, COUNTER_WINDOW};
use crate::ratelimit::{Clock, RateLimiter};
use crate::static_site;

/// App 持有服务共享的依赖：数据库、时间源，以及留言与计数接口各自的限流器。
pub struct App {
    pub db: Mutex<Connection>,
    pub now: Clock,
    pub limiter: Option<RateLimiter>,
    pub counter_limiter: Option<RateLimiter>,
    /// 单容器模式下从静态产物扫出的真实文章路径白名单（见
    /// static_site::scan_post_paths）；None（纯 API 模式）时只做前缀校验。
    pub allowed_paths: Option<HashSet<String>>,
}

impl App {
    pub fn new(db: Connection) -> Self {
        let now: Clock = Arc::new(OffsetDateTime::now_utc);
        Self {
            db: Mutex::new(db),
            now: now.clone(),
            limiter: Some(RateLimiter::new(POST_BURST, POST_WINDOW, now.clone())),
            counter_limiter: Some(RateLimiter::new(COUNTER_BURST, COUNTER_WINDOW, now)),
            allowed_paths: None,
        }
    }

    /// 路径是否指向一篇真实发布的文章。未配置白名单时放行（此时调用方
    /// 已做过 /posts/ 前缀校验）。
    pub fn post_path_allowed(&self, path: &str) -> bool {
        self.allowed_paths
            .as_ref()
            .is_none_or(|paths| paths.contains(path))
    }

    /// 取数据库连接锁：即便某次持锁期间 panic 导致 Mutex 中毒，也照样拿到内部
    /// 连接继续服务，不让单次请求的意外 panic 演变成后续所有请求永久 500。
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.db
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 决定最终对外的路由形态：
///   - 传入 static_dir 时进入「单容器」模式：根路径托管 ssg 静态产物，
///     留言板 API 收敛到 /api/guestbook/ 前缀下（与前端 fetch 的路径一致，
///     由 nest 剥掉前缀，省掉外层 nginx 反代）。
///   - 未传入时退回纯 API 模式（本地开发或仅后端镜像），行为与之前完全一致。
pub fn root_handler(app: Arc<App>, static_dir: Option<&str>) -> Router {
    let api = handler(app);
    match static_dir.map(str::trim).filter(|d| !d.is_empty()) {
        None => api,
        Some(dir) => Router::new()
            .nest("/api/guestbook", api)
            .merge(static_site::static_router(dir)),
    }
}

/// 组装路由：留言、阅读数、喜欢，以及「关于」页的汇总数据。
/// 方法不匹配时由 axum 的方法路由自动返回 405 并填好 Allow 头。
pub fn handler(app: Arc<App>) -> Router {
    Router::new()
        .route(
            "/messages",
            get(guestbook::list_messages).post(guestbook::create_message),
        )
        .route("/views", get(metrics::read_views).post(metrics::bump_views))
        .route(
            "/reactions",
            get(metrics::read_reactions).post(metrics::bump_reactions),
        )
        .route("/summary", get(metrics::handle_summary))
        .layer(middleware::from_fn(security_headers))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(app)
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_database;

    #[test]
    fn conn_survives_a_panic_while_locked() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open_database(&tmp.path().join("guestbook.db")).unwrap();
        let app = Arc::new(App::new(db));

        // 模拟某次持锁期间业务代码 panic：std::sync::Mutex 会把自己标记为
        // "中毒"，此后任何 .lock().unwrap() 都会 panic。
        let poisoning = app.clone();
        let joined = std::thread::spawn(move || {
            let _conn = poisoning.conn();
            panic!("simulated panic while holding the db lock");
        })
        .join();
        assert!(joined.is_err(), "the spawned thread should have panicked");

        // App::conn() 应该照常拿到内部连接，而不是把中毒状态传染给所有后续请求。
        let conn = app.conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
