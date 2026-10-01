#![no_main]
use libfuzzer_sys::fuzz_target;
use vak_storage::records::{GENESIS, ScopeKey, entries_from_bytes, verify_bytes};

fuzz_target!(|data: &[u8]| {
    let _ = verify_bytes(data, GENESIS);
    let _ = entries_from_bytes(data, GENESIS, None);
    let _ = entries_from_bytes(data, [7; 32], Some(&ScopeKey([1; 32])));
});
