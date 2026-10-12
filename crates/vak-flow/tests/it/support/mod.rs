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
