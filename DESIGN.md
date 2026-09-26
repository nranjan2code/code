---
name: vakyartha
description: A calm agent that listens, shows the result first, and explains the machinery when asked.
colors:
  paper: "#F7F7F8"
  surface: "#FFFFFF"
  sidebar: "#F2F2F4"
  line: "#DEDEE3"
  ink: "#202127"
  ink-2: "#41434D"
  muted: "#5C5E6A"
  ink-3: "#646674"
  primary: "#2F3C94"
  primary-wash: "#E8EAF6"
  link: "#2F3C94"
  saffron: "#F5A400"
  saffron-wash: "#FCEFD9"
  saffron-ink: "#8A5300"
  success: "#2E6B4A"
  danger: "#A33A3A"
  info: "#2F5580"
  mark-navy: "#101D3D"
  dark-paper: "#18191B"
  dark-surface: "#202124"
  dark-sidebar: "#1D1E21"
  dark-line: "#383A42"
  dark-ink: "#F0F0F2"
  dark-ink-2: "#D0D1D7"
  dark-muted: "#B5B7C0"
  dark-ink-3: "#A7A9B3"
  dark-primary: "#A3ADF7"
  dark-link: "#A3ADF7"
typography:
  hero:
    fontFamily: "system-ui"
    fontSize: "36px"
    fontWeight: 500
    lineHeight: "42px"
  greeting:
    fontFamily: "system-ui"
    fontSize: "28px"
    fontWeight: 500
    lineHeight: "34px"
  page-title:
    fontFamily: "system-ui"
    fontSize: "22px"
    fontWeight: 600
    lineHeight: "28px"
  section:
    fontFamily: "system-ui"
    fontSize: "18px"
    fontWeight: 600
    lineHeight: "26px"
  reading:
    fontFamily: "system-ui"
    fontSize: "16px"
    fontWeight: 400
    lineHeight: "26px"
  interface:
    fontFamily: "system-ui"
    fontSize: "15px"
    fontWeight: 400
    lineHeight: "22px"
  control:
    fontFamily: "system-ui"
    fontSize: "14px"
    fontWeight: 500
    lineHeight: "20px"
  meta:
    fontFamily: "system-ui"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: "18px"
  caption:
    fontFamily: "system-ui"
    fontSize: "12px"
    fontWeight: 500
    lineHeight: "16px"
  code:
    fontFamily: "ui-monospace, 'SF Mono', Menlo, Consolas, monospace"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: "22px"
rounded:
  control: "8px"
  card: "12px"
  sheet: "16px"
  pill: "999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "24px"
  xxl: "32px"
  xxxl: "48px"
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "#FFFFFF"
    rounded: "{rounded.control}"
    height: "36px"
    padding: "0 14px"
  button-secondary:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    border: "1px solid {colors.line}"
    rounded: "{rounded.control}"
    height: "36px"
    padding: "0 14px"
  chip:
    backgroundColor: "transparent"
    textColor: "{colors.ink-2}"
    border: "1px solid {colors.line}"
    rounded: "{rounded.pill}"
    padding: "4px 12px"
  card:
    backgroundColor: "{colors.surface}"
    border: "1px solid {colors.line}"
    rounded: "{rounded.card}"
    padding: "16px"
  message-box:
    backgroundColor: "{colors.surface}"
    border: "1px solid {colors.line}"
    rounded: "{rounded.sheet}"
    minHeight: "56px"
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    border: "1px solid {colors.line}"
    rounded: "{rounded.control}"
    padding: "8px 12px"
---

# Design System: Vakyartha

Status: **target system, adopted 2026-09-25** (docs/design/75-visual-refresh.md,
decisions D1 to D5). Stages V2.2 to V2.4 of `docs/plans/visual-refresh-plan.md`
move the stylesheet onto these tokens; until they land, the client still
carries the previous values. Build new work to this document, not to the
stylesheet.

## Overview

**North star: the Good Listener.** Vakyartha is an everyday agent for people
who are mostly not developers. It should feel calm and attentive: it listens,
answers in plain words, shows the result first, and explains the machinery
when someone asks. The interface conceals operational detail until that detail
helps a decision; it never removes a capability and never hides a safety
state.

The name comes from Sanskrit *vāk*, speech, and the mascot is a songbird. The
brand is **Ink and Saffron**: indigo ink for words and actions, and the mark's
saffron as the one accent, reserved for the moments Vakyartha is live.

**Key characteristics**

- Readable first: 16px conversation text, 14px controls, nothing under 12px.
- One ink colour for text and actions; one accent, and it means live.
- Light and dark designed together; neither is an inversion of the other.
- Flat at rest. The message box is the one resting element that floats.
- Plain words everywhere; technical detail one deliberate step away.

