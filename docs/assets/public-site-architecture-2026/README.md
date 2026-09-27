# Public architecture page — 2026-09-27

Routes: `/architecture` and `/doctor`. Requested by the maintainer as a public, illustrated,
open-source explanation in the existing site style. Only Vakyartha's Songbird
appears on this page. The seven other companions remain on the everyday pages.

## Content and evidence

- Intent and commitment: `docs/design/47-commitment-kernel.md`; upkeep does not
  itself dispatch scheduled work. TaskDef remains the schedule model.
- Core and CorePool: `crates/vak-server/src/core_pool.rs`; workspace, permission
  override and policy identify runtime entries. Pooling is not a cluster claim.
- Evidence and evals: `docs/design/12-evals.md` and
  `docs/design/52-outcome-directed-runtime.md`.
- Voice: the **Shipped contract** in `docs/design/49-live-voice.md`. Utterance
  transcription, spoken replies and admitted channel notes are described;
  realtime streaming and physical-microphone acceptance remain open.
- FinOps: `crates/vak-core/src/finops.rs` and the admin UI. Expenditure is estimated
  where priced; unknown usage is not presented as free, or as a provider invoice.
- Administration: `docs/design/33-admin-console.md` and the current admin UI.
- Deployment: `docs/hosting.md`, `docs/design/53-distributed-bus.md` and current
  repository invariants. Self-hosting and planned cloud-remote work are distinct.
- Ledger and context: `docs/design/02-sessions.md` and
  `docs/design/68-context-engine.md`; append-only history, whole-turn selection,
  evidence recall, capacity profiles, summaries and cache structure.
- Doctor: `crates/vak/src/doctor.rs` and `crates/vak-core/src/health.rs`;
  check categories, supported repairs, fresh report, operator-owned decisions.
- Open source: repository MIT License; implementation and design links are public.

Nine generated transparent illustrations are retained with exact prompts in
`docs/brand/library/public-site-scenes/architecture/`. WebP site assets total
about 1.3 MiB. Labels and explanations remain accessible HTML.

## Review

Browser checks at 1440 × 900 and 390 × 844, light and dark:

- Brand, illustration transparency, readable labels and page width.
- All four illustrated scenes and the workspace, admin and deployment layouts.
- Main navigation and footer round trip from Home to Architecture.
- Chapter jump and All sections return, top and home links.
- Voice disclosure opens through its native control.
- No horizontal overflow at 390px; no failed image loads.
- Static validation: all 12 exported pages, local assets, links, fragments and
  unique IDs checked with zero errors.
- Public page keeps the existing `/version`-only data boundary; no new JavaScript.

Screenshots in this folder record the review. The existing site route tests
include the new explicit Rust route and its referenced assets.

## Navigation additions

Architecture and Wallpapers are in the primary navigation, with Wallpapers last.
Doctor is linked from the footer, setup page and architecture chapter. Every
GitHub link uses a new tab with `noopener noreferrer`. Chapter returns, home and
back-to-top links work without JavaScript.

## Published and verified

Production deployment `dpl_EPdvbLEQ3m7oZtBeePTd5iq1z4jR` is aliased to
https://vakyartha.com. Live Doctor, architecture ledger/context/voice chapters,
and home navigation returned HTTP 200 with the expected content.

Validation: workspace fmt, Clippy with warnings denied, full workspace tests,
version and documentation-path checks passed. After the final page additions,
all five focused public-site Rust tests passed. The export contains 12 pages
and 40 GitHub links with new-tab protection; static link/asset/anchor checks
reported zero errors. Screenshots cover desktop and phone widths in both themes.
