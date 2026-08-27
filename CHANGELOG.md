# Changelog

## Unreleased

## 0.9.4 — install correctness and Runtime ownership

- Made a fresh install actually run. `self install` now mints the gateway
  bearer token into `<data_home>/.env`, and the Runtime opens without a
  provider credential instead of refusing to start — previously the managed
  gateway crash-looped on every clean machine, and the console that supplies
  the provider key could not start without the key.
- Generated service units pin `VAKCODER_HOME`, so a service manager resolves
  the same data home the installer used instead of a different one.
- Install verifies the authenticated handshake before reporting success.
- Implemented the Runtime singleton lock (`<data_home>/locks/runtime.lock`).
  A second gateway is refused, a lock left by a killed process is reclaimed,
  and `--gateway` now selects lock and receipt ownership instead of being
  parsed and discarded.
- SIGTERM runs graceful shutdown. Managed service stops now cancel in-flight
  runs and release the receipt and lock rather than leaving debris.
- Rewrote `vakcoder doctor` to diagnose an install without assuming any of it
  works, naming the one command that fixes each finding.
- Fixed updater version precedence: build metadata is ignored rather than
  rejected, and prereleases order against each other. `scripts/release.sh` now
  emits the `release.json` feed that `self update` consumes, which previously
  had no producer.
- Auth token comparison is constant-time.
- `self uninstall` reports separately-installed addons it cannot remove.
- Renamed the root `build-install.sh` to `scripts/build-desktop.sh`, fixed the
  README flags that never existed, repaired `scripts/server_smoke.py`, and
  removed the redundant `install_gateway_service.sh`.

## 0.9.3 — deployment and release hardening

- Routed OpenCode Zen credentials through the canonical provider configuration
  and server catalogue.
- Added clean-tree, provenance, and unique-tag checks to release and install
  gates, with a usable version bump workflow.
- Rebuilt and verified the managed Runtime, TUI, tray, and desktop installation
  from the committed release tree.

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
