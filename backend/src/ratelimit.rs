use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use time::{Duration, OffsetDateTime};

/// 可注入的时间源，让测试能拨动时钟验证窗口滚动。
pub type Clock = Arc<dyn Fn() -> OffsetDateTime + Send + Sync>;

const MAX_RATE_LIMIT_KEYS: usize = 10_000;

/// 按 key 的滑动窗口限流器，用于挡住对写接口的滥用。
/// 注意：客户端 IP 取自 X-Forwarded-For，依赖上游反向代理正确设置该头，
/// 该头可被伪造，因此这只是基础的防滥用，不是安全边界。
pub struct RateLimiter {
    limit: usize,
    window: Duration,
    now: Clock,
    max_keys: usize,
    inner: Mutex<Inner>,
}

struct Inner {
    hits: HashMap<String, Vec<OffsetDateTime>>,
    last_sweep: OffsetDateTime,
}

impl RateLimiter {
    pub fn new(limit: usize, window: Duration, now: Clock) -> Self {
        let last_sweep = now();
        Self {
            limit,
            window,
            now,
            max_keys: MAX_RATE_LIMIT_KEYS,
            inner: Mutex::new(Inner {
                hits: HashMap::new(),
                last_sweep,
            }),
        }
    }

    /// 记录一次访问并返回是否在限额内。
    pub fn allow(&self, key: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let now = (self.now)();
        let cutoff = now - self.window;

        if now - inner.last_sweep >= self.window {
            sweep(&mut inner.hits, cutoff);
            inner.last_sweep = now;
        }

        // 伪造大量来源地址时，不允许 map 无界增长。容量耗尽后的新地址共享一个保守桶，
        // 老地址仍按各自窗口正常计算。
        let key = if !inner.hits.contains_key(key) && inner.hits.len() >= self.max_keys {
            "__overflow__"
        } else {
            key
        };

        let times = inner.hits.entry(key.to_string()).or_default();
        times.retain(|t| *t > cutoff);
        if times.len() >= self.limit {
            return false;
        }
        times.push(now);
        true
    }
}

/// 删除窗口内已无访问记录的 key，避免 map 无限增长。
fn sweep(hits: &mut HashMap<String, Vec<OffsetDateTime>>, cutoff: OffsetDateTime) {
    hits.retain(|_, times| {
        times.retain(|t| *t > cutoff);
        !times.is_empty()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn bounds_tracked_keys() {
        let now = datetime!(2026-06-22 12:00 UTC);
        let mut limiter = RateLimiter::new(1, Duration::MINUTE, Arc::new(move || now));
        limiter.max_keys = 2;

        assert!(
            limiter.allow("one"),
            "first request for each bucket should pass"
        );
        assert!(
            limiter.allow("two"),
            "first request for each bucket should pass"
        );
        assert!(
            limiter.allow("three"),
            "first request for each bucket should pass"
        );
        assert!(
            !limiter.allow("four"),
            "new keys should share the full overflow bucket"
        );
        assert_eq!(limiter.inner.lock().unwrap().hits.len(), 3);
    }

    #[test]
    fn window_slides() {
        let now = Arc::new(Mutex::new(datetime!(2026-06-22 12:00 UTC)));
        let clock = now.clone();
        let limiter = RateLimiter::new(
            2,
            Duration::MINUTE,
            Arc::new(move || *clock.lock().unwrap()),
        );

        assert!(limiter.allow("ip"));
        assert!(limiter.allow("ip"));
        assert!(!limiter.allow("ip"));

        *now.lock().unwrap() += Duration::seconds(61);
        assert!(limiter.allow("ip"));
    }
}
