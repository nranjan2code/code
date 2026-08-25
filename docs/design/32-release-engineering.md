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
   (`~/.local/share/vakcoder/bin`) plus an `install.json` manifest
   {version, git_sha, installed_at, binaries}. The build tree may be
   cleaned at any time without touching a running deployment.
3. **Services are generated, never hand-edited.** `self services sync`
   renders launchd/systemd units from templates (vak-ops) referencing the
   *installed* path, unloads stale units, loads the new ones. Sync is
   idempotent: identical content + healthy process ⇒ no-op.
4. **Drift is detectable.** `self status` prints build vs manifest vs
   per-service versions and exits non-zero on mismatch; `/doctor`
   surfaces the same check so any surface reveals drift.
5. **Lifecycle is symmetric.** `self uninstall` reverses install exactly
   (stop → unload → remove units → remove binaries+manifest), preserving
   user data (`~/.vakcoder`) unless `--purge`.
6. **Updates are opt-in and atomic.** Passive `[update] url` check only
   notifies. `self update --url <manifest.json>` downloads to temp,
   verifies, renames over the installed binary, re-syncs services —
   never in-place writes, never auto-run.

## Command surface

```
vakcoder self install  [--prefix DIR]     copy release artifacts + manifest
vakcoder self services sync [NAME…]       regenerate + reload units
vakcoder self status                      drift matrix (exit ≠0 on drift)
vakcoder self uninstall [-y] [--purge]    exact reverse of install
vakcoder self update --url URL            opt-in pull-and-replace
```

## Release runbook (scripts/release.sh)

gate (fmt/clippy/test) → `cargo build --workspace --release` →
desktop bundle → `self install` → `self services sync` →
smoke (doctor parity, gateway port, bridge alive) → tag + GitHub release.
Humans and agents run the script; nobody replays steps from memory.

## Update safety & data continuity

Releases and updates must never lose data or credentials:

- **No secrets in units.** Templates embed zero credentials. The binary
  self-sources from `~/.vakcoder/.env` (gateway token, bot tokens,
  provider keys) — `secured_router_with` falls back to `.env` when the
  process env is empty. Regenerating units on a new machine therefore
  cannot strand auth.
- **Logs live at stable paths** (`~/.vakcoder/logs/<service>.log`),
  independent of install location; upgrades append, never truncate.
- **User data is out of scope for every lifecycle command.** Sessions,
  memory, inbox, tasks, checkpoints under `~/.vakcoder` are untouched by
  install/update/sync/uninstall; only `self uninstall --purge` may remove
  them, interactively confirmed.
- **Migration preserves state by identity.** Service names, ports,
  bindings, task store, and the telegram lock/cursor all key off
  `~/.vakcoder`, so moving the *binary* changes nothing else.
- **Atomic replacement.** Binaries are written temp-then-rename; units
  are diffed before rewrite; a failed sync leaves the previous healthy
  unit in place.

### Legacy migration (one-time, per machine)

1. Harvest any secrets embedded in existing hand-made units → merge into
   `~/.vakcoder/.env` (never clobber existing keys).
2. `self install` + `self services sync` — new units reference the
   installed path, source no secrets, log to the same stable paths.
3. `self status` proves: unit points at installed bin, service running,
   version parity across build/manifest/services.

## Migration of legacy deployments

Plists pointing at `target/release/*` are legacy. `self services sync`
rewrites them onto the installed path on first run; `self status` flags
any unit still exec'ing from a build tree until migrated.
