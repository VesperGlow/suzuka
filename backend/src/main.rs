use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use suzuka_backend::db::{open_database, BoxError};
use suzuka_backend::server::{root_handler, App};
use suzuka_backend::static_site::scan_post_paths;

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), BoxError> {
    let addr = env_or_default("GUESTBOOK_ADDR", "127.0.0.1:8787");
    let db_path = env_or_default(
        "GUESTBOOK_DB_PATH",
        &PathBuf::from("data").join("guestbook.db").to_string_lossy(),
    );

    let db = open_database(db_path.as_ref())?;
    let mut app = App::new(db);

    let static_dir = std::env::var("GUESTBOOK_STATIC_DIR")
        .ok()
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty());
    if let Some(dir) = &static_dir {
        // 单容器模式：从静态产物扫出真实文章路径，作为计数与留言引用的白名单。
        let paths = scan_post_paths(PathBuf::from(dir).as_path())
            .map_err(|e| format!("scan post paths under {dir}: {e}"))?;
        if paths.is_empty() {
            eprintln!("warning: no post paths found under {dir}, counters will reject all paths");
        }
        println!("post path whitelist: {} posts", paths.len());
        app.allowed_paths = Some(paths);
    }
    let app = Arc::new(app);
    let router = root_handler(app, static_dir.as_deref());

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("listen on {addr}: {e}"))?;
    println!("guestbook service listening on {addr} (database: {db_path})");

    // 收到终止信号时停止接收新请求，并给在途请求留出完成时间。
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .map_err(|e| format!("serve HTTP: {e}"))?;
    Ok(())
}

async fn shutdown_signal() {
    let interrupt = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install interrupt handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = interrupt => {}
        _ = terminate => {}
    }
    println!("shutdown signal received");
}

fn env_or_default(key: &str, fallback: &str) -> String {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => fallback.to_string(),
    }
}
