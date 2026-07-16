//! 新留言的邮件通知（可选）。
//!
//! SMTP 未配置（GUESTBOOK_SMTP_USER / GUESTBOOK_SMTP_PASSWORD 未设置）时整个
//! 模块不参与运行。发信是尽力而为：在独立的 tokio 任务里异步进行，任何失败
//! 只写日志，绝不影响留言接口本身的响应。

use lettre::message::header::ContentType;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use time::Duration;

use crate::httputil::log_timestamp;
use crate::ratelimit::{Clock, RateLimiter};

/// 每小时最多发出的通知邮件数（全局，不分来源 IP）。超出的留言不再发信，
/// 但照常入库——这是防邮件风暴的保险丝，不是业务限制；漏掉的留言仍能在
/// 留言板页面或 GET /messages 里看到。
const NOTIFY_BURST: usize = 12;
const NOTIFY_WINDOW: Duration = Duration::HOUR;

pub struct SmtpConfig {
    /// SMTP 服务器主机名，走 465 端口的隐式 TLS（SMTPS）。
    pub relay: String,
    /// 登录用户名，同时用作发件人地址（Gmail 会强制改写 From 为登录账号）。
    pub user: String,
    /// Gmail 用 App Password（需要先开两步验证），不是账号密码。
    pub password: String,
    /// 收件人，默认发给自己（= user）。
    pub to: String,
}

pub struct Notifier {
    mailer: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    to: Mailbox,
    limiter: RateLimiter,
}

impl Notifier {
    pub fn new(config: SmtpConfig, clock: Clock) -> Result<Self, String> {
        let from: Mailbox = config
            .user
            .parse()
            .map_err(|e| format!("GUESTBOOK_SMTP_USER must be an email address: {e}"))?;
        let to: Mailbox = config
            .to
            .parse()
            .map_err(|e| format!("GUESTBOOK_NOTIFY_TO must be an email address: {e}"))?;
        let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay(&config.relay)
            .map_err(|e| format!("SMTP relay {}: {e}", config.relay))?
            .credentials(Credentials::new(config.user, config.password))
            .build();
        Ok(Self {
            mailer,
            from,
            to,
            limiter: RateLimiter::new(NOTIFY_BURST, NOTIFY_WINDOW, clock),
        })
    }

    /// 发送一封通知邮件。调用方应把它 spawn 到独立任务里，不要 await 在请求路径上。
    pub async fn send(&self, subject: String, body: String) {
        if !self.limiter.allow("notify") {
            eprintln!(
                "{} notify: hourly email cap reached, skipped \"{subject}\"",
                log_timestamp()
            );
            return;
        }
        let email = match Message::builder()
            .from(self.from.clone())
            .to(self.to.clone())
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body)
        {
            Ok(email) => email,
            Err(err) => {
                eprintln!("{} notify: build email: {err}", log_timestamp());
                return;
            }
        };
        if let Err(err) = self.mailer.send(email).await {
            eprintln!("{} notify: send email: {err}", log_timestamp());
        }
    }
}
