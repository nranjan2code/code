# vak spot — "Ask. Then go live your day." (25 s)

A music-driven brand spot for the public site, YouTube pre-roll and social. Universal, peppy, no
narration. Built on the ElevenLabs creative canvas from the approved experience screens in
`docs/assets/vak-experience-2026/` and the songbird mascot.

- Final: [`vak-spot-25s.mp4`](vak-spot-25s.mp4) — 1920 × 1080, 24 fps, stereo AAC, −15.7 LUFS.
- Re-cut (2026-10-15): [`vak-spot-25s-v2.mp4`](vak-spot-25s-v2.mp4) — the same cut with the result card
  really landing at 10–12.5 s (Clip B1, below) instead of a hold. Same length, loudness and sound
  cues; choose between the two by watching both.
- Poster / thumbnail: [`keyframes/06-end-card.png`](keyframes/06-end-card.png).
- Canvas (all sources, re-rollable): <https://elevenlabs.io/app/flows/7ivRdDilWewnPh93H8Nc>
- Brand kit on ElevenLabs: `vak` (`BD3OM7OiZC3VWdzgKRiK`) — see `docs/brand/README.md`.

## Idea

Everyone hesitates to hand real work to an AI because they cannot see what it will do or take it
back. vak removes that: it works on a copy, shows the result and the checks, and nothing touches
your world until you say so. One person asks two things in one breath — one everyday, one
technical — and then *leaves*. She comes back to results and one clear decision.

## Cut

| Time | Picture | On-screen line | Source |
|---|---|---|---|
| 0–4 | Kitchen, toddler on hip, she tells the laptop what she needs | — | Clip A |
| 4–6 | Keys off the hook, out the door | — | Clip A |
| 6–10 | Plan card assembles (Screen 2) | **vak plans it.** | Clip A |
| 10–12.5 | Result card with checks (Screen 1) | **Does it. On a copy.** | Clip B |
| 12.5–16.5 | Café, thumb taps *Accept into workspace* | — | Clip B |
| 16.5–18.5 | Shared conversation, Priya's swap (Screen 4) | **Bring a friend.** | Clip C |
| 18.5–20 | Botanic garden with the kids | — | Clip C |
| 20–25 | Songbird lands beside the mark | **Ask. Then go live your day.** | Clip C |

Audio: `audio/music-take-1.mp3` at −2 dB with a fade from 23.2 s; pencil-line SFX on each card
slide (6.5 / 7.5 / 8.5 s), ticks on the checks (10.6 / 11.1 s) and the accept (15.3 s), thud +
chirp on the bird landing (20.3 / 20.45 s).

## Pipeline

1. **Keyframes** — GPT Image 2.5 (`gpt-image-2.5-sunburst`, 16:9, 2K) with the experience
   screens, mascot and mark wired as references. Prompts in [`SCRIPT.md`](SCRIPT.md). Chosen
   frames are in `keyframes/`.
2. **Motion** — Kling 3 Pro (`kling-3-pro`, 10 s, 1080p, silent), each clip pinned to a start and
   an end keyframe so every screen is exact:
   - A: `01-kitchen-ask` → `02-plan-screen` — `clips/clip-A-kitchen-to-plan.mp4` (generation `pMsYPhoFwEe5bwgNms7u`)
   - B: `03-result-screen` → `04-phone-accept` — `clips/clip-B-result-to-phone.mp4` (`Q6jRQivectxYkI3OxqvM`)
   - C: `05-coworking-screen` → `06-end-card` — `clips/clip-C-coworking-to-endcard.mp4` (`FmnXwSMZnXJZmpAsrria`)
3. **Music** — ElevenLabs Music v2, 30 s instrumental, 120 bpm. Two takes in `audio/`.
4. **SFX** — ElevenLabs Sound Effects v2. The chirp is the brand's sonic logo.
5. **Edit** — ffmpeg (command in [`SCRIPT.md`](SCRIPT.md)). B and C's UI holds are trimmed
   because the card was already present in the pinned start frame.

Raw Kling clips are committed in `clips/` (≈ 24 MB each) so the edit can be re-cut without the
canvas; the same generations remain on the canvas for re-rolls.

## Cost record

Keyframes (6 × 2 variants) + music ≈ 11,250 credits; three clips ≈ 20,360 credits. Total ≈ 31,600
credits (≈ ₹506). A 30 s Seedance 2.5 master shot was priced at 187,944 credits (1080p) and not run.

## Clip B1: the card landing (2026-10-15)

Clip B's first 2.5 s was a hold, because its start frame already showed the card. Rather than
re-roll the whole 10 s, Clip B1 is a 5 s `kling-3-pro` clip from an empty result screen to `03`:

- `keyframes/03b-empty-start.png`: `03` with the card, headline and footer removed by an image edit
  (`gpt-image-2.5-sunburst`), and the Mira header pasted back from `03` with ffmpeg so the two frames
  align pixel for pixel (an image-model fix-up shifted the layout and was not used).
- `clips/clip-B1-card-landing.mp4` (generation on node `JaKvgzGOJJWM64yHbpCa`); the v2 edit plays it
  at 2× in the 2.5 s slot, and keeps Clip B's phone segment (6–10 s) as before. Prompt in
  [`SCRIPT.md`](SCRIPT.md).
- One flaw: the sidebar's "Orion" circle turns green for about a second mid-clip (half that at 2×).
- Cost: about 4,940 credits (empty frame takes ~870, an unused fix-up ~675, the clip ~3,390).

## Known improvements

- Clip A: Kling dissolves from the door into the UI; a hard cut at 6 s is peppier and free to make
  in the edit.
- Add a voice-over later if wanted: Sana (`tKZQTIqwDrPzLv6MrPxF`), Eleven v4, spell `Vaak`.

## Deliverables still to cut from the same sources

- 8 s YouTube intro (clip C from 17 s + chirp)
- muted loop for the website hero (clips A–C, no audio, hold on the end card)
- 1:1 and 9:16 social crops
