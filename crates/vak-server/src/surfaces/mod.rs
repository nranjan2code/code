//! Chat-surface transport adapters: long-polling or webhook clients that
//! bridge an external surface into `POST /gateway/inbound` and deliver the
//! reply back.
//!
//! Deliberately distinct from `vak-delivery`'s same-named modules, which own
//! the *markup projection* for each surface (Telegram HTML, Slack mrkdwn,
//! Discord markdown). Transport lives here; formatting lives there. Both sets
//! previously sat at `src/telegram.rs` in their respective crates, which read
//! as duplication until you opened both.

pub mod discord;
pub mod slack;
pub mod telegram;

fn prepared_chunks(body: serde_json::Value, surface: &str) -> Result<Vec<String>, String> {
    let packet: vak_delivery::DeliveryPacket = serde_json::from_value(body["delivery"].clone())
        .map_err(|_| "gateway did not supply a valid delivery packet".to_string())?;
    if packet.schema_version != vak_delivery::DELIVERY_SCHEMA_VERSION || packet.surface != surface {
        return Err("gateway supplied an incompatible delivery packet".into());
    }
    if packet.chunks.is_empty() || packet.chunks.iter().any(String::is_empty) {
        return Err("gateway supplied an empty delivery packet".into());
    }
    Ok(packet.chunks)
}

/// Watches a bridge's own credential and reports when it changes.
///
/// A bridge used to read its token once at startup, which made revoking
/// one ineffective until the process restarted — and that is why an API
/// handler ended up shelling out to `launchctl` to bounce a bridge. A
/// request handler orchestrating the platform service manager is the wrong
/// shape twice over: it blocks a request on a subprocess (AGENTS.md
/// invariant 26), and it makes revocation depend on an orchestration side
/// effect instead of on the data.
///
/// So the bridge watches its own credential. Revocation and rotation
/// become facts about `.env` that the bridge notices within one poll
/// cycle, on every platform, with nothing else involved — and no handler
/// touches the service manager at all.
pub struct CredentialWatch {
    env_var: String,
    seen: String,
}

/// What the credential looks like now, relative to what the bridge started
/// with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialState {
    /// Same value the bridge is already authenticating with.
    Unchanged,
    /// Rotated. The bridge should exit so the service manager restarts it
    /// with the new value — a lifecycle decision the bridge makes about
    /// itself, never one an API handler makes for it.
    Rotated,
    /// Gone. The bridge must stop using the old value immediately.
    Revoked,
}

impl CredentialWatch {
    pub fn new(env_var: impl Into<String>, current: impl Into<String>) -> Self {
        Self {
            env_var: env_var.into(),
            seen: current.into(),
        }
    }

    /// Re-read the credential from the canonical user `.env`.
    ///
    /// Reads the file rather than the process environment cache: the whole
    /// point is to see a change written by another process after this one
    /// started. A real environment variable still wins, matching the
    /// precedence every other secret lookup uses (invariant 8).
    pub fn check(&self) -> CredentialState {
        let current = std::env::var(&self.env_var).ok().or_else(|| {
            vak_config::user_env_path()
                .and_then(|path| vak_config::read_env_file_var(&path, &self.env_var))
        });
        match current {
            None => CredentialState::Revoked,
            Some(value) if value.trim().is_empty() => CredentialState::Revoked,
            Some(value) if value == self.seen => CredentialState::Unchanged,
            Some(_) => CredentialState::Rotated,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn an_unchanged_credential_reports_unchanged() {
        // Uses the process environment, which wins over the file.
        let watch = CredentialWatch::new("PATH", std::env::var("PATH").unwrap_or_default());
        assert_eq!(watch.check(), CredentialState::Unchanged);
    }

    #[test]
    fn a_credential_that_does_not_exist_reads_as_revoked() {
        let watch = CredentialWatch::new("VAK_TEST_CREDENTIAL_THAT_DOES_NOT_EXIST", "old");
        assert_eq!(watch.check(), CredentialState::Revoked);
    }

    #[test]
    fn a_changed_credential_reads_as_rotated() {
        let watch = CredentialWatch::new("PATH", "something-else-entirely");
        assert_eq!(watch.check(), CredentialState::Rotated);
    }
}
