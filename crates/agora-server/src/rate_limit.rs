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
    chat_frame: RateLimiter,
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
            chat_frame: RateLimiter::new(240, Duration::from_secs(10)),
            chat_event: RateLimiter::new(120, Duration::from_secs(10)),
            chat_message: RateLimiter::new(20, Duration::from_secs(10)),
            presence_update: RateLimiter::new(30, Duration::from_secs(10)),
        }
    }

    pub(crate) async fn check_auth(
        &self,
        peer_addr: SocketAddr,
        headers: &HeaderMap,
        trusted_proxy_cidrs: &[TrustedProxy],
    ) -> Result<(), RateLimitExceeded> {
        self.auth
            .check(client_key(peer_addr, headers, trusted_proxy_cidrs))
            .await
    }

    pub(crate) async fn check_relationship_read(
        &self,
        peer_addr: SocketAddr,
        headers: &HeaderMap,
        trusted_proxy_cidrs: &[TrustedProxy],
    ) -> Result<(), RateLimitExceeded> {
        self.relationship_read
            .check(client_key(peer_addr, headers, trusted_proxy_cidrs))
            .await
    }

    pub(crate) async fn check_relationship_write(
        &self,
        peer_addr: SocketAddr,
        headers: &HeaderMap,
        trusted_proxy_cidrs: &[TrustedProxy],
    ) -> Result<(), RateLimitExceeded> {
        self.relationship_write
            .check(client_key(peer_addr, headers, trusted_proxy_cidrs))
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

    pub(crate) async fn check_chat_frame(&self, user_id: Uuid) -> Result<(), RateLimitExceeded> {
        self.chat_frame.check(format!("user:{user_id}")).await
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TrustedProxy {
    network: IpAddr,
    prefix_len: u8,
}

impl TrustedProxy {
    fn contains(self, address: IpAddr) -> bool {
        match (self.network, address) {
            (IpAddr::V4(network), IpAddr::V4(address)) => {
                let mask = prefix_mask_u32(self.prefix_len);
                u32::from(network) & mask == u32::from(address) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(address)) => {
                let mask = prefix_mask_u128(self.prefix_len);
                u128::from(network) & mask == u128::from(address) & mask
            }
            _ => false,
        }
    }
}

pub(crate) fn parse_trusted_proxy_cidrs(value: Option<&str>) -> Result<Vec<TrustedProxy>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };

    value
        .split(',')
        .map(str::trim)
        .map(parse_trusted_proxy_cidr)
        .collect()
}

fn parse_trusted_proxy_cidr(value: &str) -> Result<TrustedProxy, String> {
    if value.is_empty() {
        return Err("trusted proxy CIDRs cannot contain empty entries".to_string());
    }

    let (address, prefix_len) = match value.split_once('/') {
        Some((address, prefix_len)) => (address, Some(prefix_len)),
        None => (value, None),
    };
    let network = address
        .parse::<IpAddr>()
        .map_err(|_| format!("invalid trusted proxy address: {value}"))?;
    let max_prefix_len = match network {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };
    let prefix_len = match prefix_len {
        Some(prefix_len) => prefix_len
            .parse::<u8>()
            .map_err(|_| format!("invalid trusted proxy CIDR prefix: {value}"))?,
        None => max_prefix_len,
    };
    if prefix_len > max_prefix_len {
        return Err(format!(
            "trusted proxy CIDR prefix is out of range: {value}"
        ));
    }

    Ok(TrustedProxy {
        network,
        prefix_len,
    })
}

fn prefix_mask_u32(prefix_len: u8) -> u32 {
    if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_len)
    }
}

fn prefix_mask_u128(prefix_len: u8) -> u128 {
    if prefix_len == 0 {
        0
    } else {
        u128::MAX << (128 - prefix_len)
    }
}

fn client_key(
    peer_addr: SocketAddr,
    headers: &HeaderMap,
    trusted_proxy_cidrs: &[TrustedProxy],
) -> String {
    format!("ip:{}", client_ip(peer_addr, headers, trusted_proxy_cidrs))
}

