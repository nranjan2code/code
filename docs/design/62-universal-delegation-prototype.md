# 62 — Universal delegation visual prototype

Status: **superseded — use docs/design/64-agent-owned-platform.md for the Agent-owned contract; retained as historical visual context.**

This is the reviewable visual reference for the universal Vak experience. It
describes the production components and their state mapping; fictional examples
are illustrative only and are never seeded as real activity.

## Visual direction

The shell uses the existing semantic theme tokens with a warm, composed accent
and generous reading space. Conversation remains the visual centre. Activity is
quiet and compact; typed results earn their layout. The small abstract Vak mark
(`.vak-companion`) is decorative, uses a declarative glyph, and breathes only
when motion is allowed. The quiet treatment is the existing Vak mark with the
companion animation disabled.

| Token | Production choice |
| --- | --- |
| Reading width | 680–800px conversation column |
| Body text | Existing theme body scale, approximately 16px |
| Motion | 120–220ms controls; 180–300ms panels; no continuous activity loop |
| Touch target | Existing settings/chip controls, minimum 24px target |
| Status | Text labels plus shape/glyph; never colour alone |
| Reduced motion | `prefers-reduced-motion` removes companion and helper animation |

## Six-state walkthrough

### First visit

The empty conversation presents the Vak mark, a single invitation, and broad
examples such as “Research a question” and “Plan something.” No sample activity
is shown as real. The composer is immediately usable and helper customisation is
optional.

### Ordinary conversation

The same column holds the user request, Vak’s concise answer, and any earned
typed presentation. Model, workspace, and permission controls stay behind the
existing More disclosure. The user can continue talking without opening a task
dashboard.

### Two concurrent tasks

Home shows an `Ongoing` collection from actual running sessions. The composer’s
“Start next request separately” action creates a new session and leaves the
current task running. The live subagent panel shows at most the real children,
with the work label first and an optional “with Pip” identity beneath it.

### Useful rich result

The existing typed presentation timeline owns result shape: document, comparison,
timeline, artifact, approval, or readable fallback. Home previews the canonical
timeline fallback rather than scraping raw transcript text. Opening a row
re-enters the originating conversation and preserves the result identity.

### Approval

Approval remains the existing deterministic gate. The concrete action, target,
scope, and expiry are shown by the existing approval renderer. Character marks
never replace the action or pressure the user; unrelated chat cannot approve it.

### Partial failure

Successful sibling output remains visible and the result status is `partial`.
The failure renderer explains the recovery path. The Vak companion stays neutral;
animation never substitutes for evidence or an error cause.

## Helper collection and editor

Settings exposes an optional “Your agents” collection. Each saved helper has a
stable ID and revision, a safe character preset, personality, working style,
“Useful for” guidance, movement, and voice preference. The editor provides live
glyph preview, example replies, voice preview on explicit click, Create, Edit,
Duplicate, Reset changes, Restore defaults, Cancel, Save, and Remove.

Saving is atomic and revisioned. Failed saves keep the draft open. Duplicate
names are visibly disambiguated in pickers and fail closed for conversational
lookup. Personality text can guide style only; it cannot grant tools, spending,
credentials, approvals, or cross-workspace context.

## Responsive and accessibility review

- At 390px, result rows become full-width and helper labels remain readable.
- At 768px, the conversation and optional detail surface retain comfortable
  spacing without adding a permanent dashboard.
- At 1440px, reading width stays bounded while Home collections remain compact.
- Keyboard focus remains visible through existing button/select styles.
- Live ongoing work uses an accessible polite region; errors use alert semantics.
- Voice preview never autoplays and is cancelled when the editor unmounts.
- Reduced-motion mode removes nonessential helper and companion animation.

## Production boundaries

This prototype is a visual contract, not a second runtime or result schema. The
server owns profile validation and persistence, the agent runtime owns profile
application, the scheduler owns recurring work, and the typed presentation
timeline owns result identity and fallback. Unknown helpers, invalid profiles,
missing previews, and stale revisions remain explicit states.
