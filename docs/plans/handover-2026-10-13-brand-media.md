# Handover 2026-10-13: brand media and companion motion and sound

Status: the client wiring was **verified live in a browser on 2026-10-13**
(second session; see "Verified" below). What is left is the owner's
listening and choosing. Read this first, then `docs/brand/media/README.md`.

## Verified (second session, 2026-10-13)

The live run found five faults, all fixed:

- **Saved UI preferences were lost on every reload.** `store.ts` and
  `soundCues.ts` imported each other, and the store read the sound-family
  list before it existed; the loader's `catch` turned that into the
  defaults. The list is now the import-free `src/soundFamilies.ts`.
- The start sound played once per step of a run; it plays once per run.
- `wood/listening.mp3` was a silent take and exported as an empty file. The
  export refuses silent takes; Wood plays its notification tap there for now.
- The server sent `.mp3` as `application/octet-stream`.
- The server cached every file under `assets/` for a year, including the
  sounds and loops, whose names never change. Those are now revalidated by
  `ETag`.

Checked: loop while working and stills elsewhere, Reduce motion both ways,
the family preview, start, finish, error and approval sounds, the seven
character sounds, at 1440 × 900 and 390 × 844 in light and dark
(`docs/brand/media/verification-2026-10-13/`). Not checked: the desktop
shell, Vakyartha's own character sound (it is not a choice in the new-agent
form), and the sound itself, by ear.

## Prompt for the next session

> Continue the Vakyartha brand-media work. Read
> `docs/plans/handover-2026-10-13-brand-media.md` and
> `docs/brand/media/README.md`. The client wiring is verified in a browser.
> With the owner: listen to the five sound families and the companion
> sounds (Pip and Tavi first), pick a family, hero image and theme take and
> fill the "Chosen" table; make a new take of `wood/listening` and remove
> its entry from `STAND_INS` in `tools/export_app_assets.py`; then check the
> loops and sounds once in the desktop shell.

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

1. Live verification: done in a browser; the desktop shell is not checked.
2. The owner has not listened to or looked at the media: pick a sound
   family, hero image and theme take, fill the "Chosen" table in the
   media README. Pip's and Tavi's tuned sounds are repitched down more than
   an octave; listen to those first.
3. The `cargo build` warning in the `vak` binary is the linker's
   (`__eh_frame section too large`), not from the code.
4. ElevenLabs credits left are unknown (the connector cannot read them);
   about 22,770 were spent. The subscription ends around 2026-10-16.

## Traps

- The connector's run-status result is too large to show and is saved to
  a file: pass 15 to 20 session ids per call so it is, then feed the file to
  `fetch_flow_results.py`. Signed links last two hours.
- Generated videos have a black background; `make_loops.py` removes only
  black connected to the frame edge.
- The sound-effects model ignores requested pitch; tune, never trust it.
