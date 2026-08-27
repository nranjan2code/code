# 32 — Release engineering

Release artifacts ship one Runtime authority and its client surfaces. The
release contract is intentionally greenfield: installation starts with an empty
data home, and no release code reads, imports, or translates another layout or
protocol.

## Version and provenance

`[workspace.package].version` in the root `Cargo.toml` is the only handwritten
version. Crates and Tauri derive their version from it. Release and install
scripts require a clean commit; `scripts/check-release-version.sh` rejects
dirty trees, drift in manifests, bundles, binaries, and changelog data, and a
version that is already tagged on another commit. Start a release with
`scripts/bump-version.sh patch|minor|major|X.Y.Z`, review the generated
changelog heading, commit it, run the release gate, and only then create the
matching `vX.Y.Z` tag.
Release binaries record the exact commit SHA.

## Build and install

Use `scripts/build-install.sh` for local installs and `scripts/release.sh` for a
candidate artifact. The managed program root is:

```text
<data_home>/runtime-bin/versions/<version>/
<data_home>/runtime-bin/current -> versions/<version>
```

The launcher at `~/.local/bin/vakcoder` points to `current`. Service units
execute that installed launcher, never `target/`, Cargo's bin directory, or an
app bundle. The TUI and desktop are clients and connect to the authenticated
Runtime; they do not embed a second server.

```bash
scripts/build-install.sh --gates
scripts/build-install.sh --with-tui --with-desktop
scripts/build-install.sh --no-service
```

`--no-service` is mandatory for disposable tests. Service installation is
explicit and owns the gateway process; the tray is an optional client addon.

## Canonical filesystem

Path resolution belongs to `vak-config` and is consumed by `vak-runtime`:

| Purpose | Location |
|---|---|
| Runtime state | `<data_home>/state.db` |
| Secrets | `<data_home>/.env` |
| Session ledgers | `<data_home>/sessions/<project>/<session>.jsonl` |
| Blobs | `<data_home>/blobs/<sha256>` |
| Operation audit | `<data_home>/audit/operations.jsonl` |
| Runtime receipt | `<data_home>/runtime/gateway.json` |
| Locks | `<data_home>/locks/runtime.lock` |
| Logs | `<data_home>/logs/` |
| Rebuildable cache | `<cache_home>/` |

On macOS, the default data home is `~/Library/Application Support/vakcoder`;
on Linux it is `~/.local/share/vakcoder`. `VAKCODER_HOME` is a complete,
self-contained test root, not an alternate compatibility layout.

## Lifecycle invariants

1. Every installed program has one version and one managed path.
2. Activation stages a version, checks `--version`, runs doctor/health checks,
   then atomically switches `current`.
3. Services are rendered from the installed path and are idempotent.
4. Install, update, and uninstall never rewrite Runtime state. Purging state is
   a separate, explicit operation.
5. Sessions and operation audit are append-only. SQLite projections and caches
   may be rebuilt from their authoritative records.
6. Secrets never appear in service units, command arguments, logs, receipts, or
   release archives.

## Release gate

Every candidate runs:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-release-version.sh
```

The artifact lane then installs into a disposable data home, starts one
gateway, registers a project, creates a session, runs and cancels a turn,
exercises TUI/desktop/admin clients, verifies backup/restore, and confirms
that no service points to a build tree. The same checks run with no provider
credential using deterministic Runtime tests; live provider checks are
separate and require a user-supplied key.
