# 32 — Release engineering
Status: implemented in 2.0.0

The practical runbook — what to run, and what a person on each platform
gets — is `docs/release-and-install.md`. This document owns the contract.

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
   (`/Applications/vak.app/Contents/MacOS/` on macOS,
   `~/.local/share/vak/bin/` on Linux) plus an `install.json` manifest
   {version, git_sha, installed_at, binaries}. The build tree may be
   cleaned at any time without touching a running deployment.
3. **Services are generated, never hand-edited.** `self services-sync`
   renders launchd/systemd units from templates (vak-ops) referencing the
   *installed* path and the canonical default workspace resolved by
   `vak_config::paths::default_workspace()`,
   unloads stale units, loads the new ones. Sync is idempotent: identical
   content + healthy process ⇒ no-op. The working directory is part of the
   service contract so gateway config and the project secret scope are not
   replaced by the service manager's `/` default. Generated units also carry the invoking
   user's non-secret `HOME`; without it, canonical path resolution can mistake
   the working directory for the user home and migrate `.vak` out of the
   project. The shared path resolver independently falls back to the operating
   system account home when GUI launch environments omit `HOME`, and fails
   closed to an absolute root rather than treating the current workspace as a
   home directory.
4. **Drift is detectable, and the mechanical fixes are one flag away.**
   `self status` prints build vs manifest vs per-service versions and
   exits non-zero on mismatch; `/doctor` surfaces the same check so any
   surface reveals drift. `vak doctor --repair` acts on the checks that
   have a known mechanical fix (self version parity → `self install
   --force`) and re-collects the report; checks with no mechanical fix
   (provider auth, config warnings) are left for the operator — doctor
   never guesses at those.
5. **Lifecycle is symmetric.** `self uninstall` reverses install exactly
   (stop → unload → remove units → remove binaries+manifest), preserving
   user data unless `--purge`.
6. **Updates are opt-in and atomic.** Passive `[update] url` check only
   notifies. `self update --url <manifest.json>` downloads to temp,
   verifies, renames over the installed binary, re-syncs services —
   never in-place writes, never auto-run.

14. **A release proves the artifact is the build it gated.** Every gate
    before this one verifies the *tree* — versions agree, bundles match
    their sources, tests pass — and none of them touches the binary. So
    `release.json` could assert a version the artifact had never been asked
    about, and did. `scripts/release.sh` now executes the collected `vak
    --version` and refuses to publish unless it reports both this version
    and this commit. Two consequences: `--no-build` cannot quietly publish
    whatever binaries were lying in the developer's `target/release` (it
    now requires an explicit `VAK_RELEASE_BIN_DIR` and still faces this
    gate), and a cross-published artifact that cannot be executed here is
    reported as unverified rather than silently trusted.
15. **Every build is `--locked`.** Release, CI, Docker, the linux and macOS
    verification stacks, and every smoke script pin to `Cargo.lock`. Only
    the Docker image did before, which meant the shipped binaries and the
    container could be built from different dependency graphs with nothing
    anywhere to say so. A build that would need to re-resolve fails instead.
16. **A gate that cannot see the tag it was asked to build is not a gate.**
    `release.yml` fires on `push: tags: v*` and `actions/checkout`
    materialises that tag, so the "this version has not shipped" check
    refused the exact tag it was invoked for and every automated release
    failed there. The check now refuses only a tag pointing at a
    *different* commit, which is the condition it was written for: this
    version already shipped from other code.
17. **A staleness guard may fail open locally and never at release.**
    `scripts/ui_bundle_check.rs` accepts a missing `.src-manifest` so a
    checkout predating the manifest still builds — correct for a local
    build, and a hole at release time, where deleting one file would
    silently disarm every bundle check at once. `release.sh` requires all
    three manifests to exist before it trusts any of them.
18. **A shipped artifact that is never built in CI is not shipped, it is
    hoped for.** The Linux container image is built, served, and probed on
    every CI run — `/health` for liveness and `/app` because a 200 there is
    what proves the committed bundles actually reached the binary — and
    asserted to carry a real commit stamp.

