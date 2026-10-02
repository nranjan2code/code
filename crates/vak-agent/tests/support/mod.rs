use std::sync::atomic::{AtomicU64, Ordering};

/// A capacity identity no other provider in this process has had or will
/// have. Capacity gates are process-wide and outlive the provider that made
/// them, so a key derived from an address hands a later provider at a freed
/// address the earlier one's cooldowns and held reservations.
pub fn capacity_key() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("test-provider:{}", NEXT.fetch_add(1, Ordering::Relaxed))
}
