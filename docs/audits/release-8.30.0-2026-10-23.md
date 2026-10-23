# 8.30.0 release and local installation — 2026-10-23

Status: historical. Native macOS aarch64 release and installed-component verification.

The maintainer requested a version bump, release and local installation. The
release is retained locally per `docs/release-and-install.md`; no public binary
publication was performed. Release source is `b9980856a`, tagged locally as
`v8.30.0`.

- `scripts/bump-version.sh 8.30.0` refreshed the workspace version and lockfile.
- Release notes carry the three Unreleased items into `8.30.0`.
- `python3 blueprint/harness/run.py deep` refreshed the measures with no
  `needs review` lines.
- `scripts/release.sh` passed: version, record, advisory, frontend, formatting,
  clippy and full workspace test gates; optimized native macOS/aarch64 binaries
  were built. Workspace tests passed; ignored tests were reported by Cargo.
- Artifact provenance reports `vak 8.30.0 (b9980856a)`. Artifacts and checksums
  are retained in `dist/8.30.0/`.
- `dist/8.30.0/vak self install --force` installed `/Applications/Vakyartha.app`.
  `self verify` verified the CLI, desktop, delivery worker and frontend.
- `self services-sync` updated `com.vak.gateway` and `com.vak.desktop`.
  `self status` reports both running on 8.30.0 with the matching manifest.
- The existing conversation `01a16652-e896-7663-b8ce-afdbb93acd13` completed a
  fresh direct-answer turn (`What is 3 + 4?` → `3 + 4 = 7.`).

The release feed points to localhost for local update-path exercise. No tag or
branch was pushed. At install time status also reported the existing model
catalogue network retry and missing Telegram token; those did not prevent the
release, install, verification or direct-answer turn.
