use std::{collections::HashMap, net::SocketAddr, time::Duration};

use axum::http::{header, HeaderMap};
use tokio::sync::Mutex;
use uuid::Uuid;

pub(crate) struct RateLimiters {
    auth: RateLimiter,
    relationship_read: RateLimiter,
    relationship_write: RateLimiter,
    chat_message: RateLimiter,
}

impl RateLimiters {
    pub(crate) fn new() -> Self {
        Self {
            auth: RateLimiter::new(60, Duration::from_secs(60)),
            relationship_read: RateLimiter::new(120, Duration::from_secs(60)),
            relationship_write: RateLimiter::new(40, Duration::from_secs(60)),
            chat_message: RateLimiter::new(20, Duration::from_secs(10)),
        }
    }

    pub(crate) async fn check_auth(
        &self,
        peer_addr: SocketAddr,
        headers: &HeaderMap,
        trust_proxy_headers: bool,
    ) -> Result<(), RateLimitExceeded> {
        self.auth
            .check(client_key(peer_addr, headers, trust_proxy_headers))
            .await
    }

    pub(crate) async fn check_relationship_read(
        &self,
        peer_addr: SocketAddr,
        headers: &HeaderMap,
        trust_proxy_headers: bool,
    ) -> Result<(), RateLimitExceeded> {
        self.relationship_read
            .check(client_key(peer_addr, headers, trust_proxy_headers))
            .await
    }

    pub(crate) async fn check_relationship_write(
        &self,
        peer_addr: SocketAddr,
        headers: &HeaderMap,
        trust_proxy_headers: bool,
    ) -> Result<(), RateLimitExceeded> {
        self.relationship_write
            .check(client_key(peer_addr, headers, trust_proxy_headers))
            .await
    }

    pub(crate) async fn check_chat_message(&self, user_id: Uuid) -> Result<(), RateLimitExceeded> {
        self.chat_message.check(format!("user:{user_id}")).await
    }
}

struct RateLimiter {
    limit: u32,
    window: Duration,
    buckets: Mutex<HashMap<String, Bucket>>,
}

struct Bucket {
    window_started: std::time::Instant,
    count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RateLimitExceeded {
    retry_after_seconds: u64,
}

impl RateLimitExceeded {
    pub(crate) fn message(&self) -> String {
        format!(
            "rate limit exceeded; retry in {} seconds",
            self.retry_after_seconds
        )
    }
}

impl RateLimiter {
    fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    async fn check(&self, key: String) -> Result<(), RateLimitExceeded> {
        let now = std::time::Instant::now();
        let mut buckets = self.buckets.lock().await;
        if buckets.len() > 10_000 {
            let stale_after = self.window + self.window;
            buckets.retain(|_, bucket| now.duration_since(bucket.window_started) <= stale_after);
        }

        let bucket = buckets.entry(key).or_insert(Bucket {
            window_started: now,
            count: 0,
        });
        let elapsed = now.duration_since(bucket.window_started);
        if elapsed >= self.window {
            bucket.window_started = now;
            bucket.count = 0;
        }

        if bucket.count >= self.limit {
            let retry_after_seconds = self.window.saturating_sub(elapsed).as_secs().max(1);
            return Err(RateLimitExceeded {
                retry_after_seconds,
            });
        }

        bucket.count += 1;
        Ok(())
    }
}

fn client_key(peer_addr: SocketAddr, headers: &HeaderMap, trust_proxy_headers: bool) -> String {
    trusted_forwarded_addr(headers, trust_proxy_headers)
        .map(|value| format!("ip:{value}"))
        .unwrap_or_else(|| format!("ip:{}", peer_addr.ip()))
}

fn trusted_forwarded_addr(headers: &HeaderMap, trust_proxy_headers: bool) -> Option<String> {
    if !trust_proxy_headers {
        return None;
    }
    forwarded_for(headers).or_else(|| header_value(headers, "x-real-ip"))
}

fn forwarded_for(headers: &HeaderMap) -> Option<String> {
    header_value(headers, "x-forwarded-for").and_then(|value| {
        value
            .split(',')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn header_value(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(name)
        .or_else(|| headers.get(header::HeaderName::from_static(name)))?
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_after_limit() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));

        assert!(limiter.check("user:a".to_string()).await.is_ok());
        assert!(limiter.check("user:a".to_string()).await.is_ok());
        assert!(limiter.check("user:a".to_string()).await.is_err());
        assert!(limiter.check("user:b".to_string()).await.is_ok());
    }

    #[test]
    fn ignores_forwarded_client_key_by_default() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.7, 10.0.0.2".parse().unwrap());

        assert_eq!(
            client_key("127.0.0.1:1234".parse().unwrap(), &headers, false),
            "ip:127.0.0.1"
        );
    }

    #[test]
    fn extracts_forwarded_client_key_when_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.7, 10.0.0.2".parse().unwrap());

        assert_eq!(
            client_key("127.0.0.1:1234".parse().unwrap(), &headers, true),
            "ip:203.0.113.7"
        );
    }

    #[test]
    fn falls_back_to_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.8".parse().unwrap());

        assert_eq!(
            client_key("127.0.0.1:1234".parse().unwrap(), &headers, true),
            "ip:203.0.113.8"
        );
    }
}
