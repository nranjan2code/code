# Sandbox audit reproductions

These sources deliberately assert observed defects; passing is evidence of the problem, not validation of correct behavior. All filesystem data is disposable and the `.env` fixture contains only a fake marker.

`reproductions.rs` was temporarily registered as `crates/vak-tools/tests/__sandbox_audit_probe.rs` and run with `cargo test -p vak-tools --test __sandbox_audit_probe`. All four tests passed. To repeat, copy it to that unused test path, run the command, and remove only the temporary copy afterward. The control-file test is macOS-only; others exercise the raw tool as deterministic fixtures.

`broker-probe.rs` is a standalone executable linked against the current `vak_tools`, `tokio`, and `serde_json` rlibs from `target/debug/deps`, with that directory supplied as rustc's `-L dependency=...` and edition 2024. Build dependencies with `cargo test -p vak-tools --lib sandbox` first. Compile and run the probe under a fresh `.vak/scratch/` directory: it creates a `fixture` directory beside its executable. The same executable serves as the real broker worker when invoked with `__tool_worker`.

Observed output:

```text
Confirmed real broker: successful Bash result, zero sandbox events received.
Broker timeout: elapsed_ms=3027, result="command timed out after 1000ms"
```

The timing will vary; the probe asserts at least 2.5 seconds for a three-second sleep with a one-second timeout. It uses no provider and does not load Core or personal configuration. Remove its own disposable build directory after reproducing.

An additional Node check evaluated the Sandbox branch extracted from `store.ts` with signal stubs. Sequence: start A, finish A, start B, select A, deliver stdout. A received B's output; B remained empty. This confirms that selection and event destination currently share state.
