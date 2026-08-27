# 32 — Release engineering

One version, one install location, one lifecycle. No hand-synced stamps,
no plists pointing into build trees, no spot-fixing deploys.

## Invariants

1. **Version singularity.** `[workspace.package] version` is THE version.
   Every crate inherits (`version.workspace = true`). Desktop bundles
   derive it by *omitting* `version` from tauri.conf.json (Tauri falls
   back to the package version); frontend package.json carries no
   authoritative stamp. Any second place that needs a number derives it
   at build/run time from the binary (`--version`, `env!("CARGO_PKG_VERSION")`).
2. **Installed ≠ built.** Services never execute from `target/`.
   `self install` copies release artifacts into a managed prefix
   (`/Applications/vakcoder.app/Contents/MacOS/` on macOS,
   `~/.local/share/vakcoder/bin/` on Linux) plus an `install.json` manifest
   {version, git_sha, installed_at, binaries}. The build tree may be
   cleaned at any time without touching a running deployment.
3. **Services are generated, never hand-edited.** `self services sync`
   renders launchd/systemd units from templates (vak-ops) referencing the
   *installed* path and the workspace directory from which sync was run,
   unloads stale units, loads the new ones. Sync is idempotent: identical
   content + healthy process ⇒ no-op. The working directory is part of the
   service contract so gateway config and project `.env` are not replaced by
   the service manager's `/` default.
4. **Drift is detectable.** `self status` prints build vs manifest vs
   per-service versions and exits non-zero on mismatch; `/doctor`
   surfaces the same check so any surface reveals drift.
5. **Lifecycle is symmetric.** `self uninstall` reverses install exactly
   (stop → unload → remove units → remove binaries+manifest), preserving
   user data unless `--purge`.
6. **Updates are opt-in and atomic.** Passive `[update] url` check only
   notifies. `self update --url <manifest.json>` downloads to temp,
   verifies, renames over the installed binary, re-syncs services —
   never in-place writes, never auto-run.

## Command surface

```
vakcoder self install   [--prefix DIR] [--force]   place this build + manifest
vakcoder self reinstall [--prefix DIR] [-y]        clear the prefix, place fresh
vakcoder self verify    [--prefix DIR]             components vs recorded digests
vakcoder self status    [--prefix DIR]             drift matrix (exit != 0 on drift)
vakcoder self services-sync [--prefix DIR] [NAME…] regenerate + reload units
                                      (using the invoking workspace)
vakcoder self uninstall [--prefix DIR] [-y] [--purge]  exact reverse of install
vakcoder self update    [--prefix DIR] [--url URL] [-y] [--dry-run]
```

`--prefix` is accepted by **every** subcommand and resolved in one place
(`install::layout::InstallRoot::resolve`): explicit flag, then
`VAKCODER_PREFIX`, then the platform default. A prefix only `install`
understood produced installs that could not afterwards be inspected,
updated, or removed.

## Integrity and atomicity

7. **The manifest records a digest per component.** `install.json` is
   schema 2: `{schema, version, git_sha, installed_at, prefix,
   components[{name, path, sha256, required}]}`. `self verify` checks
   every component against its digest, so tampering and truncation are
   detected rather than assumed absent. Schema 1 manifests are migrated
   on read.
8. **Every mutation is a transaction.** Install and update stage all
   files first, verify the whole set, then move them into place, keeping
   what they displaced until the last move succeeds. Any failure restores
   the prior state — there is no half-installed prefix, and never a
   window where the CLI is new while the tray is old.
9. **Downloaded artifacts are verified before they are trusted.** The
   feed carries a SHA-256 per artifact; a mismatch aborts before anything
   is placed. A feed entry without a digest is refused outright.
10. **Version decisions are semantic, never lexical.** Comparing version
    strings puts `0.10.0` below `0.8.0`. Ordering goes through
    `semver::Version` — with build metadata stripped, because the crate's
    own `Ord` treats it as significant while semver 10 requires it be
    ignored for precedence.
11. **One writer owns the install root.** Build scripts build; placement
    is always `self install`. A script that copied a bundle into place
    separately produced an app the installer could not see, at a path
    that on a case-insensitive volume was the same directory.
