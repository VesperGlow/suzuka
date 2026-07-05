use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde::Deserialize;
use serde_json::json;
use tempfile::TempDir;
use time::macros::datetime;
use time::Duration;
use tower::ServiceExt;

use suzuka_backend::db::{open_database, CURRENT_SCHEMA_VERSION};
use suzuka_backend::guestbook::{POST_BURST, POST_WINDOW};
use suzuka_backend::metrics::{COUNTER_BURST, COUNTER_WINDOW};
use suzuka_backend::ratelimit::{Clock, RateLimiter};
use suzuka_backend::server::{handler, App};

fn test_app() -> (Router, TempDir) {
    let tmp = TempDir::new().unwrap();
    let db = open_database(&tmp.path().join("guestbook.db")).unwrap();
    let clock: Clock = Arc::new(|| datetime!(2026-06-22 12:00 UTC));
    let app = Arc::new(App {
        db: Mutex::new(db),
        now: clock.clone(),
        limiter: None,
        counter_limiter: Some(RateLimiter::new(COUNTER_BURST, COUNTER_WINDOW, clock)),
    });
    (handler(app), tmp)
}

async fn send(router: &Router, request: Request<Body>) -> (StatusCode, Vec<u8>) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body.to_vec())
}

fn post_json(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

#[derive(Deserialize, Debug)]
struct Message {
    id: i64,
    #[serde(default)]
    email: String,
    #[serde(default)]
    website: String,
    content: String,
    #[serde(default)]
    ref_url: String,
}

#[derive(Deserialize, Debug)]
struct MessagePage {
    messages: Vec<Message>,
    #[serde(default)]
    next_before_id: i64,
    total_count: i64,
}

#[derive(Deserialize, Debug)]
struct Counter {
    count: i64,
}

#[tokio::test]
async fn create_and_list_messages() {
    let (router, _tmp) = test_app();
    let body = r#"{"name":"Suzuka","email":"suzuka@example.com","website":"https://example.com","content":"hello <b>world</b>","ref_title":"An article","ref_url":"/posts/example/"}"#;
    let (status, response) = send(&router, post_json("/messages", body)).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "POST body = {}",
        String::from_utf8_lossy(&response)
    );

    let (status, response) = send(&router, get("/messages?limit=50")).await;
    assert_eq!(status, StatusCode::OK);
    let page: MessagePage = serde_json::from_slice(&response).unwrap();
    assert_eq!(page.total_count, 1);
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].content, "hello <b>world</b>");
    assert_eq!(page.messages[0].ref_url, "/posts/example/");
    assert_eq!(page.messages[0].email, "", "email must stay private");
    assert_eq!(page.messages[0].website, "https://example.com");

    // 不带分页参数时保持旧的裸数组响应。
    let (status, response) = send(&router, get("/messages")).await;
    assert_eq!(status, StatusCode::OK);
    let legacy: Vec<Message> = serde_json::from_slice(&response).unwrap();
    assert_eq!(legacy.len(), 1);
}

