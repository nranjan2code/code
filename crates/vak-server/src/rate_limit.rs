use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RateLimitConfig {
    /// Peers authorized to assert a client IP through X-Real-IP.
    pub trusted_proxy_ips: Vec<IpAddr>,
    /// Max requests per window for `POST /gateway/inbound`.
    pub inbound_per_min: u32,
    /// Max requests per window for `POST /sessions`.
    pub sessions_per_min: u32,
    /// Max requests per window for `POST /sessions/{id}/run`.
    pub runs_per_min: u32,
    /// Max requests per window for all other POST endpoints.
    pub other_post_per_min: u32,
    /// Window duration in seconds.
    pub window_secs: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            trusted_proxy_ips: Vec::new(),
            inbound_per_min: 30,
            sessions_per_min: 5,
            runs_per_min: 10,
            other_post_per_min: 20,
            window_secs: 60,
        }
    }
}

impl RateLimitConfig {
    pub fn from_settings(s: Option<vak_config::RateLimitSettings>) -> Self {
        let d = Self::default();
        match s {
            None => d,
            Some(s) => Self {
                trusted_proxy_ips: s.trusted_proxy_ips,
                inbound_per_min: s.inbound_per_min.unwrap_or(d.inbound_per_min),
                sessions_per_min: s.sessions_per_min.unwrap_or(d.sessions_per_min),
                runs_per_min: s.runs_per_min.unwrap_or(d.runs_per_min),
                other_post_per_min: s.other_post_per_min.unwrap_or(d.other_post_per_min),
                window_secs: s.window_secs.unwrap_or(d.window_secs),
            },
        }
    }
}

#[derive(Debug, Clone)]
struct WindowBucket {
    count: u32,
    window_start: Instant,
}

#[derive(Debug, Clone, Default)]
struct IpWindow {
    buckets: HashMap<String, WindowBucket>,
}

impl IpWindow {
    fn check_and_increment(&mut self, key: &str, max: u32, window: Duration) -> Result<(), u64> {
        let now = Instant::now();
        let bucket = self
            .buckets
            .entry(key.to_string())
            .or_insert_with(|| WindowBucket {
                count: 0,
                window_start: now,
            });

        if now.duration_since(bucket.window_start) > window {
            bucket.count = 0;
            bucket.window_start = now;
        }

        if bucket.count >= max {
            let wait_secs = window
                .checked_sub(now.duration_since(bucket.window_start))
                .unwrap_or(Duration::ZERO)
                .as_secs()
                .max(1);
            return Err(wait_secs);
        }

        bucket.count += 1;
        Ok(())
    }

    fn cleanup(&mut self, window: Duration) {
        let now = Instant::now();
        self.buckets
            .retain(|_, b| now.duration_since(b.window_start) <= window);
    }
}

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<RwLock<HashMap<IpAddr, IpWindow>>>,
    config: RateLimitConfig,
    home: std::path::PathBuf,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig, home: std::path::PathBuf) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            config,
            home,
        }
    }

    async fn check(&self, ip: IpAddr, key: &str, max: u32) -> Result<(), u64> {
        let window = Duration::from_secs(self.config.window_secs);
        let mut map = self.inner.write().await;

        // Periodic cleanup: remove expired windows to bound memory
        if map.len() > 1000 {
            for w in map.values_mut() {
                w.cleanup(window);
            }
        }

        map.entry(ip)
            .or_default()
            .check_and_increment(key, max, window)
    }

    fn limit_for_path(&self, method: &str, path: &str) -> Option<(&str, u32)> {
        if method != "POST" {
            return None;
        }

        if path == "/gateway/inbound" {
            Some(("inbound", self.config.inbound_per_min))
        } else if path == "/sessions" {
            Some(("sessions", self.config.sessions_per_min))
        } else if path.starts_with("/sessions/") && path.ends_with("/run") {
            Some(("runs", self.config.runs_per_min))
        } else {
            Some(("other_post", self.config.other_post_per_min))
        }
    }
}

fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>, trusted_proxy_ips: &[IpAddr]) -> IpAddr {
    let peer = peer.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    if trusted_proxy_ips.contains(&peer)
        && let Some(ip) = headers
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<IpAddr>().ok())
    {
        return ip;
    }
    peer
}

