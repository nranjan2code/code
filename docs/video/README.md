# VAK architecture video

This folder contains the research, creative brief, narration, prompts, and agent workflow for the Remotion-based VAK architecture explainer.

## Source files

- [`AGENTS.md`](AGENTS.md) — reproduction, editing, enhancement, and verification instructions for coding agents
- [`RESEARCH.md`](RESEARCH.md) — Remotion and audio/caption research with source links
- [`SCRIPT.md`](SCRIPT.md) — scene-by-scene storyboard and timing plan
- [`STORY_FILM.md`](STORY_FILM.md) — director’s premise, visual language, beat map, and editing rules
- [`PROMPTS.md`](PROMPTS.md) — master visual prompt, scene prompts, and voice direction
- [`spot-2026/`](spot-2026/README.md) — the 25 s brand spot “Ask. Then go live your day.” (ElevenLabs canvas, keyframes, clips, audio, final MP4)
- [`ASSET_MANAGEMENT.md`](ASSET_MANAGEMENT.md) — canonical diagram library, manifests, syncing, and replacement rules
- [`narration.txt`](narration.txt) — canonical narration input for voice generation
- [`story-narration.txt`](story-narration.txt) — narration input for the cinematic second asset
- [`../../video/vak-architecture/`](../../video/vak-architecture/) — Remotion source project
- [`../../video/vak-story/`](../../video/vak-story/) — cinematic second video asset
- [`../../scripts/video/`](../../scripts/video/) — voice, caption, and render scripts

## Build flow

```text
narration.txt
  → TTS audio
  → speech-to-text word timestamps
  → captions JSON
  → Remotion composition + scenes + audio
  → MP4 render
```

The video is intentionally data-driven: scene content lives in `sceneData.ts`, while the renderer provides a reusable motion system. This makes it practical to update architecture terms without rebuilding the visual language from scratch.

## Quick start

```sh
npm --prefix video/vak-architecture install
npm --prefix video/vak-architecture run dev
```

Then render locally with:

```sh
npm --prefix video/vak-architecture run render
```

For narration and captions, run the scripts documented in [`AGENTS.md`](AGENTS.md).

## Design goals

- Technical clarity over spectacle
- Flat 2D diagrams instead of 3D or sci-fi metaphors
- Narration-led pacing with large labels and optional captions
- One visual system that can be extended for future VAK features
- Reproducible renders from committed source, prompts, and scripts

## Navigation

[Research](RESEARCH.md) · [Storyboard](SCRIPT.md) · [Prompts](PROMPTS.md) · [Agent instructions](AGENTS.md) · [Remotion project](../../video/vak-architecture/)

[Asset management](ASSET_MANAGEMENT.md) · [Story film](../../video/vak-story/)
