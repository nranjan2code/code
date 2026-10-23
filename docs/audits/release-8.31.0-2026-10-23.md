# 8.31.0 release and local installation — 2026-10-23

Status: historical. Native macOS aarch64 release and installed-component verification.

The maintainer requested a version bump, release and local installation.
Version 8.31.0 is retained locally per `docs/release-and-install.md`; no
public binary publication or push was performed. Release source is
`cbd4f7881`, tagged locally as `v8.31.0`.

- The workspace version and lockfile were bumped to 8.31.0; release notes
  carry the two Unreleased Topics/turn-name changes into this version.
- `python3 blueprint/harness/run.py deep` refreshed measures with no
  `needs review` lines; generated views were committed separately.
- `scripts/release.sh` passed: version, records, advisory, frontend, formatting,
  Clippy and full workspace test gates. The native optimized macOS/aarch64
  build completed and its provenance check reported
  `vak 8.31.0 (cbd4f7881)`.
- Artifacts and `SHA256SUMS` are retained in `dist/8.31.0/`. The update feed
  points at localhost for local testing.
- `dist/8.31.0/vak self install --force` installed
  `/Applications/Vakyartha.app`; `self verify` passed for the CLI, desktop,
  delivery worker and desktop frontend.
- `self services-sync` updated `com.vak.gateway` and `com.vak.desktop`.
  `self status` showed both running with the 8.31.0 manifest.
- A fresh turn in the installed desktop answered `What is 2 + 2? Reply with
  just the answer.` with `4`.

At status time the installed CLI reported the existing automation state as
Corrupt. This did not block installation, component verification, service
reconciliation or the fresh answer. No credentials or user data were changed.
