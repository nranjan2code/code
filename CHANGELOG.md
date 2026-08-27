# Changelog

## 0.9.2 — greenfield Runtime cut-over

- Consolidated the workspace on one canonical Runtime authority graph:
  `vak-runtime` behind `vak-server`, with `vak-client` adapters for CLI, TUI,
  desktop, and admin UI.
- Removed superseded parallel crates, state owners, and incompatible command
  surfaces. The shipped contract starts from an empty data home.
- Canonicalized configuration, session JSONL, SQLite state, blobs, audit,
  runtime receipt, locks, logs, and rebuildable cache paths.
- Enforced append-only session history, typed errors, capability epochs,
  cancellation propagation, brokered tools, scoped secrets, and discovered
  provider models across the Runtime boundary.
- Updated installation, service control, release smoke checks, UI clients,
  architecture notes, and `AGENTS.md` to the same ownership and data layout.
- Removed inactive channel/service controls from the tray and installer; the
  delivery crate remains a pure projection boundary for explicit adapters.

Verification for this release:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-release-version.sh
(cd crates/vak-admin-ui && npm run build)
(cd crates/vak-desktop/ui && npm run build)
```
