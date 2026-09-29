# Agent instructions for Vakyartha videos

This file is the starting point for any AI agent creating, editing, or reproducing a video in this repository. Public-facing videos use **Vakyartha**; `vak` remains the CLI and internal code name. Read the repository's root `AGENTS.md` and `docs/brand/README.md` before changing brand artwork or product claims.

## Find the right production

| Project | Purpose | Editable source | Rendered output |
|---|---|---|---|
| `video/ensemble-selfie/` | Eight-character work-to-selfie film | `src/EnsembleSelfie.tsx`, `public/ensemble/` | `out/ensemble-selfie.mp4` |
| `video/vak-architecture/` | Narrated architecture explainer | `src/`, `docs/video/narration.txt` | See its render scripts below |
| `video/vak-story/` | Cinematic product story | `src/`, `assetManifest.ts` | Its `out/` directory |

Each production must keep its own source, assets, generation scripts, dependencies, render command, and a README in its project folder. Keep the final file when the user asks to preserve or reuse a video. Never use a one-off shell command as the only source for music, captions, imagery, or scene timing. Record the tool versions and any externally sourced asset origin. Do not put credentials or private material in source, audio, logs, or rendered frames.

## Create or edit a production

1. Read the production's README and inspect its existing source and assets. Check `git status` before editing, and leave unrelated work alone.
2. Keep one production per `video/<name>/` folder. Put renderable scene code in `src/`, source assets in `public/` or a documented asset directory, generators in `scripts/`, and output in `out/`.
3. Use approved Vakyartha character and logo assets from `docs/brand/`. Preserve each character's identity. Verify factual product claims against shipped code and current design docs; check each design doc's `Status:` line.
4. Make the requested change, update any source generation scripts and README, then typecheck. Render a representative still before rendering the full video. Inspect an opening frame, a work or middle frame, and the closing frame. Check the audio when the production includes it.
5. Give the user the absolute path to the finished video and point to its source folder. State any render or verification limitation clearly.

For a new video, first write a short scene plan in its README: audience, duration, size, frame rate, character/action sequence, audio source, and final shot. Set up a reproducible `npm run render` (or equivalent) before considering the production complete. A video that exists only as an MP4 is not an editable source.

## Eight-character selfie film

The film follows the public website style: white and soft neutral surfaces, indigo, saffron accents, readable system sans type, and original character art. It is silent by design. Run from the repository root:

```sh
npm --prefix video/ensemble-selfie install
npm --prefix video/ensemble-selfie run typecheck
npm --prefix video/ensemble-selfie run preview
npm --prefix video/ensemble-selfie run render
```

The scene source controls the individual work moments and final gathering; `src/index.tsx` controls duration and resolution. The dusk wallpaper and eight individual portraits are copied from their canonical `docs/brand/` sources. See `video/ensemble-selfie/README.md` for details. Do not alter the legacy `vak-story` film when editing this separate production.

## Architecture explainer (existing production)

## Goal

Maintain the Remotion project in `video/vak-architecture/` as the reproducible source for the VAK architecture explainer. Keep the narration, scene data, visual language, and render commands in sync.

## Reproduce

```sh
npm --prefix video/vak-architecture install
npm --prefix video/vak-architecture run dev
```

For a narrated render:

```sh
node scripts/video/generate-voiceover.mjs
node scripts/video/generate-captions.mjs
node scripts/video/render-vak-architecture.mjs
```

The final command is the canonical narrated render. `npm --prefix video/vak-architecture run render`
is a visual-only validation render when no generated audio or caption assets are present.

The cinematic second asset is reproduced with:

```sh
npm --prefix video/vak-story install
npm --prefix video/vak-story run typecheck
npm --prefix video/vak-story run render
```

Its render command first runs `scripts/video/sync-story-assets.mjs`, which copies every image declared
by `video/vak-story/src/assetManifest.ts` from `docs/tutor/` into the production’s ignored asset cache.

The voice scripts require `OPENAI_API_KEY`. Optional environment variables are `VAK_TTS_MODEL`, `VAK_TTS_VOICE`, and `VAK_STT_MODEL`. Never commit keys or generated audio containing sensitive material.

For another production, set `VAK_VIDEO_ID` and `VAK_NARRATION_FILE`; the scripts then write into that
production’s own `public/audio` and `public/captions` folders.

## Edit safely

- Change scene wording and node labels in `video/vak-architecture/src/sceneData.ts`.
- Change animation and layout in `video/vak-architecture/src/VakArchitectureVideo.tsx`.
- Change narration in `docs/video/narration.txt`, then regenerate voice and captions.
- Keep the eight-scene, 90-second structure unless the user asks for a different duration.
- Preserve VAK invariants: authorize before dispatch, JSONL is the source of truth, provider/model are frozen per session, Managed Flow does not replace direct chat, and Operations Center reports evidence rather than synthetic values.
- For a visual change, make one targeted change at a time and render a short/still preview before a full render.

## Enhance

Prefer reusable components and data-driven scenes. Add a new scene to `sceneData.ts` before adding bespoke layout code. Keep typography large enough for 16:9 playback and ensure the narration remains understandable without captions.

Use the existing architecture posters in `docs/tutor/` as factual references, not as unverified generated copy. When implementation changes the architecture, update the source design document and this video’s narration/prompts together.

## Verify

```sh
npm --prefix video/vak-architecture run typecheck
npm --prefix video/vak-architecture run render:still
npm --prefix video/vak-architecture run render
```

Inspect the opening, agent-loop, Managed Flow, and closing scenes for clipping, timing, unreadable text, missing media, or incorrect crate names. Do not claim the video is current if the source architecture has changed without updating the script.
