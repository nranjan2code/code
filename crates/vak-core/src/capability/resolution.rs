//! Whether a capability is usable right now.
//!
//! Every capability is knowable offline: it is declared, or it has been
//! revoked. Nothing here talks to anything. An MCP server's connection state
//! — its catalog, its last failure, its retry backoff — belongs to the
//! on-demand pool (`vak_mcp::McpManager`), the one place a server is ever
//! started, and reaches the registry as declared data. The registry once
//! probed servers itself on a background timer, which meant spawning them
//! with nobody asking and re-spawning them after the pool had evicted them
//! for idleness.

use serde::{Deserialize, Serialize};

/// The lifecycle of one capability's usability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Resolution {
    /// Declared and usable.
    Available,
    /// Revoked. Kept as a tombstone while a live epoch still references it,
    /// so a session mid-turn can still resolve what it was told about.
    Retired { reason: String },
}

impl Resolution {
    /// Whether a turn may use this capability.
    pub fn is_usable(&self) -> bool {
        matches!(self, Resolution::Available)
    }

    /// One-line status for the operator report and the model's standing
    /// section. Both read this, so they cannot disagree.
    pub fn summary(&self) -> String {
        match self {
            Resolution::Available => "ready".into(),
            Resolution::Retired { reason } => format!("removed: {reason}"),
        }
    }
}