pub(crate) fn client_ip(
    peer_addr: SocketAddr,
    headers: &HeaderMap,
    trusted_proxy_cidrs: &[TrustedProxy],
) -> IpAddr {
    trusted_forwarded_addr(peer_addr, headers, trusted_proxy_cidrs)
        .unwrap_or_else(|| peer_addr.ip())
}

fn trusted_forwarded_addr(
    peer_addr: SocketAddr,
    headers: &HeaderMap,
    trusted_proxy_cidrs: &[TrustedProxy],
) -> Option<IpAddr> {
    if !is_trusted_proxy(peer_addr.ip(), trusted_proxy_cidrs) {
        return None;
    }
    forwarded_for(headers, trusted_proxy_cidrs).or_else(|| header_ip(headers, "x-real-ip"))
}

fn is_trusted_proxy(address: IpAddr, trusted_proxy_cidrs: &[TrustedProxy]) -> bool {
    trusted_proxy_cidrs
        .iter()
        .any(|trusted_proxy| trusted_proxy.contains(address))
}

fn forwarded_for(headers: &HeaderMap, trusted_proxy_cidrs: &[TrustedProxy]) -> Option<IpAddr> {
    header_value(headers, "x-forwarded-for").and_then(|value| {
        for value in value.split(',').rev().map(str::trim) {
            let address = value.parse().ok()?;
            if !is_trusted_proxy(address, trusted_proxy_cidrs) {
                return Some(address);
            }
        }
        None
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
                .check_relationship_write(peer_addr, &headers, &[])
                .await
                .is_ok());
        }
        assert!(limits
            .check_relationship_write(peer_addr, &headers, &[])
            .await
            .is_err());
        assert!(limits.check_staff_write(Uuid::nil()).await.is_ok());
    }

    #[test]
    fn ignores_forwarded_client_key_by_default() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.7, 10.0.0.2".parse().unwrap());

        assert_eq!(
            client_key("198.51.100.2:1234".parse().unwrap(), &headers, &[]),
            "ip:198.51.100.2"
        );
    }

    #[test]
    fn extracts_forwarded_client_key_only_from_a_configured_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.7, 10.0.0.2, 10.0.0.3".parse().unwrap(),
        );
        let trusted = parse_trusted_proxy_cidrs(Some("10.0.0.0/8")).unwrap();

        assert_eq!(
            client_key("10.0.0.4:1234".parse().unwrap(), &headers, &trusted),
            "ip:203.0.113.7"
        );
    }

    #[test]
    fn ignores_loopback_forwarded_headers_without_an_explicit_cidr() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.7".parse().unwrap());

        assert_eq!(
            client_ip("127.0.0.1:1234".parse().unwrap(), &headers, &[]),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn falls_back_to_real_ip_when_a_forwarded_header_is_not_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.8".parse().unwrap());

        assert_eq!(
            client_key("198.51.100.2:1234".parse().unwrap(), &headers, &[]),
            "ip:198.51.100.2"
        );
    }

    #[test]
    fn parses_ip_and_cidr_trusted_proxies() {
        let trusted = parse_trusted_proxy_cidrs(Some("192.0.2.1, 2001:db8::/32")).unwrap();

        assert!(trusted[0].contains("192.0.2.1".parse().unwrap()));
        assert!(!trusted[0].contains("192.0.2.2".parse().unwrap()));
        assert!(trusted[1].contains("2001:db8:1::1".parse().unwrap()));
        assert!(!trusted[1].contains("2001:db9::1".parse().unwrap()));
    }

    #[test]
    fn rejects_invalid_trusted_proxy_cidrs() {
        assert!(parse_trusted_proxy_cidrs(Some("192.0.2.1/33")).is_err());
        assert!(parse_trusted_proxy_cidrs(Some("not-an-ip")).is_err());
        assert!(parse_trusted_proxy_cidrs(Some("192.0.2.1,,192.0.2.2")).is_err());
    }
}
