# Vakyartha video agent guide

Read this before creating or editing any production under `video/`. Also follow the repository root `AGENTS.md` and the brand rules in `docs/brand/README.md`. Public-facing work says **Vakyartha**; `vak` remains the internal code and CLI name.

## Productions

| Folder | Film | How to rebuild |
|---|---|---|
| `ensemble-selfie/` | Eight characters work, then come together for a selfie | `npm --prefix video/ensemble-selfie run render` |
| `vak-architecture/` | Narrated architecture explainer | `node scripts/video/render-vak-architecture.mjs` after generating audio and captions |
| `vak-story/` | Cinematic product story | `npm --prefix video/vak-story run render` |

Read each production's README and scripts before editing. `docs/video/AGENTS.md` contains the longer architecture video contract and shared verification guidance.

## Reproducibility contract

- Give each new production its own `video/<name>/` folder with a README, source scenes, local or documented source assets, generation scripts, pinned dependencies, and a single render command.
- Preserve the editable source **and** the requested final render. Never leave a source asset, music track, caption file, or render parameter known only to a one-off command or chat transcript.
- Describe the intended audience, scene order, duration, dimensions, frame rate, sound source, and final shot in the README. Include exact commands to regenerate and edit.
- Keep approved character identities and logo geometry. Use assets from `docs/brand/` and respect its separate artwork terms. Verify product claims against shipped code and the current design docs; read their `Status:` lines.
- Keep credentials, user data, and other sensitive content out of source assets and media. Do not change another production or unrelated work while editing one film.

## Edit and verify

1. Read the target README, inspect source/assets, and check `git status`.
2. Edit scene source and any generator that produces affected audio, captions, or assets. Update the README if the workflow or film changes.
3. Typecheck or build, render a representative still, and visually inspect it.
4. Render the full film. Inspect opening, middle, and closing frames; confirm audio exists and follows the scene timing.
5. Report the finished video's absolute path and the editable project path. State any unverified part.

For `ensemble-selfie/`, the scene is `src/EnsembleSelfie.tsx`, the composition settings are `src/index.tsx`, and `npm run render` renders the silent film. Keep its visual language aligned with the public site: neutral white and soft surfaces, indigo actions, saffron accents, system sans type, and original character artwork without emoji or chat-markdown styling.
