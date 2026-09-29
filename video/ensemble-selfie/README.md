# Vakyartha ensemble selfie

The editable source for the 24-second, 1280 × 720, 30 fps character film. All eight characters work on small tasks inspired by their public-site examples, gather for a selfie, and resolve into the approved dusk wallpaper. The film is silent by design.

## Recreate

From the repository root:

```sh
npm --prefix video/ensemble-selfie install
npm --prefix video/ensemble-selfie run typecheck
npm --prefix video/ensemble-selfie run render
```

The render is `video/ensemble-selfie/out/ensemble-selfie.mp4`. The committed render is kept alongside the source for easy reuse. Chrome Headless Shell may download on the first render.

## Edit

- `src/EnsembleSelfie.tsx`: character order, work activities, text, animation, and selfie timing.
- `src/index.tsx`: duration, frame rate, and resolution.
- `public/ensemble/`: copies of the eight transparent portraits from `docs/brand/characters/` and the final background from `docs/brand/library/wallpapers/ensemble-dusk-1920x1080.jpg`. Keep the originals as the identity reference when changing art.

For a quick still: `npm --prefix video/ensemble-selfie run preview`. To inspect the timeline: `npm --prefix video/ensemble-selfie run dev`.

The film uses the Vakyartha name and existing character artwork, whose separate terms are in `docs/brand/README.md`.
