# 8.31.2 release and local installation — 2026-10-23

Status: historical. Source-link/width correction released and verified locally on macOS aarch64.

The source-link correction is commit `e922e9c07`; release source is
`049f8dbcb`, tagged and pushed as `v8.31.2`. Source changes and version bump
are pushed to `origin/main`. Artifacts remain local in `dist/8.31.2/`; no
public binary hosting was created.

- The clean `scripts/release.sh` passed version, record, advisory, frontend,
  formatting, Clippy and workspace tests: 3,922 passed, zero failed, 25 ignored.
  The optimized CLI passed provenance as `vak 8.31.2 (049f8dbcb)`.
  Log: `/tmp/vak-8.31.2-release.log`.
- All three artifact checksums pass. The canonical installer placed the CLI,
  desktop, delivery worker and desktop frontend in `/Applications/Vakyartha.app`;
  `self verify` passes. `~/.local/bin/vak` links to the installed CLI.
- `self services-sync` restarted gateway and desktop; `self status` reports
  both running and the gateway `/version` reports `8.31.2` and `049f8dbcb`.
- The existing weather answer now shows compact `geocoding-api.open-meteo.com`
  and `api.open-meteo.com` links in installed WebKit. The original complete
  destinations remain in the accessibility link values. The conversation has
  no horizontal scrollbar in the reported desktop view.
  Screenshot: `/tmp/vak-8.31.2-source-links-fixed.png`.
- A fresh installed-app request “What is 8 + 6? Reply with just the answer.”
  answered `14`, completing the post-install conversation smoke check.
- The production renderer regression passes independent 1440×900 and 390×844
  viewports in light/dark, with raw URL titles/publishers, meaningful titles,
  long unbroken content and unsafe destinations. Both client builds, fixture
  typecheck and all 47 Node scripts pass.

Full native channel/form-state/action qualification remains tracked under doc 104.
