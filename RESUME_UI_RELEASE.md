# UI release resume guide

Checkpoint: 2026-09-10, workspace version `3.0.43`.

This repository is intentionally paused at a reviewable checkpoint because the
Codex weekly secondary quota is nearly exhausted and the base-model weekly
bucket is exhausted. Do not spend the remaining quota on broad refactors.

## What is in this checkpoint

- Everyday and Advanced UI wiring, semantics, focus handling, retries, and
  error visibility were tightened.
- Everyday “My tasks” now uses persisted non-archived session history.
- Seed/update reconciliation is centralized and preserves user-edited seeded
  content while advancing untouched standard content.
- Client/Admin bundles and the acceptance script were rebuilt and checked.
- The evidence ledger is at
  `docs/reviews/ui-completion-checklist-2026-09-10.md`.

## Resume sequence

Run from the repository root, in order:

```sh
git status --short
git log -1 --oneline
sed -n '1,240p' RESUME_UI_RELEASE.md
scripts/check-version.sh
scripts/ui-acceptance-check.sh
cargo test --workspace
scripts/server_smoke.py
scripts/upgrade-gate.sh
scripts/macos-check.sh
scripts/linux-check.sh
```

Then inspect the evidence ledger and close only evidence-backed gaps. The
remaining planned verification is the full screen-reader/Tauri matrix,
authenticated desktop/browser lifecycle matrix, historical presentation-delta
replay, cross-surface fixture coverage, interaction latency, and the Linux
headless Docker check. Historical replay must use durable append-only
projection history; do not synthesize it from the live ring buffer.

## Release procedure after verification

1. Decide the release scope and changelog entry. For the next release, bump
   deliberately with `scripts/bump-version.sh <new-semver>`.
2. Run `scripts/check-version.sh`, all relevant tests, both platform checks,
   and the complete release script (`scripts/release.sh`) only after the
   version and evidence are consistent.
3. Review `git diff --check`, `git diff --stat`, generated artifacts, and the
   final clean tree. Never commit secrets or `.env` files.
4. Commit with a release-specific message and push the current branch without
   force-push. Record the commit, tag/artifact locations, and any deferred
   checks in the next handoff.

If any check fails, preserve the failure output, fix the smallest root cause,
rerun the affected checks, and update the evidence ledger before release.