#[derive(Serialize)]
struct RateLimitResponse {
    error: String,
    retry_after_secs: u64,
}

#[allow(clippy::result_large_err)]
pub async fn rate_limit_layer(
    State(limiter): State<RateLimiter>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response, Response> {
    // An arbitrary caller can set X-Forwarded-For/X-Real-IP and otherwise
    // evade login throttling by changing the header on every request.
    // A trusted-proxy identity policy may be added separately; the socket
    // peer is the only authenticated source address here.
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip());
    let ip = client_ip(req.headers(), peer, &limiter.config.trusted_proxy_ips);

    let method = req.method().as_str().to_owned();
    let path = req.uri().path().to_string();

    if let Some((key, max)) = limiter.limit_for_path(&method, &path)
        && limiter.check(ip, key, max).await.is_err()
    {
        let ip_str = ip.to_string();
        vak_core::security_events::record(
            &limiter.home,
            vak_core::security_events::EventKind::RateLimit,
            "rate_limit",
            &format!("path={path} limit={key}:{max}/min"),
            Some(&ip_str),
        );
        if let Some(hub) = crate::events::global() {
            hub.emit_security("RateLimit", &path);
        }
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "60")],
            Json(RateLimitResponse {
                error: "rate limit exceeded".into(),
                retry_after_secs: 60,
            }),
        )
            .into_response());
    }

    Ok(next.run(req).await)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn test_limiter(config: RateLimitConfig) -> RateLimiter {
        let dir = tempfile::tempdir().unwrap();
        RateLimiter::new(config, dir.keep())
    }

    #[test]
    fn proxy_client_ip_requires_an_explicit_trusted_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "198.51.100.7".parse().unwrap());
        let proxy: IpAddr = "127.0.0.1".parse().unwrap();
        let attacker: IpAddr = "203.0.113.9".parse().unwrap();
        assert_eq!(client_ip(&headers, Some(attacker), &[proxy]), attacker);
        assert_eq!(client_ip(&headers, Some(proxy), &[]), proxy);
        assert_eq!(
            client_ip(&headers, Some(proxy), &[proxy]),
            "198.51.100.7".parse::<IpAddr>().unwrap()
        );
    }

    #[tokio::test]
    async fn sliding_window_allows_within_limit() {
        let limiter = test_limiter(RateLimitConfig {
            inbound_per_min: 5,
            window_secs: 60,
            ..Default::default()
        });
        let ip = "127.0.0.1".parse().unwrap();
        for _ in 0..5 {
            limiter.check(ip, "inbound", 5).await.unwrap();
        }
    }

    #[tokio::test]
    async fn sliding_window_rejects_over_limit() {
        let limiter = test_limiter(RateLimitConfig {
            inbound_per_min: 3,
            window_secs: 60,
            ..Default::default()
        });
        let ip = "10.0.0.1".parse().unwrap();
        limiter.check(ip, "inbound", 3).await.unwrap();
        limiter.check(ip, "inbound", 3).await.unwrap();
        limiter.check(ip, "inbound", 3).await.unwrap();
        assert!(limiter.check(ip, "inbound", 3).await.is_err());
    }

    #[tokio::test]
    async fn different_ips_are_independent() {
        let limiter = test_limiter(RateLimitConfig {
            inbound_per_min: 1,
            window_secs: 60,
            ..Default::default()
        });
        let ip1: IpAddr = "10.0.0.1".parse().unwrap();
        let ip2: IpAddr = "10.0.0.2".parse().unwrap();
        limiter.check(ip1, "inbound", 1).await.unwrap();
        assert!(limiter.check(ip1, "inbound", 1).await.is_err());
        limiter.check(ip2, "inbound", 1).await.unwrap();
    }

    #[tokio::test]
    async fn different_keys_are_independent() {
        let limiter = test_limiter(RateLimitConfig {
            inbound_per_min: 1,
            sessions_per_min: 1,
            window_secs: 60,
            ..Default::default()
        });
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        limiter.check(ip, "inbound", 1).await.unwrap();
        assert!(limiter.check(ip, "inbound", 1).await.is_err());
        limiter.check(ip, "sessions", 1).await.unwrap();
    }
}
