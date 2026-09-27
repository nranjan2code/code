# Vakyartha visual asset library

Status: **curated artwork and reference assets, 2026-09-27**. Open
[the gallery](index.html) to browse the cutouts, expression atlases, 3D
Songbird, and wallpapers on light, dark, and checkerboard grounds.

The optional [Dimensional visual pack](dimensional/README.md) has the eight
individual portraits, compact glyphs, app icon, app and site lockups, and a
runtime setting. Its source and export recipe are kept separately from the
approved flat platform artwork.

## What to use

| Need | Asset | Background | Guidance |
| --- | --- | --- | --- |
| Small product logo, app icon, print mark | [`../mark/vakyartha-songbird.svg`](../mark/vakyartha-songbird.svg) and generated [`../exports/`](../exports/) | Transparent or defined tile | Canonical geometry. Use these for identity and at small sizes. |
| Large 3D brand illustration | [`art/songbird-3d-transparent.png`](art/songbird-3d-transparent.png) | Transparent alpha | Editorial artwork. Its bevels/geometry differ slightly from the vector master. The Dimensional pack derives an optional app icon from it. |
| Agent portrait without a background | [`../characters/`](../characters/)`<id>.png` | Transparent alpha | Eight 512 × 512 source portraits. The app uses generated WebP size tiers, not these source PNGs. |
| Agent expression poses | [`../characters/`](../characters/)`<id>-atlas.png` | Transparent alpha | Eight 4 × 2 atlases. Use the runtime state mapping in `docs/design/71-agent-character-system.md`. |
| Light wallpaper | [`wallpapers/ensemble-day-master.png`](wallpapers/ensemble-day-master.png) | Scene | All eight characters in a warm reading room. |
| Dark wallpaper | [`wallpapers/ensemble-dusk-master.png`](wallpapers/ensemble-dusk-master.png) | Scene | All eight characters in a dusk reading room. |
| Additional 3D mood references | [`../explorations/2026-09-27-3d/`](../explorations/2026-09-27-3d/) | Scene | Concept images; do not treat as approved identity exports. |

The character portraits and atlases already appear throughout the web and
desktop Agent UI through `AgentMark`. The Dimensional option swaps in the
alternate idle portraits and small face glyphs, the in-app and site lockups,
browser favicon, and supported running desktop window icons. The installed operating-system icon remains a build-time
choice. The flat vector stays the production geometry reference.

## Wallpaper exports

Each scene has these ready-to-use JPEGs:

| Size | Aspect | Typical use |
| --- | --- | --- |
| 1920 × 1080 | 16:9 | Full HD monitor |
| 2560 × 1440 | 16:9 | QHD monitor |
| 3840 × 2160 | 16:9 | 4K monitor |
| 2560 × 1600 | 16:10 | Laptop |
| 3840 × 2400 | 16:10 | High-density laptop |

The generated masters are **1672 × 941** (day) and **1586 × 992** (dusk).
Larger JPEGs are deterministic Lanczos enlargements, not native 4K renders;
their dimensions fit the screen, but they contain no extra scene detail.
Keep the masters for later higher-resolution re-generation. Export with:

```sh
npm ci --prefix scripts/brand
npm --prefix scripts/brand run wallpapers
npm --prefix scripts/brand run wallpapers:check
```

The export script checks the reviewed master dimensions, center-crops to
16:9 or 16:10, writes JPEG quality 92 with full chroma resolution, and can
compare every saved export byte-for-byte. Re-review the crop if a new master
changes composition.

## Recreate or improve an image

1. Read [`prompts.md`](prompts.md). It records the **exact prompts for the
   three new generated masters**, their input references, and the built-in
   image generator used. The image model is nondeterministic: a prompt can
   reproduce the direction, not identical pixels.
2. Keep the eight existing transparent source portraits and atlases in
   `docs/brand/characters/`. Their original generation prompts are not in
   the repository; these files are the source of truth. Do not invent a
   provenance record for them.
3. For a new character expression, maintain the identity anchors in
   `docs/design/71-agent-character-system.md`, output a transparent 512 px
   portrait and 1024 × 512 4 × 2 atlas, and check alpha and 24 px readability.
4. After changing a character master or the flat SVG master, run
   `npm --prefix scripts/brand run generate` and
   `npm --prefix scripts/brand run check`. Review the web and desktop Agent
   surfaces and 16/24/32/64 px brand sizes before adopting the change.
5. Store new concepts in a dated `docs/brand/explorations/` directory with
   their prompt and reference images. Promote only selected source artwork
   into this library and list it here. Preserve the prior source; never
   silently overwrite an asset used by the product.

The character cutouts and the 3D Songbird PNG have genuine transparent
pixels. The wallpaper masters have opaque scenes. The 3D illustration is
an optional companion asset; the SVG remains the authoritative logo.
