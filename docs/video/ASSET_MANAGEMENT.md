# Video asset management

The video work has two layers:

1. `docs/tutor/` is the source library of architecture diagrams. These images remain reusable across
   documentation, presentations, and videos.
2. Each Remotion production declares the exact images it uses in a local manifest, then a sync script
   copies those images into that production's `public/assets/` directory before rendering.

This keeps the production self-contained for Remotion while preserving one canonical source for the
architecture visuals. Do not hand-copy files into a video project without adding them to its manifest.

## Current productions

- `video/vak-architecture/` — the original layer-by-layer explainer.
- `video/vak-story/` — the cinematic request-journey asset. Its manifest is
  `video/vak-story/src/assetManifest.ts`.

## Replacing an image

Replace or regenerate the matching file in `docs/tutor/`, keep the filename stable when the concept
is unchanged, and render again. If the concept changes, update the manifest `id`, source filename,
role, and alt text together. The render script will resync the new source.

## Adding an image

1. Add the source image to `docs/tutor/` and document it in `docs/tutor/README.md`.
2. Add an entry to the production's `assetManifest.ts`.
3. Reference the manifest id from scene data, not a raw filename.
4. Run `npm --prefix video/vak-story run typecheck` and a still render.

## Generated media

Generated voice, music, captions, copied assets, and rendered output are build products. They are
ignored by the production `.gitignore` unless deliberately promoted as a release asset. Keep prompts,
narration, manifests, and scripts committed so another agent can regenerate them.
