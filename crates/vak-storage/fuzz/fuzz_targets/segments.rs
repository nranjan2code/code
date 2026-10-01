#![no_main]
use libfuzzer_sys::fuzz_target;
use vak_storage::records::{GENESIS, verify_bytes};
use vak_storage::segments::decode_sealed;

fuzz_target!(|data: &[u8]| {
    if let Ok(frames) = decode_sealed(data) {
        let _ = verify_bytes(&frames, GENESIS);
    }
});
