# Agent instructions: reproduce and evolve the VAK architecture video

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
