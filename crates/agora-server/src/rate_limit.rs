use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use axum::http::{header, HeaderMap};
use tokio::sync::Mutex;
use uuid::Uuid;

pub(crate) struct RateLimiters {
    auth: RateLimiter,
    relationship_read: RateLimiter,
    relationship_write: RateLimiter,
    staff_read: RateLimiter,
    staff_write: RateLimiter,
    chat_event: RateLimiter,
    chat_message: RateLimiter,
    presence_update: RateLimiter,
}

impl RateLimiters {
    pub(crate) fn new() -> Self {
        Self {
            auth: RateLimiter::new(60, Duration::from_secs(60)),
            relationship_read: RateLimiter::new(120, Duration::from_secs(60)),
            relationship_write: RateLimiter::new(40, Duration::from_secs(60)),
            staff_read: RateLimiter::new(240, Duration::from_secs(60)),
            staff_write: RateLimiter::new(60, Duration::from_secs(60)),
            chat_event: RateLimiter::new(120, Duration::from_secs(10)),
            chat_message: RateLimiter::new(20, Duration::from_secs(10)),
            presence_update: RateLimiter::new(30, Duration::from_secs(10)),
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

    pub(crate) async fn check_staff_read(&self, user_id: Uuid) -> Result<(), RateLimitExceeded> {
        self.staff_read.check(format!("staff:{user_id}")).await
    }

    pub(crate) async fn check_staff_write(&self, user_id: Uuid) -> Result<(), RateLimitExceeded> {
        self.staff_write.check(format!("staff:{user_id}")).await
    }

    pub(crate) async fn check_chat_event(&self, user_id: Uuid) -> Result<(), RateLimitExceeded> {
        self.chat_event.check(format!("user:{user_id}")).await
    }

    pub(crate) async fn check_presence_update(
        &self,
        user_id: Uuid,
    ) -> Result<(), RateLimitExceeded> {
        self.presence_update.check(format!("user:{user_id}")).await
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
    format!("ip:{}", client_ip(peer_addr, headers, trust_proxy_headers))
}

pub(crate) fn client_ip(
    peer_addr: SocketAddr,
    headers: &HeaderMap,
    trust_proxy_headers: bool,
) -> IpAddr {
    trusted_forwarded_addr(headers, trust_proxy_headers).unwrap_or_else(|| peer_addr.ip())
}

fn trusted_forwarded_addr(headers: &HeaderMap, trust_proxy_headers: bool) -> Option<IpAddr> {
    if !trust_proxy_headers {
        return None;
    }
    forwarded_for(headers).or_else(|| header_ip(headers, "x-real-ip"))
}

fn forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    header_value(headers, "x-forwarded-for").and_then(|value| {
        value
            .split(',')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse().ok())
    })
}

fn header_ip(headers: &HeaderMap, name: &'static str) -> Option<IpAddr> {
    header_value(headers, name).and_then(|value| value.parse().ok())
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

    #[tokio::test]
    async fn keeps_staff_limits_separate_from_public_relationship_limits() {
        let limits = RateLimiters::new();
        let headers = HeaderMap::new();
        let peer_addr = "127.0.0.1:1234".parse().unwrap();

        for _ in 0..40 {
            assert!(limits
                .check_relationship_write(peer_addr, &headers, false)
                .await
                .is_ok());
        }
        assert!(limits
            .check_relationship_write(peer_addr, &headers, false)
            .await
            .is_err());
        assert!(limits.check_staff_write(Uuid::nil()).await.is_ok());
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
    fn extracts_forwarded_client_ip_when_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "127.0.0.1, 10.0.0.2".parse().unwrap());

        assert!(client_ip("10.0.0.2:1234".parse().unwrap(), &headers, true).is_loopback());
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