13. **2.0.0 is the supported baseline.** An install manifest, data home,
    or config written by an earlier version is refused whole by the one
    shared message in `vak_core::baseline`, which names the artifact, the
    version found, the baseline expected, and the single command that
    resolves it. Nothing is folded forward and nothing is migrated: the v1
    manifest fold-forward that used to live in `crates/vak/src/install/manifest.rs`
    is deleted, because half-adopting an old install produced a tree that
    `verify` and `update` disagreed about. The release feed carries no
    version below the baseline, so `self update` from an older line reports
    the baseline rather than resolving. See `AGENTS.md` invariant 29 and
    `docs/design/46-stabilization-install-and-onboarding.md` Part VII.

## Command surface

```
vak self install   [--prefix DIR] [--force]   place this build + manifest
vak self reinstall [--prefix DIR] [--yes]     clear the prefix, place fresh
vak self verify    [--prefix DIR]             components vs recorded digests
vak self status    [--prefix DIR]             drift matrix (exit != 0 on drift)
vak self services-sync [--prefix DIR] [NAME…] regenerate + reload units
                                      (using the invoking workspace)
vak self uninstall [--prefix DIR] [--yes] [--purge]
                                              exact reverse of install
vak self update    [--prefix DIR] [--url URL] [--yes] [--dry-run]
vak doctor         [--trust] [--repair]       health report; --repair acts on known fixes
```

`scripts/vak.sh <verb>` dispatches to the above (or to `scripts/build.sh` /
`scripts/release.sh` for `build`/`release`) so there's one entry point for
someone who doesn't already know which script or subcommand owns a given
lifecycle step — it never reimplements a verb, only routes to it.

`--prefix` is accepted by **every** subcommand and resolved in one place
(`install::layout::InstallRoot::resolve`): explicit flag, then
`VAK_PREFIX`, then the platform default. A prefix only `install`
understood produced installs that could not afterwards be inspected,
updated, or removed.

## Integrity and atomicity

7. **The manifest records a digest per component.** `install.json` is
   schema 2: `{schema, version, git_sha, installed_at, prefix,
   components[{name, path, sha256, required}]}`. `self verify` checks
   every component against its digest, so tampering and truncation are
   detected rather than assumed absent. Schema 1 manifests are migrated
   on read.

   A component may be a **tree**, not only a file: on macOS the desktop
   frontend is recorded as `desktop-frontend`, path `Contents/Resources`,
   digested over every file it contains including their relative paths, so
   a deleted, added or renamed asset all move the digest. Digesting only
   `index.html` would miss the failure that actually happens — the shell
   arrives and the JS it names does not, and the app opens a blank window
   with nothing in the console to say why.

   The manifest is **excluded from a tree it lives inside**. In a bundle
   `install.json` sits in `Contents/Resources`, and a manifest cannot carry
   a digest of a tree that contains the manifest: the value changes the
   moment it is written, and every fresh install verifies as corrupt.

7a. **Anything the manifest does not describe is not installed.** `verify`
   reads the manifest and nothing else, so a component absent from it is
   invisible to every check in the product. That is how a macOS bundle
   could ship with no frontend at all, report success, and verify clean:
   `write_metadata` skipped the copy silently when
   `crates/vak-client-ui/dist` was missing, and nothing described the copy
   afterwards. Installing the desktop app into a bundle now **requires**
   its frontend, checked before the transaction commits so a refusal
   leaves nothing behind, and records it as a component afterwards.
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
`release.json` feed that `self update` consumes. Gates and compilation share
one isolated Cargo target directory that is removed when the script exits;
release-only package identities never accumulate in the developer `target/`.
The script requires 15 GiB free by default (override with
`VAK_RELEASE_MIN_FREE_GB`) and builds only the three packages that own shipped
components.

