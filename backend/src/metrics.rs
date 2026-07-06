use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::Duration;

use crate::guestbook::{is_json_content_type, too_many_requests};
use crate::httputil::{internal_error, valid_post_url, write_error, write_json, ClientIp};
use crate::server::App;

/// 阅读数 / 喜欢这类计数写接口的限流：正常浏览也会触发，因此放得比留言宽松，
/// 仅用于挡住单 IP 的刷量；真正的防自刷靠前端 localStorage 节流。
pub const COUNTER_BURST: usize = 60;
pub const COUNTER_WINDOW: Duration = Duration::MINUTE;

/// 阅读数 / 喜欢计数接口的统一返回体。
#[derive(Serialize)]
struct CounterResponse {
    path: String,
    count: i64,
}

// 把某张计数表（page_views / reactions）的表名绑定进处理器。
// 表名是代码内常量，不来自用户输入，可安全拼入 SQL。

pub async fn read_views(state: State<Arc<App>>, query: RawQuery) -> Response {
    read_counter(state, query, "page_views").await
}

pub async fn bump_views(
    state: State<Arc<App>>,
    ip: ClientIp,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    bump_counter(state, ip, headers, body, "page_views").await
}

pub async fn read_reactions(state: State<Arc<App>>, query: RawQuery) -> Response {
    read_counter(state, query, "reactions").await
}

pub async fn bump_reactions(
    state: State<Arc<App>>,
    ip: ClientIp,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    bump_counter(state, ip, headers, body, "reactions").await
}

async fn read_counter(
    State(app): State<Arc<App>>,
    RawQuery(raw): RawQuery,
    table: &str,
) -> Response {
    let raw = raw.unwrap_or_default();
    let page_path = url::form_urlencoded::parse(raw.as_bytes())
        .find(|(k, _)| k == "path")
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_default();
    if !valid_post_url(&page_path) {
        return write_error(
            StatusCode::BAD_REQUEST,
            "path must be a relative /posts/ path",
        );
    }

    let conn = app.conn();
    let count: Option<i64> = match conn
        .query_row(
            &format!("SELECT count FROM {table} WHERE path = ?1"),
            [&page_path],
            |row| row.get(0),
        )
        .optional()
    {
        Ok(count) => count,
        Err(err) => return internal_error("unable to load count", &err),
    };

    write_json(
        StatusCode::OK,
        &CounterResponse {
            path: page_path,
            count: count.unwrap_or(0),
        },
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BumpInput {
    #[serde(default)]
    path: String,
}

async fn bump_counter(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    body: Bytes,
    table: &str,
) -> Response {
    if let Some(limiter) = &app.counter_limiter {
        if !limiter.allow(&ip) {
            return too_many_requests(COUNTER_WINDOW);
        }
    }

    if !is_json_content_type(&headers) {
        return write_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content type must be application/json",
        );
    }

    let Ok(input) = serde_json::from_slice::<BumpInput>(&body) else {
        return write_error(StatusCode::BAD_REQUEST, "invalid request body");
    };

    let page_path = input.path.trim().to_string();
    if !valid_post_url(&page_path) {
        return write_error(
            StatusCode::BAD_REQUEST,
            "path must be a relative /posts/ path",
        );
    }

    let conn = app.conn();
    let count: i64 = match conn.query_row(
        &format!(
            "INSERT INTO {table} (path, count) VALUES (?1, 1)
ON CONFLICT(path) DO UPDATE SET count = count + 1
RETURNING count"
        ),
        [&page_path],
        |row| row.get(0),
    ) {
        Ok(count) => count,
        Err(err) => return internal_error("unable to update count", &err),
    };

    write_json(
        StatusCode::OK,
        &CounterResponse {
            path: page_path,
            count,
        },
    )
}

/// 返回全站累计的阅读量与喜欢数，供「关于」页展示。只读，无需限流。
/// 路由已限定为 GET，方法校验交给路由层。
pub async fn handle_summary(State(app): State<Arc<App>>) -> Response {
    let conn = app.conn();
    let views: i64 = match conn.query_row(
        "SELECT COALESCE(SUM(count), 0) FROM page_views",
        [],
        |row| row.get(0),
    ) {
        Ok(count) => count,
        Err(err) => return internal_error("unable to load summary", &err),
    };
    let reactions: i64 =
        match conn.query_row("SELECT COALESCE(SUM(count), 0) FROM reactions", [], |row| {
            row.get(0)
        }) {
            Ok(count) => count,
            Err(err) => return internal_error("unable to load summary", &err),
        };

    write_json(
        StatusCode::OK,
        &json!({ "views": views, "reactions": reactions }),
    )
}
