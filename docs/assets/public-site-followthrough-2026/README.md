# Public site: follow-through and personality

Status: implemented, verified and published to https://vakyartha.com.
Date: 2026-09-27.

Preview: https://33fa1ur95-omv0ll961-sutraworks-lab.vercel.app
Deployment: `dpl_89NKWvvAQzQpBTAm4x3HrAj1dpQc` (Vercel project `www`).
The preview uses the project's existing Vercel sign-in protection.
The final production deployment is `dpl_FUsXeukWnLu3nkncbQ82KzynvHjr`,
including a mobile framing correction verified on the live site. All seven revised public pages returned HTTP 200
with their new content; the new illustration also returned HTTP 200.

## Changes

- Preserve the existing hero, white/charcoal themes, indigo actions, character
  scenes, expandable examples, typography and navigation.
- Add a continuity story, a plain explanation of commitments, and content about
  memory, approved reusable skills, useful results, decisions and spending.
- Keep the distinction between ongoing records and configured scheduled work;
  explain setup, scope of private history, and incomplete results.
- Show Vakyartha plus seven distinct personalities using the canonical names,
  portraits and descriptions from `crates/vak-client-ui/src/agentGlyph.ts`.
  Example responses also express each companion's voice.
- Update Home, Examples, How it works, Your control, Ways to use it, Our name,
  and Get started; regenerate the embedded bundle and static hosting export.
- Add a generated scene of Vakyartha, Mira and Moss continuing a document.
  Original, prompts and alpha-preserving WebP export are recorded in
  `docs/brand/library/public-site-scenes/continuity-prompts.md`.

## Verification

- All seven revised routes checked at 1440 × 900 and 390 × 844 in light/dark.
  No horizontal page overflow or observed broken image.
- Viewport captures saved here for each route/theme/size, plus the continuity,
  commitment and companion sections. Key sections visually inspected.
- New story disclosures open; the returning-work result explicitly says the
  file has not been sent.
- Commitment/recurrence disclosure opens and explains runtime prerequisites.
- Walkthrough tabs select panels; ArrowRight moves selection to the next tab.
- Both decisions in the review illustration produce the correct explanation
  and state that no real file changed.
- Site build and `--check` pass (44 generated files).
- `node --check crates/vak-server/site/src/site.js` passes.
- `cargo test -p vak-server --lib site::tests`: 5 passed.
- Local links, assets, fragment targets and unique IDs checked on all ten
  exported pages: no errors.
- `git diff --check` passes for the public site and content study.

No runtime behaviour, brand masters or existing character identities were
changed. Prior unrelated workspace changes, including the layout's existing
Documentation link, were preserved. The static review uses no real conversation,
provider call, file approval or scheduled execution.
