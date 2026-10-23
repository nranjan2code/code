# Source links and answer width — 2026-10-23

Status: historical. Shared client correction and local browser acceptance; released/installed as 8.31.2 (see the release record).

The maintainer reported raw source URLs and horizontal overflow in installed
8.31.1. Source titles had been populated with full query URLs. The Sources
list and card shell used intrinsic grid tracks; wrapping with `break-word`
did not reduce their minimum content width.

- Keep meaningful source titles; otherwise use a non-URL publisher or the
  destination hostname as the link label. Preserve the complete href and
  existing safe-link checks. Do not repeat the hostname beside the same label.
- Let card/source grid tracks shrink with `minmax(0, 1fr)`; allow unbroken
  content to wrap with `overflow-wrap: anywhere`. Constrain the answer body
  to its available width. Do not hide overflow at the conversation boundary.
- Extend the production renderer fixture with the reported long weather URL
  as title and publisher, a named source, unbroken findings/titles and an
  unsafe destination. Check both page width and each source/card boundary.
- The real browser matrix passes 1440×900 and 390×844 in light and dark.
  Screenshot: `/tmp/vak-source-links-matrix.png`. The full destination remains
  exact; safe/unsafe links and meaningful titles retain their behavior.
- Both frontend bundles, the rich-response fixture typecheck and all 47 Node
  regression scripts pass. Build log: `/tmp/vak-source-links-build.log`.

This is a renderer/layout correction. It requires no prompt or authoring
schema change. Full native/form-state qualification remains tracked separately.

Release and installed-view evidence: [8.31.2](release-8.31.2-2026-10-23.md).