### One system, two densities

The **everyday** surfaces (the shared client in `crates/vak-client-ui`, for
the desktop and the web app) use the sizes in this document.

The **operator** surfaces (the admin console and the client's technical
views: Workbench, terminal, diffs, receipts) use the same colours, faces and
rules with a compact spacing step. They may show IDs, paths and monospace
readouts, because that is their job; the 12px minimum still applies.

## Colours

The complete palette is in the front matter. Every text pair clears WCAG AA
(4.5:1); the ratios are computed, never estimated (doc 75 §5.1 lists them).

- **Paper** (`#F6F5F1`, dark `#0F1120`): the window. Lighter and less yellow
  than the old cream; dark mode is indigo night, not brown-black.
- **Surface** (`#FFFFFF`, dark `#171A2B`): cards, sheets, the message box.
- **Ink** (`#1B1E36`, dark `#ECEBF5`), **ink-2** and **ink-3**: text, in three
  steps. Ink-3 is the quietest text allowed, including placeholders.
- **Primary** (`#2F3C94`, dark `#A3ADF7`): buttons, links and selection. Text
  on it is white in light and dark ink (`#0F1120`) in dark, where the
  primary lightens so one token serves as both fill and link.
- **Saffron** (`#F5A400`): the one accent. **Saffron-ink** (`#8A5300`) when
  text must carry it. The stylesheet names them `--live` and `--live-ink`.
- **Success** and **danger**: state only, always with an icon and a word.

### Named rules

**The Ink Rule.** Indigo carries text, buttons, links and the selected item.
Nothing decorative uses it.

**Saffron Means Live.** Saffron fills the microphone while listening, the dot
while an agent works, and the badge when Vakyartha waits for a person. It
appears nowhere else, so a glance says whether Vakyartha needs you. Saffron is
a fill; it is never text on a light ground.

**The No-Second-Palette Rule.** Every colour resolves to a token or a
`color-mix()` of tokens. Literal hex belongs only in a theme's token block
and in the theme previews in Settings, which must depict their own palette.

**Colour Is Never Alone.** Every state pairs its hue with a second channel:
a word, an icon, or a fill difference (hollow at rest, solid when running,
ringed when it needs a person), plus an accessible label. Roughly 8% of men
cannot tell a green dot from a red one.

**Tinting a ground invalidates the calibration.** Text tokens are measured
against untinted grounds. On a tinted ground (an approval card, a saffron
wash) step every text token up one level and re-measure.

### Themes

Four choices: **Match system** (the default, resolved in script to a concrete
`data-theme` so every reader of a token sees the same answer), **Light**,
**Dark** and **High contrast**. A stored theme id that no longer exists
resolves to Match system.

## Typography

- **Default: the system sans font**, including titles and greetings. The bundled
  Newsreader face remains an optional choice for people who prefer a serif.
- **Font choices:** interface, conversation/results, and code each have their
  own setting. Installed-font presets fall back to common platform fonts.
- **Text size:** 75–125% scales the shared role tokens across the client;
  text never drops below 12px. Code size remains independent.

| Role | Size / line | Face and weight |
|---|---|---|
| Hero | 36 / 42 | System 500 |
| Greeting | 28 / 34 | System 500 |
| Page title | 22 / 28 | System 600 |
| Section | 18 / 26 | System 600 |
| Reading | 16 / 26 | System 400 |
| Interface | 15 / 22 | System 400 |
| Control | 14 / 20 | System 500 |
| Meta | 13 / 18 | System 400 |
| Caption | 12 / 16 | System 500 |

### Named rules

**The Twelve-Pixel Floor.** No text is smaller than 12px at the default text
size, on any surface.

**Three Weights.** 400, 500 and 600. Nothing in between.

**Sentence Case.** Labels, buttons and headings use sentence case. No
capitalised micro-labels.

**Monospace Means Code.** File names, counts, versions and percentages in
everyday screens use the text face.

Answers wrap at about 68 characters. The text-size preference scales every
step.

## Layout

A three-region CSS grid: `sidebar` / `main` / `dock`, with a `banner` row
beneath. The banner row is `auto`, so it takes no space when nothing is shown.

**Anything placed directly in the app grid names its area.** A child with no
`grid-area` is auto-placed into the dock track: the setup banner once did this
and squeezed the whole workspace on first run.

**One breakpoint owns one layout.** Below 900px the sidebar and dock become
overlays; below 600px it is one column. The narrow layout exists to read,
answer an approval and steer a run, so an approval gets full-width controls
with nothing truncated.

