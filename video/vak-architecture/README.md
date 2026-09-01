# VAK Architecture Video

This is the Remotion source project for the VAK architecture explainer. The composition is registered
as `VakArchitecture` at 1920×1080, 30 fps, and 90 seconds.

Run these commands from the repository root:

```sh
npm --prefix video/vak-architecture install
npm --prefix video/vak-architecture run dev
npm --prefix video/vak-architecture run typecheck
npm --prefix video/vak-architecture run render:still
npm --prefix video/vak-architecture run render
```

For the narrated build, use the repository scripts so voiceover, captions, and optional music are
discovered and passed into Remotion consistently:

```sh
node scripts/video/generate-voiceover.mjs
node scripts/video/generate-captions.mjs
node scripts/video/render-vak-architecture.mjs
```

The storyboard, narration, research, prompts, and agent reproduction instructions live in
[`docs/video/README.md`](../../docs/video/README.md).
