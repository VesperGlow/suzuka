use std::str::FromStr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::Duration;

use crate::httputil::{internal_error, valid_post_url, write_error, write_json, ClientIp};
use crate::server::App;

/// 每个来源 IP 在 POST_WINDOW 内最多允许 POST_BURST 次留言提交。
pub const POST_BURST: usize = 5;
pub const POST_WINDOW: Duration = Duration::MINUTE;
const MESSAGE_PAGE_SIZE: i64 = 50;
const MAX_MESSAGE_PAGE_SIZE: i64 = 100;
const LEGACY_MESSAGE_LIMIT: i64 = 500;

#[derive(Serialize)]
pub struct Message {
    pub id: i64,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub website: String,
    pub content: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ref_title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ref_url: String,
    pub created_at: String,
}

#[derive(Serialize)]
struct MessagePage {
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "is_zero")]
    next_before_id: i64,
    total_count: i64,
}

fn is_zero(value: &i64) -> bool {
    *value == 0
}

/// 请求体结构。id / created_at 是响应里的已知字段，客户端传来时接受但忽略
/// （与旧实现一致）；其余未知字段一律拒绝。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageInput {
    #[serde(default)]
    #[allow(dead_code)]
    id: Option<i64>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    website: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    ref_title: String,
    #[serde(default)]
    ref_url: String,
    #[serde(default)]
    #[allow(dead_code)]
    created_at: Option<String>,
}

pub async fn list_messages(State(app): State<Arc<App>>, RawQuery(raw): RawQuery) -> Response {
    let raw = raw.unwrap_or_default();
    let params = parse_query(&raw);
    let paginated = params.iter().any(|(k, _)| k == "limit" || k == "before_id");

    let (mut limit, mut before_id) = (LEGACY_MESSAGE_LIMIT, 0i64);
    if paginated {
        match message_page_params(&params) {
            Ok(parsed) => (limit, before_id) = parsed,
            Err(text) => return write_error(StatusCode::BAD_REQUEST, &text),
        }
    }

    let conn = app.conn();
    let result = if before_id > 0 {
        query_messages(
            &conn,
            "SELECT id, name, email, website, content, ref_title, ref_url, created_at
FROM messages
WHERE id < ?1
ORDER BY id DESC
LIMIT ?2",
            params![before_id, limit + 1],
        )
    } else {
        query_messages(
            &conn,
            "SELECT id, name, email, website, content, ref_title, ref_url, created_at
FROM messages
ORDER BY id DESC
LIMIT ?1",
            params![limit + 1],
        )
    };
    let mut messages = match result {
        Ok(messages) => messages,
        Err(err) => return internal_error("unable to load messages", &err),
    };

    if !paginated {
        messages.truncate(limit as usize);
        return write_json(StatusCode::OK, &messages);
    }

    let mut next_before_id = 0;
    if messages.len() > limit as usize {
        messages.truncate(limit as usize);
        next_before_id = messages.last().map_or(0, |m| m.id);
    }
    let total_count: i64 =
        match conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0)) {
            Ok(count) => count,
            Err(err) => return internal_error("unable to load messages", &err),
        };

    write_json(
        StatusCode::OK,
        &MessagePage {
            messages,
            next_before_id,
            total_count,
        },
    )
}

fn query_messages(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Message>, rusqlite::Error> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |row| {
        Ok(Message {
            id: row.get(0)?,
            name: row.get(1)?,
            // email 是私密字段，仅入库不外发。
            email: String::new(),
            website: row.get(3)?,
            content: row.get(4)?,
            ref_title: row.get(5)?,
            ref_url: row.get(6)?,
            created_at: row.get(7)?,
        })
    })?;
    rows.collect()
}

