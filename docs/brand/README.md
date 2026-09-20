# vak brand assets

Source-of-truth marketing assets for vak. The runtime character package lives in
`crates/vak-client-ui/public/characters/` (see `docs/design/71-agent-character-system.md`); this
folder holds the marketing-ready derivatives and the brand facts that generations must respect.

## Mascot — the vak songbird

`mascot/vak-songbird-pose-01..08.png` — eight expression poses cropped from the canonical atlas
(`crates/vak-client-ui/public/characters/vak-atlas.png`), 443 × 443, transparent.

| Pose | Expression | Use |
|---|---|---|
| 01 | attentive, eyes on camera | default hero, end cards, beside the mark |
| 02 | eyes closed, serene | idle / resting |
| 03 | looking right | inspecting, pointing at content |
| 04 | looking up-right | thinking |
| 05 | side glance | sceptical beat |
| 06 | wide-eyed | surprise / delight |
| 07 | happy, eyes closed | satisfied nod |
| 08 | wink | social only — **never beside the logo** |

Anchors that must survive any pose: three-feather wave crest, indigo body, saffron throat, cream
face, calm expression, legible at 24 px.

## Mark

`mark/vak-mark-glyph.png` — the Devanagari **व** with the prompt chevron, amber bar and blue dash,
keyed out of the app icon tile. Use this on paper backgrounds; never place the white tile on paper.
The tiled icon remains `crates/vak-desktop/icons/icon.png`.

## Palette (light / paper — the marketing ground)

| Token | Hex | Role |
|---|---|---|
| paper | `#f4f1ea` | background |
| surface | `#faf8f3` | cards |
| ink | `#23211c` | text |
| terracotta (light) | `#a8462a` | the one accent on paper |
| sage action green | `#3f6b4f` | primary buttons in the experience screens |
| mark navy | `#101d3d` | the glyph |
| saffron | `#f5a400` | crest, bar, gate dot |

Dark tokens are in `DESIGN.md`; marketing defaults to paper.

## Typography

Editorial serif (Newsreader / Source Serif) for headlines; Inter / SF Pro for body and UI. The
experience screens in `docs/assets/vak-experience-2026/` are the reference for hierarchy.

## Voice

Brand voice on ElevenLabs: **Sana – Confident Indian Voice** (`tKZQTIqwDrPzLv6MrPxF`). Pronounce
*vak* as the Hindi वाक् — spell it `Vaak` in TTS prompts.

Sonic logo: `docs/video/spot-2026/audio/sfx-chirp.mp3` — a two-note songbird chirp. Use it on the
mascot landing in every video.

## Lines

- Campaign line: **Ask. Then go live your day.**
- Product tagline: *An agent you can inspect, constrain, and extend.*

## ElevenLabs

Brand kit `vak` (`BD3OM7OiZC3VWdzgKRiK`) carries the palette, rules, mark, mascot, the four
experience screens and the voice, so generations made on the canvas stay on brand.

## Rules

- One accent per composition; accents mean something.
- Flat tonal depth; shadows only on things that float.
- No dark sci-fi, neon, glow, particles, holograms, circuit boards.
- No fake benchmarks, testimonials or customer names.
- vak is universal — never frame it as developer-only.
- No URL in brand pieces until a domain is live.
