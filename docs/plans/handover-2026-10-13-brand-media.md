# Handover 2026-10-13: brand media and companion motion and sound

Status: the client wiring is built and committed but **not yet verified
live**. Read this first, then `docs/brand/media/README.md`.

## Prompt for the next session

> Continue the Vakyartha brand-media work. Read
> `docs/plans/handover-2026-10-13-brand-media.md` and
> `docs/brand/media/README.md` (Batch 4, "In the app", and the tuned
> companion table). First verify the client wiring live: build
> (`cd crates/vak-client-ui && npm run build`, then
> `cargo build -p vak -p vak-server --bins`), start the `vak-media-live`
> preview from `.claude/launch.json` (port 8933, workspace
> `/tmp/vak-live/media`, token in `/tmp/vak-live/test-gateway-token`), sign
> in with `vak open app --print --port 8933 -C /tmp/vak-live/media`, set
> Settings › Appearance › Visual pack to Dimensional 3D, run a turn and
> confirm the active companion's loop plays (and stills elsewhere), Reduce
> motion shows the still, Settings › Sound › Sound family plays a preview,
> turn start/finish/error/approval play their sounds (Network tab:
> `/app/assets/sounds/...` 200), and clicking a character in the agent
> picker plays its tuned sound. Check 1440×900 and 390×844, light and dark,
> with screenshots. Fix what fails, then run the full gate
> (`scripts/gate.sh`) and commit to main.

## What was done (all on main)

- **ElevenLabs media**, all in `docs/brand/media/` with `manifest.json`
  (every file, prompt, model, take) and `elevenlabs/generations.json`
  (118 generation records, links and workspace codes stripped): eight
  saved companion voices (Indian English), spoken lines and announcements
  (English ships; Hindi kept), five system-sound families, idle videos and
  transparent loops for all eight companions, theme music (use
  `theme-instrumental-*`), hero images.
- **Tools** in `docs/brand/media/tools/`: `fetch_flow_results.py`
  (downloads a flow's takes from a saved run-status result),
  `make_loops.py`, `export_app_assets.py` (the app's copies),
  `tune_companions.py` (companion signatures on their exact cue notes,
  verified within 9 cents), `export_records.py`. They need ffmpeg (Homebrew)
  and a venv with numpy, scipy, pillow (`pip install --only-binary=:all:`).
- **Client** (`crates/vak-client-ui`):
  - `src/soundCues.ts` is the one sound path: `playSound(moment)` for work
    start (`listening`), finish, error, approval and notification, and
    `playCompanionSound(id)` on a character click. Families in
    `public/assets/sounds/<family>/`, companions in
    `public/assets/sounds/companions/`. Quiet hours silence all but
    approval. Preference `soundFamily` (default `glass`), picker in
    Settings › Sound. The old synthesized tones and `characterSound.ts`
    are removed.
  - `AgentMark` plays `public/assets/brand/dimensional/loops/<id>-<64|128>.webp`
    in the Dimensional 3D pack while listening, thinking, working or
    waiting and visible; still otherwise, and under app or OS reduced
    motion. `characterLoop()` in `src/agentGlyph.ts`.

## Open

1. Live verification above (not done).
2. The owner has not listened to or looked at the media: pick a sound
   family, hero image and theme take, fill the "Chosen" table in the
   media README. Pip's and Tavi's tuned sounds are repitched down more than
   an octave; listen to those first.
3. The `cargo build` printed one warning in the `vak` binary; this work
   changed no Rust, so check whether it predates it before the gate.
4. ElevenLabs credits left are unknown (the connector cannot read them);
   about 22,770 were spent. The subscription ends around 2026-10-16.

## Traps

- The connector's run-status result is too large to show and is saved to
  a file: pass 15 to 20 session ids per call so it is, then feed the file to
  `fetch_flow_results.py`. Signed links last two hours.
- Generated videos have a black background; `make_loops.py` removes only
  black connected to the frame edge.
- The sound-effects model ignores requested pitch; tune, never trust it.
