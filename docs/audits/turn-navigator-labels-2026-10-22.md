# Turn navigator answer titles — 2026-10-22

Status: historical. Shared-client regression check; browser-qualified before release
and not yet visually qualified in installed native WebKit.

Released and locally installed as 8.29.0 on 2026-10-23; see
[release verification](release-8.29.0-2026-10-23.md). The checks below describe
the pre-release acceptance; native visual qualification remains open.

The maintainer reported a hover showing a long raw request. The navigator read
only accepted plan names, then the request. The card redesign removed the plan
tool, leaving every new turn on that raw-request fallback. This was a client
contract dependency missed by the redesign, not a model failure.

The shared resolver now reads the same turn's recorded answer heading, then
its authored card titles. Derived runtime receipt
cards cannot supply the work title. Untitled navigator previews show “Turn N”.
The sidebar reuses recorded answer titles, retaining its request fallback for
untitled work. No inference from request words, title-writing tool, extra model
request or text-length cutoff is introduced.

Checks:

- `node crates/vak-client-ui/tests/turn-pairing.mjs`: canonical directive
  identity, current headings with nested inline text, authored
  card titles, different-turn exclusion, derived receipt exclusion and no
  raw-request navigator label; latest sidebar title follows the same resolver.
- Both frontend hosts build with TypeScript checks.
- The existing navigator browser fixture now contains current Markdown answer
  headings and no plan outline. Hover shows “Reading club result 50/51” rather
  than “Request 50/51”. Home reaches turn 1 with its answer title.
- Browser checks at 1440 × 900 and 390 × 844, light and dark; narrow previews
  stay within the viewport without horizontal overflow. Screenshots:
  [desktop light](../assets/turn-navigator-labels-2026-10-22/desktop-light.png),
  [desktop dark](../assets/turn-navigator-labels-2026-10-22/desktop-dark.png),
  [mobile light](../assets/turn-navigator-labels-2026-10-22/mobile-light.png),
  [mobile dark](../assets/turn-navigator-labels-2026-10-22/mobile-dark.png).

An answer with no authored heading or card title has no semantic title to read;
its turn number is an explicit fallback. Native installed acceptance remains
open. This change does not reinstate plan-authored layout or plan checks.

The maintainer then rejected retention of obsolete plan compatibility. The
parser, answer outlines on timelines/pages, plan-only reply filters and
correction tags are deleted. Stored ledgers are not rewritten. Generic card
builders remain because inline fences and runtime cards still use them.

Closeout: navigator and old-plan cleanup committed as `bc8d1f92b`, pushed to
main. The full gate passed before that commit, and the subsequent combined
progress/receipt-isolation full gate passed on 2026-10-23. The installed app
remains the earlier 8.28.0 release; these follow-ups are unreleased.