fn parse_query(raw: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(raw.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn message_page_params(params: &[(String, String)]) -> Result<(i64, i64), String> {
    let first = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.trim().to_string())
    };

    let mut limit = MESSAGE_PAGE_SIZE;
    if let Some(value) = first("limit").filter(|v| !v.is_empty()) {
        limit = value
            .parse()
            .ok()
            .filter(|v| (1..=MAX_MESSAGE_PAGE_SIZE).contains(v))
            .ok_or(format!(
                "limit must be between 1 and {MAX_MESSAGE_PAGE_SIZE}"
            ))?;
    }

    let mut before_id = 0;
    if let Some(value) = first("before_id").filter(|v| !v.is_empty()) {
        before_id = value
            .parse()
            .ok()
            .filter(|v| *v >= 1)
            .ok_or("before_id must be a positive integer".to_string())?;
    }
    Ok((limit, before_id))
}

pub async fn create_message(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(limiter) = &app.limiter {
        if !limiter.allow(&ip) {
            return too_many_requests(POST_WINDOW);
        }
    }

    if !is_json_content_type(&headers) {
        return write_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "content type must be application/json",
        );
    }

    // serde_json 对未知字段（deny_unknown_fields）、尾随内容与非法 UTF-8 都会报错，
    // 与旧实现的 DisallowUnknownFields + ensureJSONEnd 等价。
    let Ok(input) = serde_json::from_slice::<MessageInput>(&body) else {
        return write_error(StatusCode::BAD_REQUEST, "invalid request body");
    };

    let mut item = Message {
        id: 0,
        name: input.name.trim().to_string(),
        email: input.email.trim().to_string(),
        website: input.website.trim().to_string(),
        content: input.content.trim().to_string(),
        ref_title: input.ref_title.trim().to_string(),
        ref_url: input.ref_url.trim().to_string(),
        created_at: String::new(),
    };
    if let Err(text) = validate_message(&item) {
        return write_error(StatusCode::BAD_REQUEST, &text);
    }

    item.created_at = (app.now)()
        .to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .unwrap_or_default();

    let conn = app.conn();
    if let Err(err) = conn.execute(
        "INSERT INTO messages (name, email, website, content, ref_title, ref_url, created_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            item.name,
            item.email,
            item.website,
            item.content,
            item.ref_title,
            item.ref_url,
            item.created_at
        ],
    ) {
        return internal_error("unable to save message", &err);
    }
    item.id = conn.last_insert_rowid();
    item.email = String::new();

    write_json(StatusCode::CREATED, &item)
}

pub fn is_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<mime::Mime>().ok())
        .is_some_and(|m| m.essence_str() == "application/json")
}

pub fn too_many_requests(window: Duration) -> Response {
    let mut response = write_error(
        StatusCode::TOO_MANY_REQUESTS,
        "too many requests, please try again later",
    );
    response.headers_mut().insert(
        header::RETRY_AFTER,
        window.whole_seconds().to_string().parse().unwrap(),
    );
    response
}

fn validate_message(item: &Message) -> Result<(), String> {
    if item.name.is_empty() {
        return Err("name is required".to_string());
    }
    if item.content.is_empty() {
        return Err("content is required".to_string());
    }
    if item.ref_title.is_empty() != item.ref_url.is_empty() {
        return Err("ref_title and ref_url must be provided together".to_string());
    }
    if !item.ref_url.is_empty() && !valid_post_url(&item.ref_url) {
        return Err("ref_url must be a relative /posts/ path".to_string());
    }
    if !item.email.is_empty() && email_address::EmailAddress::from_str(&item.email).is_err() {
        return Err("email must be a valid email address".to_string());
    }
    if !item.website.is_empty() && !valid_website(&item.website) {
        return Err("website must be an http or https URL".to_string());
    }

    let limits = [
        ("name", &item.name, 40),
        ("email", &item.email, 120),
        ("website", &item.website, 200),
        ("content", &item.content, 2000),
        ("ref_title", &item.ref_title, 200),
        ("ref_url", &item.ref_url, 300),
    ];
    for (name, value, max) in limits {
        // UTF-8 合法性由 String 类型本身保证，这里只需按字符数（rune）限长。
        if value.chars().count() > max {
            return Err(format!("{name} must be at most {max} characters"));
        }
    }
    Ok(())
}

fn valid_website(value: &str) -> bool {
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    (parsed.scheme() == "http" || parsed.scheme() == "https")
        && parsed.host_str().is_some_and(|h| !h.is_empty())
        && parsed.username().is_empty()
        && parsed.password().is_none()
}
