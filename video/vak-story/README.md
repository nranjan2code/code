# VAK Story Film

This is the second VAK video asset: a 90-second cinematic explainer organized around one request
moving through the system. It uses the architecture posters in `docs/tutor/` as evidence-led visual
cutaways rather than presenting them as a static slide deck. Every tutor poster is declared in the
story manifest and used as a directed visual beat.

From the repository root:

```sh
npm --prefix video/vak-story install
npm --prefix video/vak-story run typecheck
npm --prefix video/vak-story run dev
npm --prefix video/vak-story run render:still
npm --prefix video/vak-story run render
```

The render command synchronizes the declared poster assets before rendering. Asset ownership and
replacement rules are documented in [`docs/video/ASSET_MANAGEMENT.md`](../../docs/video/ASSET_MANAGEMENT.md).

To generate this film’s own voice track and then render it:

```sh
VAK_VIDEO_ID=vak-story VAK_NARRATION_FILE=docs/video/story-narration.txt node scripts/video/generate-voiceover.mjs
VAK_VIDEO_ID=vak-story node scripts/video/generate-captions.mjs
npm --prefix video/vak-story run render
```