Spacing follows a 4px grid (4, 8, 12, 16, 24, 32, 48). Sidebar rows are 40px,
buttons 36px, and the message box at least 56px. Conversation content is
centred at about 720px.

## Elevation and depth

Flat at rest; surfaces separate by tone and spacing, with a 1px line only
where spacing cannot do it. Shadow means floating:

- The message box floats over the conversation and carries one soft shadow.
- Sheets, menus, toasts and the connection pill leave the layout and carry
  the overlay shadow.
- Cards, results, buttons and sidebar rows never carry a shadow.

## Shapes

Radius 8 for controls and inputs, 12 for cards, 16 for the message box and
sheets, and full for avatars, pills and status dots.

## Motion

- 120ms for hover and press, 200ms for menus and panels, 320ms for sheets.
  One curve: `cubic-bezier(0.2, 0, 0, 1)`.
- New messages fade up 6px; a result reveals once; skeletons replace loading
  text.
- Nothing moves under the pointer.
- Reduced motion, from the system or the app setting, stills everything.

## Components

### Buttons
Primary: primary fill, white text. Secondary: surface fill, ink text, 1px
line. Text buttons use the link colour. Danger: a danger-tinted wash with
danger text, never a solid red fill. Each says what happens ("Review
changes", "Keep draft", "Apply 1 change").

### Chips
Pill-shaped, ink-2 text, 1px line; selected chips take the primary wash.

### Cards and results
Surface fill, 12px radius, 1px line, 16px padding, no shadow. In a
conversation only the result is a card; the answer's prose sits on the page.

### Inputs
Surface fill, 1px line, 8px radius; focus shows a 2px primary ring.
Placeholders use ink-3.

### Sheets
One sheet component for every dialog: centred, 16px radius, the title with
the close button beside it, a focus trap, Escape to close, and focus returned
to whatever opened it.

### Navigation
Sidebar rows are 40px, with the agent's character, its name and a one-line
status. The selected row takes the surface fill. Opening an agent shows a
spinner on its own row, never loading text.

### Status signals
A small dot: hollow at rest, solid while running, ringed when it needs a
person, and saffron only while live. The header names only what needs
attention ("Working", "Needs your decision", "Needs an AI service"); a ready
conversation shows no status.

### Connection pill
Hidden while connected. A first connection is silent for five seconds; a
drop shows a small pill above the message box after 1.5 seconds, with its own
word and dot fill for reconnecting, catching up and offline.

### Evidence meter
A four-segment track for the satisfaction lattice (asserted < cited <
observed < attested). Segments are solid up to the level achieved and hollow
past it; a 2px rule beneath marks the level required; a shortfall takes a
dashed saffron border, because a shortfall is a gap that needs a person. It
carries an `aria-label` naming both levels.

### Approval gate
A saffron-washed card that states the pending effect and its facts, with the
decision buttons full width on phones. On the tinted ground every text token
steps up one level (see the calibration rule).

## Words

Name things by what people recognise: your agents, folder, draft, changes,
connections, AI service. Show numbers and IDs only when they help a decision;
byte counts, hashes, run IDs and paths live under Technical details. Setup
and error messages say what happened and what to do next, one sentence each.
The glossary is doc 75 §7.

## Brand

- The public identity is the Vakyartha Songbird: the open V-shaped wing, the
  swept crest and the saffron throat, with the complete Vakyartha wordmark
  wherever the public name needs to be read. Its vector master is
  `docs/brand/mark/vakyartha-songbird.svg`; generated symbol, wordmark and
  platform exports follow `docs/brand/README.md`.
- The app icon uses the complete indigo tile with the bird centred inside it.
  Use standalone colour, reverse and single-ink bird shapes for surfaces that
  need those treatments. Never add a tile, shading, bevel or glow to the mark.
- Use the small bird symbol at 32px and below; larger character portraits are
  conversation companions, not logo substitutions.
- The 3D character portraits are for large moments at 64px or more; flat
  glyphs take over at 32px and below.
- The product line is the campaign line, "Ask. Then go live your day."

## Do's and don'ts

### Do
- Reserve saffron for live states and indigo for actions and selection.
- Keep every text pair above 4.5:1, measured.
- Put technical detail behind Details or Show technical details.
- Pair every state colour with a word or an icon.
- Replace an old rule when you change a component; never append an override.

### Don't
- Use text smaller than 12px, or a weight other than 400, 500 or 600.
- Add a shadow to a resting card, button or row.
- Use monospace for anything that is not code or file contents.
- Introduce a second accent, a gradient, or glass effects.
- Show an ID, path, byte count or hash on an everyday screen by default.