`build.sh` keeps incremental artifacts for the current workspace version but
runs `cargo clean` when the shared workspace version changes. This is required
because every crate inherits that version and Cargo otherwise retains the old
hashed package identities indefinitely. `--no-desktop` excludes both the
frontend and Rust desktop package. Dev and test profiles retain line-table
debugging instead of full debug information, and tests disable incremental
state because one-shot test harness caches cost more disk than they save here.
`bump-version.sh` removes the now-invalid repository target generation and uses
Cargo metadata, rather than a workspace compilation, to refresh `Cargo.lock`.

## Canonical filesystem layout

`vak_config::paths` is the single source of truth for data, cache, and log
homes. Project-local `.vak/` and the CLI workspace-trust marker are
separate scoped contracts; pre-0.8 `~/.vak` references below describe
legacy migration only.

### macOS (default)

| Purpose | Path |
|---------|------|
| **Data home** | `~/Library/Application Support/vak` |
| **Cache** | `~/Library/Caches/vak` |
| **Logs** | `~/Library/Logs/vak` |
| **User secrets** | Shared secret scope (macOS Keychain, or the encrypted-file fallback under `<data_home>` when Keychain is unreachable) |
| **Config state** | `<data_home>/` (sessions, memory, tasks, inbox) |
| **Store DB** | `<cache>/store.db` (rebuildable from JSONL) |
| **Binary bundle** | `/Applications/vak.app/Contents/MacOS/` |

### Linux (default)

| Purpose | Path |
|---------|------|
| **Data home** | `~/.local/share/vak` |
| **Cache** | `~/.cache/vak` |
| **Logs** | `~/.local/state/vak/logs` |
| **User secrets** | Shared secret scope (Linux Secret Service, or the encrypted-file fallback under `<data_home>` on headless hosts with no D-Bus session) |
| **Installed binaries** | `~/.local/share/vak/bin/` |

### VAK_HOME override

Setting `VAK_HOME=/some/path` nests everything under that directory:
`/some/path/`, `/some/path/cache/`, `/some/path/logs/`. Used for
self-contained sandboxes (tests, portable installs).

## Update safety & data continuity

Releases and updates must never lose data or credentials:

- **No secrets in units.** Templates embed zero credentials. The binary
  self-sources from the canonical Shared secret scope named by
  `user_env_path()` — resolved through `vak_config::credentials`, never a
  plaintext file (docs/design/44-shared-config.md, "Secrets Chain") —
  covering gateway token, bot tokens, and provider keys;
  `secured_router_with` falls back to that scope when the process env is
  empty. Regenerating units on a new machine therefore cannot strand auth.
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

### Layout (canonical only)

There is no migration step. A fresh install writes state to the canonical
platform locations on first use: `data_home()` for sessions, config, and
    gateway state; `cache_home()` for the rebuildable store index; and
`logs_dir()` for service logs. `default_workspace()` (`~/vak-home`) is the
sole workspace root, carrying its `.vak/` shared layer and Shared secret
scope. With no prior user base to upgrade, the pre-0.8 `~/.vak` dotdir is
not migrated — operators who need retained data restore from a `vak backup`
export.

## Units that exec outside the managed prefix

A unit pointing at `target/release/*` is drift, not a supported deployment.
`self services-sync` rewrites it onto the installed path; `self status`
flags any unit still exec'ing from a build tree until it does.

## Frontend bundle staleness

`vak-admin-ui/dist` is committed and embedded into `vak-server` by
`include_dir!`; `vak-client-ui/dist` is gitignored and copied into the app
bundle by `self install`. Both can silently ship a frontend that no longer
matches its source — that is how v0.8.1 shipped a blank admin console.

Three layers now, innermost first:

1. `npm run build` writes `dist/.src-manifest`, one sha256 per source file
   (`crates/vak-admin-ui/scripts/stamp.mjs`, and the same file under
   `crates/vak-client-ui/scripts/`).