#[tokio::test]
async fn message_pagination() {
    let (router, _tmp) = test_app();
    for i in 0..55 {
        let (status, response) = send(
            &router,
            post_json("/messages", r#"{"name":"Suzuka","content":"message"}"#),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "POST {} body = {}",
            i + 1,
            String::from_utf8_lossy(&response)
        );
    }

    let load = |target: String| {
        let router = router.clone();
        async move {
            let (status, response) = send(&router, get(&target)).await;
            assert_eq!(status, StatusCode::OK, "GET {target}");
            serde_json::from_slice::<MessagePage>(&response).unwrap()
        }
    };

    let first = load("/messages?limit=20".to_string()).await;
    assert_eq!(first.total_count, 55);
    assert_eq!(first.messages.len(), 20);
    assert_ne!(first.next_before_id, 0);

    let second = load(format!(
        "/messages?limit=20&before_id={}",
        first.next_before_id
    ))
    .await;
    assert_eq!(second.total_count, 55);
    assert_eq!(second.messages.len(), 20);
    assert!(second.messages[0].id < first.messages.last().unwrap().id);

    let third = load(format!(
        "/messages?limit=20&before_id={}",
        second.next_before_id
    ))
    .await;
    assert_eq!(third.messages.len(), 15);
    assert_eq!(third.next_before_id, 0);
}

#[tokio::test]
async fn message_pagination_rejects_bad_parameters() {
    let (router, _tmp) = test_app();
    for target in [
        "/messages?limit=0",
        "/messages?limit=101",
        "/messages?limit=nope",
        "/messages?before_id=0",
        "/messages?before_id=nope",
    ] {
        let (status, _) = send(&router, get(target)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "GET {target}");
    }
}

#[tokio::test]
async fn rate_limit() {
    let now = Arc::new(Mutex::new(datetime!(2026-06-22 12:00 UTC)));
    let clock_source = now.clone();
    let clock: Clock = Arc::new(move || *clock_source.lock().unwrap());

    let tmp = TempDir::new().unwrap();
    let db = open_database(&tmp.path().join("guestbook.db")).unwrap();
    let app = Arc::new(App {
        db: Mutex::new(db),
        now: clock.clone(),
        limiter: Some(RateLimiter::new(POST_BURST, POST_WINDOW, clock)),
        counter_limiter: None,
    });
    let router = handler(app);

    let post = |router: Router| async move {
        let mut request = post_json("/messages", r#"{"name":"Suzuka","content":"hello"}"#);
        request
            .headers_mut()
            .insert("x-forwarded-for", "203.0.113.7".parse().unwrap());
        // 直连方是私网反代，X-Forwarded-For 才会被采信。
        request
            .extensions_mut()
            .insert(ConnectInfo::<SocketAddr>("10.0.0.2:1234".parse().unwrap()));
        let (status, _) = send(&router, request).await;
        status
    };

    for i in 0..POST_BURST {
        assert_eq!(
            post(router.clone()).await,
            StatusCode::CREATED,
            "request {}",
            i + 1
        );
    }
    assert_eq!(post(router.clone()).await, StatusCode::TOO_MANY_REQUESTS);

    *now.lock().unwrap() += POST_WINDOW + Duration::SECOND;
    assert_eq!(post(router.clone()).await, StatusCode::CREATED);
}

async fn bump(router: &Router, endpoint: &str, path: &str) -> Counter {
    let body = json!({ "path": path }).to_string();
    let (status, response) = send(router, post_json(endpoint, &body)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "POST {endpoint} body = {}",
        String::from_utf8_lossy(&response)
    );
    serde_json::from_slice(&response).unwrap()
}

async fn read(router: &Router, endpoint: &str, path: &str) -> Counter {
    let (status, response) = send(router, get(&format!("{endpoint}?path={path}"))).await;
    assert_eq!(status, StatusCode::OK, "GET {endpoint}");
    serde_json::from_slice(&response).unwrap()
}

#[tokio::test]
async fn counters() {
    let (router, _tmp) = test_app();

    // 未记录过的文章读数为 0。
    assert_eq!(read(&router, "/views", "/posts/example/").await.count, 0);
    // 连续自增。
    assert_eq!(bump(&router, "/views", "/posts/example/").await.count, 1);
    assert_eq!(bump(&router, "/views", "/posts/example/").await.count, 2);
    assert_eq!(read(&router, "/views", "/posts/example/").await.count, 2);
    // 反应与阅读数互不影响，且按 path 独立计数。
    assert_eq!(
        bump(&router, "/reactions", "/posts/example/").await.count,
        1
    );
    assert_eq!(read(&router, "/views", "/posts/example/").await.count, 2);
    assert_eq!(read(&router, "/views", "/posts/other/").await.count, 0);
}

#[tokio::test]
async fn summary() {
    let (router, _tmp) = test_app();

    let summary = |router: Router| async move {
        let (status, response) = send(&router, get("/summary")).await;
        assert_eq!(status, StatusCode::OK);
        let out: serde_json::Value = serde_json::from_slice(&response).unwrap();
        (
            out["views"].as_i64().unwrap(),
            out["reactions"].as_i64().unwrap(),
        )
    };

    // 空库汇总为 0。
    assert_eq!(summary(router.clone()).await, (0, 0));
    // 跨多篇文章累加。
    bump(&router, "/views", "/posts/a/").await;
    bump(&router, "/views", "/posts/a/").await;
    bump(&router, "/views", "/posts/b/").await;
    bump(&router, "/reactions", "/posts/a/").await;
    assert_eq!(summary(router).await, (3, 1));
}

#[tokio::test]
async fn counter_rejects_bad_path() {
    let (router, _tmp) = test_app();
    let cases: Vec<(&str, Request<Body>)> = vec![
        ("get missing path", get("/views")),
        (
            "get external",
            get("/views?path=https://example.com/posts/a/"),
        ),
        ("get non-post", get("/reactions?path=/about/")),
        (
            "post non-post",
            post_json("/views", r#"{"path":"/about/"}"#),
        ),
        (
            "post traversal",
            post_json("/reactions", r#"{"path":"/posts/../about/"}"#),
        ),
        (
            "post unknown field",
            post_json("/views", r#"{"path":"/posts/a/","x":1}"#),
        ),
    ];
    for (name, request) in cases {
        let (status, response) = send(&router, request).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{name}: body = {}",
            String::from_utf8_lossy(&response)
        );
    }
}

#[tokio::test]
async fn validation() {
    let (router, _tmp) = test_app();
    let cases = [
        ("missing name", r#"{"name":"","content":"hello"}"#.to_string()),
        ("missing content", r#"{"name":"Suzuka","content":""}"#.to_string()),
        (
            "name too long",
            format!(r#"{{"name":"{}","content":"hello"}}"#, "界".repeat(41)),
        ),
        (
            "content too long",
            format!(r#"{{"name":"Suzuka","content":"{}"}}"#, "a".repeat(2001)),
        ),
        (
            "reference title only",
            r#"{"name":"Suzuka","content":"hello","ref_title":"Article"}"#.to_string(),
        ),
        (
            "external reference",
            r#"{"name":"Suzuka","content":"hello","ref_title":"Article","ref_url":"https://example.com/posts/a/"}"#.to_string(),
        ),
        (
            "non-post reference",
            r#"{"name":"Suzuka","content":"hello","ref_title":"Article","ref_url":"/about/"}"#.to_string(),
        ),
        (
            "traversing reference",
            r#"{"name":"Suzuka","content":"hello","ref_title":"Article","ref_url":"/posts/../about/"}"#.to_string(),
        ),
        (
            "invalid email",
            r#"{"name":"Suzuka","content":"hello","email":"not-an-email"}"#.to_string(),
        ),
        (
            "unsafe website",
            r#"{"name":"Suzuka","content":"hello","website":"javascript:alert(1)"}"#.to_string(),
        ),
        (
            "website credentials",
            r#"{"name":"Suzuka","content":"hello","website":"https://user:pass@example.com"}"#.to_string(),
        ),
        (
            "unknown field",
            r#"{"name":"Suzuka","content":"hello","admin":true}"#.to_string(),
        ),
    ];
    for (name, body) in cases {
        let (status, response) = send(&router, post_json("/messages", &body)).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{name}: body = {}",
            String::from_utf8_lossy(&response)
        );
    }
}

#[tokio::test]
async fn wrong_content_type_rejected() {
    let (router, _tmp) = test_app();
    let request = Request::builder()
        .method("POST")
        .uri("/messages")
        .header("content-type", "text/plain")
        .body(Body::from(r#"{"name":"Suzuka","content":"hello"}"#))
        .unwrap();
    let (status, _) = send(&router, request).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[test]
fn database_schema_version() {
    let tmp = TempDir::new().unwrap();
    let db = open_database(&tmp.path().join("guestbook.db")).unwrap();
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
}
