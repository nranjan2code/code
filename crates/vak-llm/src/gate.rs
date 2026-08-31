use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::error::LlmError;

#[derive(Clone)]
pub(crate) struct ProviderGate {
    base_url: String,
    api_key: String,
    slots: usize,
    semaphore: Arc<tokio::sync::Semaphore>,
}

pub(crate) struct ProviderPermit {
    _permit: tokio::sync::OwnedSemaphorePermit,
    _lease: ProviderLease,
}

impl ProviderGate {
    pub(crate) fn new(base_url: &str, api_key: &str) -> Self {
        let slots = provider_parallelism(base_url);
        Self {
            base_url: base_url.to_owned(),
            api_key: api_key.to_owned(),
            slots,
            semaphore: Arc::new(tokio::sync::Semaphore::new(slots)),
        }
    }

    pub(crate) async fn acquire(
        &self,
        cancel: &CancellationToken,
    ) -> Result<ProviderPermit, LlmError> {
        let permit = tokio::select! {
            _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
            permit = self.semaphore.clone().acquire_owned() => permit
                .map_err(|_| LlmError::Aborted { partial: None })?,
        };
        let lease =
            ProviderLease::acquire(&self.base_url, &self.api_key, self.slots, cancel).await?;
        Ok(ProviderPermit {
            _permit: permit,
            _lease: lease,
        })
    }
}

struct ProviderLease {
    path: PathBuf,
}

impl ProviderLease {
    async fn acquire(
        base_url: &str,
        api_key: &str,
        slots: usize,
        cancel: &CancellationToken,
    ) -> Result<Self, LlmError> {
        let root = std::env::temp_dir().join("vak-provider-gates");
        std::fs::create_dir_all(&root)
            .map_err(|e| LlmError::Network(format!("provider gate setup failed: {e}")))?;
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "lock") && stale_lease(&path) {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        let hash = gate_hash(base_url, api_key);
        loop {
            for slot in 0..slots.max(1) {
                let path = root.join(format!("gate-{hash:016x}-{slot}.lock"));
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                {
                    Ok(mut file) => {
                        let _ = writeln!(file, "{}", std::process::id());
                        return Ok(Self { path });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        if stale_lease(&path) {
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                    Err(error) => {
                        return Err(LlmError::Network(format!(
                            "provider gate acquire failed: {error}"
                        )));
                    }
                }
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
            }
        }
    }
}

impl Drop for ProviderLease {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn provider_parallelism(base_url: &str) -> usize {
    if let Ok(value) = std::env::var("VAK_PROVIDER_CONCURRENCY")
        && let Ok(value) = value.parse::<usize>()
    {
        return value.max(1);
    }
    let local = reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| {
            host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "[::1]"
        });
    if local { 1 } else { 8 }
}

fn gate_hash(base_url: &str, api_key: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in base_url
        .as_bytes()
        .iter()
        .chain([0].iter())
        .chain(api_key.as_bytes())
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub(crate) fn route_identity(name: &str, base_url: &str, api_key: &str) -> String {
    format!("{name}:{:016x}", gate_hash(base_url, api_key))
}

/// Stable, non-secret credential identity for frozen route contracts.
pub fn credential_id(base_url: &str, api_key: &str) -> String {
    format!("{:016x}", gate_hash(base_url, api_key))
}

#[cfg(unix)]
fn stale_lease(path: &std::path::Path) -> bool {
    let Ok(pid) = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok())
        .ok_or(())
    else {
        return true;
    };
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| !status.success())
        .unwrap_or(true)
}

#[cfg(not(unix))]
fn stale_lease(_path: &std::path::Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::{credential_id, gate_hash, provider_parallelism};

    #[test]
    fn local_endpoints_default_to_one_slot() {
        assert_eq!(provider_parallelism("http://localhost:11434/v1"), 1);
        assert_eq!(provider_parallelism("http://127.0.0.1:1234/v1"), 1);
        assert_eq!(provider_parallelism("http://[::1]:1234/v1"), 1);
    }

    #[test]
    fn remote_endpoints_default_to_eight_slots() {
        assert_eq!(provider_parallelism("https://api.example.test/v1"), 8);
        assert_eq!(provider_parallelism("https://localhost.example.test/v1"), 8);
        assert_eq!(provider_parallelism("https://127.0.0.10/v1"), 8);
    }

    #[test]
    fn one_provider_key_shares_capacity_across_models() {
        assert_eq!(
            gate_hash("https://api.example.test/v1", "key"),
            gate_hash("https://api.example.test/v1", "key")
        );
    }

    #[test]
    fn credentials_define_independent_pools() {
        assert_ne!(
            gate_hash("https://api.example.test/v1", "key-a"),
            gate_hash("https://api.example.test/v1", "key-b")
        );
    }

    #[test]
    fn credential_identity_is_stable_and_does_not_contain_secret() {
        let secret = "provider-secret-value";
        let identity = credential_id("https://api.example.test/v1", secret);
        assert_eq!(
            identity,
            credential_id("https://api.example.test/v1", secret)
        );
        assert!(!identity.contains(secret));
        assert_eq!(identity.len(), 16);
    }
}
