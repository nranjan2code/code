use std::sync::atomic::{AtomicU64, Ordering};

/// A capacity identity no other provider in this process has had or will
/// have. Capacity gates are process-wide and outlive the provider that made
/// them, so a fixed name or an address lets one test's provider inherit
/// another's cooldowns and held reservations. `Default` draws a fresh one,
/// so a mock that derives `Default` gets its own key too.
pub struct CapacityKey(pub String);

impl Default for CapacityKey {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(format!(
            "test-provider:{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}

/// A `POST /triggers` body for a script automation every `every_secs`.
#[allow(dead_code)]
pub fn script_trigger(name: &str, command: &str, every_secs: u64) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "kind": { "kind": "schedule", "schedule": {
            "kind": "interval", "every_secs": every_secs, "anchor": chrono::Utc::now(),
        }},
        "action": { "kind": "script", "command": command },
    })
}

/// A `POST /triggers` body for a prompt automation every `every_secs`.
#[allow(dead_code)]
pub fn prompt_trigger(name: &str, text: &str, every_secs: u64) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "kind": { "kind": "schedule", "schedule": {
            "kind": "interval", "every_secs": every_secs, "anchor": chrono::Utc::now(),
        }},
        "action": { "kind": "prompt", "text": text },
    })
}
