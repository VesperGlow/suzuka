use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use suzuka_backend::backup;
use suzuka_backend::db::{open_database, BoxError};
use suzuka_backend::notify::{Notifier, SmtpConfig};
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

    // 管理 token：配置后开启 DELETE /messages/{id}（无 WebUI，curl 管理留言用）。
    app.admin_token = optional_env("GUESTBOOK_ADMIN_TOKEN");
    if app.admin_token.is_some() {
        println!("admin message deletion enabled (GUESTBOOK_ADMIN_TOKEN is set)");
    }

    // 新留言邮件通知（可选）：USER/PASSWORD 成对出现才开启，TO 默认发给自己。
    let smtp_user = optional_env("GUESTBOOK_SMTP_USER");
    let smtp_password = optional_env("GUESTBOOK_SMTP_PASSWORD");
    if smtp_user.is_some() != smtp_password.is_some() {
        return Err("GUESTBOOK_SMTP_USER and GUESTBOOK_SMTP_PASSWORD must be set together".into());
    }
    if let (Some(user), Some(password)) = (smtp_user, smtp_password) {
        let to = optional_env("GUESTBOOK_NOTIFY_TO").unwrap_or_else(|| user.clone());
        let config = SmtpConfig {
            relay: env_or_default("GUESTBOOK_SMTP_RELAY", "smtp.gmail.com"),
            user,
            password,
            to: to.clone(),
        };
        app.notifier = Some(Arc::new(Notifier::new(config, app.now.clone())?));
        println!("new-message email notifications enabled (to {to})");
    }

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

    // 每日数据库一致性快照（VACUUM INTO）：WAL 下直接复制 db 文件可能拿到
    // 半成品，落快照后直接备份整个数据目录即可。默认写到 <db 目录>/backups/。
    let db_dir = Path::new(&db_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let backup_dir = optional_env("GUESTBOOK_BACKUP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| db_dir.join("backups"));
    println!("daily database snapshots to {}", backup_dir.display());
    backup::spawn(app.clone(), backup_dir);

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
    optional_env(key).unwrap_or_else(|| fallback.to_string())
}

fn optional_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