2. Each crate's `build.rs` re-verifies that manifest and fails the build with
   `cargo::error` naming the offending file — so a plain `cargo build` or
   `cargo test` after a UI edit stops, instead of quietly embedding the
   previous bundle. Shared via `include!("../../scripts/ui_bundle_check.rs")`.
   A missing manifest is accepted, so an older checkout still builds.
3. `scripts/build.sh` builds the admin frontend (it already built the desktop
   one); `scripts/release.sh` rebuilds both and additionally fails on a
   *committed* `dist/` diff, which a local build cannot see.

**The public site is the third such bundle** (docs/design/48-web-client.md
§4.6). `crates/vak-server/site/dist` is committed and embedded with
`include_dir!`, and has the identical trap minus npm: it is built by
`site/build.py`, which writes the same `.src-manifest` format, so
`vak-server`'s `build.rs` refuses to compile against a stale one. It runs
unconditionally in `scripts/build.sh` (no npm needed), `--check` in CI, and
rebuild-and-diff in `scripts/release.sh`. `.gitignore`'s blanket `dist/`
rule has to keep negating it, or the whole bundle silently stops being
committed.

**A release feed carries executables only.** So `self update` refuses to
touch a macOS bundle that would have its `vak-desktop` replaced: the
frontend under `Contents/Resources` is not in the feed, and updating the
binary alone leaves it driving the previous version's UI — a skew `verify`
cannot see, because the files it digests did not change. The remedy the
error names is the disk image.

## Release discipline

Rules earned from the 1.0.0 run, where four defects stacked so that no
release could be cut at all and none of them was individually visible.

**Never move a version line backwards.** `0.11.51 -> 0.2.0` cost, in order:
every 0.11.x install stranded with no update path; a minor-bump path that
could never be taken again, because `0.3.0`-`0.11.50` were already tagged; an
abandoned release that left a dirty tree and a phantom installed build; and a
monotonicity gate that failed closed and hid the rest. If a version line must
restart, restart it *forward*, past the whole retired range — which is what
`1.0.0` did.

**Neutralise optional inputs inside pipelines.** Under `set -o pipefail`, `ls
dist` on a checkout with no `dist/` fails the pipeline, then the assignment,
then the script — mid-report, so the ✓ lines already printed read as the
complete run. Any best-effort command in a pipeline needs `|| true`.

**Build the frontends before the gates.** `cargo clippy --all-targets` and
`cargo test --workspace` both compile `vak-desktop`, whose tauri codegen
hard-fails without the gitignored `crates/vak-client-ui/dist`. Gates first
meant a release only passed on a machine that had built the desktop UI at
some earlier point — the exact leftover-state dependency this document exists
to remove.

**Check every worktree before pushing, not just `origin`.** A clean
fast-forward against `origin/main` says nothing about a local `main` holding
unpushed commits, or a sibling worktree holding an uncommitted version bump.
Before a push or a release, read `git worktree list`, each worktree's dirty
count, and `git log` in *both* directions against origin.

**Never pipe a gate script through `tail` or a pager.** The pipeline reports
the pager's exit status, so a failed release prints success.

**A gate must not accept "Unreleased" when a tag is being cut.**
`check-version.sh` passes a CHANGELOG whose newest heading is `## Unreleased`
— right for day-to-day work, wrong at release time, because the release can
ship with its own entry unnamed. `release.sh` therefore refuses unless the
newest heading is the version being released, and it makes that check and
the dirty-tree check first, before its clean room deletes anything or the
test suite runs. `bump-version.sh` refuses a dirty tree, so a bump is always
its own commit.

**A failing service is not a stopped one.** `self status` and `doctor` read
each managed unit's last exit status (`last exit code` from `launchctl
print`, `ExecMainStatus` from systemd). A service that last exited non-zero
is shown as failing with that status and fails `doctor`; a service that was
stopped cleanly still reads as down. Before this, a crash-looping service
showed as `down ✓` and `doctor` passed.

**An untagged install says so.** Inside a clone that has the installed
commit, `self status` notes when the install is not the `v<version>` tag it
claims to be, as when an abandoned attempt left a 0.2.5 build installed that
no tag held.
