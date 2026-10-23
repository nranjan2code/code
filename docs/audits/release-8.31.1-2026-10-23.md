# 8.31.1 release and local installation — 2026-10-23

Status: historical. Native macOS aarch64 release and installed-component verification.

The maintainer authorized commit, push, version bump, release and local installation.
Implementation commit `b9ada8ca3` and release source `a9df70321` are pushed to
`origin/main`; `v8.31.1` names the release source and is pushed. Binary artifacts
are retained locally in `dist/8.31.1/`, not published to a public binary host.

- The clean `scripts/release.sh` passed version, record, advisory, frontend,
  formatting, Clippy and full workspace tests: 3,922 passed, zero failed,
  25 ignored. The optimized build passed provenance as
  `vak 8.31.1 (a9df70321)`. Log: `/tmp/vak-8.31.1-release.log`.
- `SHA256SUMS` verifies the CLI, desktop and delivery worker.
- `dist/8.31.1/vak self install --force` installed all three components and
  the desktop frontend in `/Applications/Vakyartha.app`; `self verify` passes.
- `self services-sync` updated gateway and desktop. `self status` reports
  both running; the gateway `/version` reports `8.31.1` and `a9df70321`.
- Existing conversation history renders in installed WebKit. A fresh request
  “What is 7 + 5? Reply with just the answer.” answered `12`.
  Screenshot: `/tmp/vak-8.31.1-installed-answer.png`.
- Status still reports the earlier automation `Corrupt` error dated
  2026-10-23T11:54:06Z, also recorded for the previous release. It did not
  prevent component/service verification or the fresh answer.

The maintainer subsequently reported long raw source URLs and horizontal
conversation overflow in the installed app. That follow-up is being fixed;
this release does not claim that layout acceptance passed.
