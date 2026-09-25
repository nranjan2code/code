# Vakyartha brand assets

Source-of-truth marketing assets for Vakyartha. The public name is Vakyartha and
the official website is https://vakyartha.com. Internal commands, package names,
identifiers and data paths remain `vak`. The runtime character package lives in
`crates/vak-client-ui/public/characters/` (see `docs/design/71-agent-character-system.md`); this
folder holds the marketing-ready derivatives and the brand facts that generations must respect.

## Mascot — the Vakyartha songbird

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

`mark/vak-logo-master.png` is the canonical artwork: the exact 1254px RGBA image
supplied and approved by the maintainer on 2026-09-25 as `Va Logo.png`.
SHA-256: `d0db2c2956a3a19179417335e2ec65abc931210613a0391b89e78eb10b963de3`.
Preserve its navy-and-saffron woven V, paper tile, proportions and colour treatment.
Do not redraw it, add a bevel, shadow, glow or 3D effect, or change it by theme.
Production exports trim the exterior canvas and clip only the stray edge pixels;
they embed the original raster unchanged. The tile bounds are 73,73 to 1181,1181.
Display the complete tile in the client, admin, website, documents and social;
do not add another coloured tile, crop the mark, stretch it or add a shadow.

The maintainer requested this correction and the platform exports on 2026-09-25.
Platform adaptations are generated from the same geometry:

| Surface | Export contract |
|---|---|
| UI, favicon, notifications | Complete tile, transparent rounded corners, tight canvas; UI controls its display size. |
| macOS Dock / Finder | `icon.icns` and `app-icon.png`: 824px tile in a transparent 1024px canvas; native 1x/2x representations. |
| macOS menu bar | `tray-template.png`: 36px monochrome alpha mask for an 18pt status item. AppKit supplies light/dark/selection colour. This is the sole monochrome, tile-free exception. |
| Other desktop trays | `tray-color.png`: complete colour tile at 32px. |
| Browser installation | Actual 192px and 512px icons, plus an opaque 512px maskable export with the mark inside its safe circle. |
| iOS / Apple touch | Opaque square paper ground; the OS supplies the corner mask. |
| Android adaptive | Transparent foreground with safe padding; matching paper background. Legacy and round exports are generated too. |

Run on macOS (Node 20.9+; `iconutil` is supplied by macOS):

```sh
npm ci --prefix scripts/brand
npm --prefix scripts/brand run generate
npm --prefix scripts/brand run check
```

`sharp` is pinned to 0.35.4 in the isolated asset-tool manifest and lockfile. It
renders the approved source deterministically; it adds no application dependency.
Do not hand-edit generated exports. Rebuild the client, admin and public-site
bundles after changing assets or their framing. Baseline screenshots and the
labelled before image in the visual-refresh review remain historical evidence.

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

Brand voice on ElevenLabs: **Sana – Confident Indian Voice** (`tKZQTIqwDrPzLv6MrPxF`). Use **Vakyartha** in new public narration; internal `vak` commands retain their name.

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
- Vakyartha is universal — never frame it as developer-only.
- The public website is https://vakyartha.com; do not substitute similarly named domains.
