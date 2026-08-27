//! Canonical connection discovery and readiness for local Runtime adapters.
//!
//! A local client never guesses a port or parses a second configuration
//! format. The gateway owns one atomically-written receipt; this module turns
//! that receipt into a typed client and validates the authenticated transport
//! handshake before a surface presents itself as connected.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Client, ClientError, Health, Version};

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("gateway receipt is unavailable at {0}")]
    ReceiptUnavailable(PathBuf),
    #[error("gateway receipt is malformed: {0}")]
    ReceiptMalformed(#[from] serde_json::Error),
    #[error("gateway receipt contains an invalid endpoint")]
    InvalidEndpoint,
    #[error("gateway receipt endpoint must be bound to loopback")]
    NonLoopbackEndpoint,
    #[error("gateway process {0} is not alive")]
    ProcessNotAlive(u32),
    #[error("gateway reported an incompatible protocol {0}")]
    IncompatibleProtocol(u32),
    #[error("gateway is not ready: {0}")]
    NotReady(String),
    #[error("gateway handshake failed: {0}")]
    Handshake(#[from] ClientError),
}

#[derive(Debug, Deserialize)]
struct GatewayReceipt {
    addr: String,
    pid: u32,
    token: String,
}

/// The one local gateway connection advertised by the Runtime process.
///
/// The token is deliberately redacted from debug output. Surfaces must retain
/// this object only for the lifetime of a request/stream and rediscover it
/// after transport failure instead of treating startup credentials as eternal.
#[derive(Clone)]
pub struct GatewayConnection {
    base_url: String,
    token: String,
    pid: u32,
}

impl std::fmt::Debug for GatewayConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GatewayConnection")
            .field("base_url", &self.base_url)
            .field("pid", &self.pid)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

/// A successful authenticated connection handshake.
#[derive(Clone, Debug)]
pub struct GatewayHandshake {
    pub health: Health,
    pub version: Version,
}

impl GatewayConnection {
    /// Resolve the canonical managed local Runtime receipt.
    pub fn discover_local() -> Result<Self, ConnectionError> {
        Self::from_receipt_path(
            vak_config::paths::data_home()
                .join("runtime")
                .join("gateway.json"),
        )
    }

    /// Resolve a receipt at an explicit path. This is primarily useful for
    /// isolated tests and never writes the path.
    pub fn from_receipt_path(path: impl AsRef<Path>) -> Result<Self, ConnectionError> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .map_err(|_| ConnectionError::ReceiptUnavailable(path.to_path_buf()))?;
        let receipt: GatewayReceipt = serde_json::from_str(&raw)?;
        if receipt.addr.trim().is_empty() || receipt.token.trim().is_empty() || receipt.pid == 0 {
            return Err(ConnectionError::InvalidEndpoint);
        }
        if !process_alive(receipt.pid) {
            return Err(ConnectionError::ProcessNotAlive(receipt.pid));
        }
        let base_url =
            if receipt.addr.starts_with("http://") || receipt.addr.starts_with("https://") {
                receipt.addr
            } else {
                format!("http://{}", receipt.addr)
            };
        let endpoint =
            reqwest::Url::parse(&base_url).map_err(|_| ConnectionError::InvalidEndpoint)?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || endpoint.path() != "/"
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(ConnectionError::InvalidEndpoint);
        }
        if !matches!(endpoint.host_str(), Some("127.0.0.1" | "localhost" | "::1")) {
            return Err(ConnectionError::NonLoopbackEndpoint);
        }
        Client::new(base_url.clone(), receipt.token.clone()).map_err(ConnectionError::Handshake)?;
        Ok(Self {
            base_url,
            token: receipt.token,
            pid: receipt.pid,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The authenticated transport credential for a native adapter. Browser
    /// adapters must use the server-issued HttpOnly session cookie instead.
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn process_id(&self) -> u32 {
        self.pid
    }

    pub fn client(&self) -> Result<Client, ConnectionError> {
        Client::new(self.base_url.clone(), self.token.clone()).map_err(ConnectionError::Handshake)
    }

    /// Validate the same authenticated version and health contract for every
    /// native surface before it calls itself connected.
    pub async fn handshake(&self) -> Result<GatewayHandshake, ConnectionError> {
        let client = self.client()?;
        let version = client.version().await?;
        let health = client.health().await?;
        if version.version.trim().is_empty() {
            return Err(ConnectionError::NotReady(
                "the gateway returned an empty version".to_owned(),
            ));
        }
        if health.protocol != 1 {
            return Err(ConnectionError::IncompatibleProtocol(health.protocol));
        }
        if health.status != "ok" {
            return Err(ConnectionError::NotReady(health.status));
        }
        Ok(GatewayHandshake { health, version })
    }

    pub async fn discover_local_ready() -> Result<Self, ConnectionError> {
        let connection = Self::discover_local()?;
        connection.handshake().await?;
        Ok(connection)
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    // The authenticated handshake below is authoritative on platforms without
    // POSIX process probing.
    true
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn receipt_normalizes_loopback_address_and_redacts_token() {
        let dir = tempfile::tempdir().expect("tempdir");
        let receipt = dir.path().join("gateway.json");
        std::fs::write(
            &receipt,
            format!(
                r#"{{"addr":"127.0.0.1:8901","pid":{},"token":"secret-value"}}"#,
                std::process::id()
            ),
        )
        .expect("receipt");
        let connection = GatewayConnection::from_receipt_path(&receipt).expect("connection");
        assert_eq!(connection.base_url(), "http://127.0.0.1:8901");
        assert!(format!("{connection:?}").contains("[REDACTED]"));
        assert!(!format!("{connection:?}").contains("secret-value"));
    }

    #[test]
    fn receipt_rejects_non_loopback_endpoint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let receipt = dir.path().join("gateway.json");
        std::fs::write(
            &receipt,
            format!(
                r#"{{"addr":"192.0.2.1:8901","pid":{},"token":"secret-value"}}"#,
                std::process::id()
            ),
        )
        .expect("receipt");
        assert!(matches!(
            GatewayConnection::from_receipt_path(&receipt),
            Err(ConnectionError::NonLoopbackEndpoint)
        ));
    }
}
