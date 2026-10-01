# vak-storage fuzzing

Targets for the frame reader (`frames`) and the sealed-segment reader
(`segments`). Neither may panic or allocate without bound on any input. This
crate is outside the workspace and is not part of `cargo test --workspace`.

    cargo install cargo-fuzz          # once; needs a nightly toolchain
    cd crates/vak-storage
    cargo +nightly fuzz run frames    # or: segments