12. **Blocking HTTP is confined to its own thread.** These subcommands
    dispatch from inside the CLI's tokio runtime, where constructing or
    dropping a `reqwest::blocking` client panics.

## Release runbook (scripts/)

```
scripts/bump-version.sh <semver>    THE version + lockfile refresh
scripts/check-version.sh            proves no second stamp exists
scripts/build.sh                    build, then `self install`, then `self verify`
scripts/release.sh --base-url URL   gate, build, checksum, emit release.json
```

`release.sh` gates on `check-version.sh`, `cargo fmt --check`, `cargo
clippy -D warnings`, `cargo test`, a clean working tree, and an unused
`v<version>` tag before it builds anything. It then writes
`dist/<version>/` containing the binaries, `SHA256SUMS`, and the
`release.json` feed that `self update` consumes.

## Canonical filesystem layout

`vak_config::paths` is the single source of truth. Every crate resolves
homes through it; no crate hardcodes `~/.vakcoder` or platform-specific
paths directly.

### macOS (default)

| Purpose | Path |
|---------|------|
| **Data home** | `~/Library/Application Support/vakcoder` |
| **Cache** | `~/Library/Caches/vakcoder` |
| **Logs** | `~/Library/Logs/vakcoder` |
| **User secrets** | `<data_home>/.env` |
| **Config state** | `<data_home>/` (sessions, memory, tasks, inbox) |
| **Store DB** | `<cache>/store.db` (rebuildable from JSONL) |
| **Binary bundle** | `/Applications/vakcoder.app/Contents/MacOS/` |

### Linux (default)

| Purpose | Path |
|---------|------|
| **Data home** | `~/.local/share/vakcoder` |
| **Cache** | `~/.cache/vakcoder` |
| **Logs** | `~/.local/state/vakcoder/logs` |
| **User secrets** | `<data_home>/.env` |
| **Installed binaries** | `~/.local/share/vakcoder/bin/` |

### VAKCODER_HOME override

Setting `VAKCODER_HOME=/some/path` nests everything under that directory:
`/some/path/`, `/some/path/cache/`, `/some/path/logs/`. Used for
self-contained sandboxes (tests, portable installs).

## Update safety & data continuity

Releases and updates must never lose data or credentials:

- **No secrets in units.** Templates embed zero credentials. The binary
  self-sources from the user `.env` at `data_home()/.env` (gateway token,
  bot tokens, provider keys) — `secured_router_with` falls back to `.env`
  when the process env is empty. Regenerating units on a new machine
  therefore cannot strand auth.
- **Logs live at stable paths** under `logs_dir()`, independent of
  install location; upgrades append, never truncate.
- **User data is out of scope for every lifecycle command.** Sessions,
  memory, inbox, tasks, checkpoints under the data home are untouched by
  install/update/sync/uninstall; only `self uninstall --purge` may remove
  them, interactively confirmed.
- **Atomic replacement.** Binaries are written temp-then-rename; units
  are diffed before rewrite; a failed sync leaves the previous healthy
  unit in place.
- **Stale-process detection.** `self status` compares binary mtime vs
  unit mtime; when the binary is newer and the process is still running
  on the old version, a kickstart (`launchctl kickstart -k` on macOS,
  `systemctl --user restart` on Linux) converges the process. Three
  tests in vak-ops enforce this invariant.

### Legacy migration (one-time, per machine)

The pre-0.8 layout stored everything under `~/.vakcoder/`. On first run,
`migrate_legacy_home()` performs a one-time rename:

1. `~/.vakcoder` → the platform data home (atomic rename under `$HOME`).
2. `store.db*` → relocated to the cache home (rebuildable, not user data).
3. `logs/` contents → relocated to `logs_dir()` so Console.app / journald
   keeps seeing them.
4. When `VAKCODER_HOME` is set, migration is skipped (the override is
   already self-contained).
5. If both old and new locations exist, migration refuses to guess a merge
   order and surfaces an error for manual resolution.

All entry points (vakcoder binary, vak-tray, vak-desktop) call migration
at startup before any other logic.

## Migration of legacy deployments

Plists pointing at `target/release/*` are legacy. `self services sync`
rewrites them onto the installed path on first run; `self status` flags
any unit still exec'ing from a build tree until migrated.
