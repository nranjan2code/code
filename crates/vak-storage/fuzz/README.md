# vak-storage fuzzing

Targets for the frame reader (`frames`) and the sealed-segment reader
(`segments`). Neither may panic or allocate without bound on any input. This
crate is outside the workspace and is not part of `cargo test --workspace`.

The seed corpus is committed in `corpus/<target>/`, where `cargo fuzz`
looks for it. The stable gate, `tests/fuzz_corpus.rs` in `vak-storage`,
runs the same entry points over that corpus and 20,000 deterministic
mutations of it on every `cargo test`, and checks a write torn at every
byte. Regenerate the corpus from the readers' real output with
`VAK_UPDATE_CORPUS=1 cargo test -p vak-storage --test fuzz_corpus`.

    cargo install cargo-fuzz          # once; needs a nightly toolchain
    cd crates/vak-storage
    cargo +nightly fuzz run frames    # or: segments
