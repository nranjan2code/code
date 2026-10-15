# "Heard" — the generated shots (2026-10-15)

The 60-second brand film "Heard" is scripted in
[`docs/brand/media/README.md`](../../brand/media/README.md) ("The film").
Its rule: generated video only for the desk and window shots, never for
the product screens, which come from real recordings of the app. This
folder holds those two generated shots and their keyframes. The edit is
not made yet.

Canvas (all sources, re-rollable): ElevenLabs flow **Heard film 2026-10**
(`zPKK08EKJxvfyMCBWJvk`).

## Shots

| Film time | File | Keyframe | Length |
|---|---|---|---|
| 0–6 s | `clips/shot-1-desk-morning.mp4` | `keyframes/01-desk-morning.png` | 6.0 s |
| 44–52 s | `clips/shot-6-window-wide.mp4` | `keyframes/06-window-wide.png` | 8.0 s |

`*-alt.png` are the second keyframe take of each, not animated.

**The laptop screen is a flat mid-grey panel on purpose.** It is a clean
plate for the real screen recording, tracked in at edit time; the camera
is locked, so a four-corner pin holds for the whole shot. In the window
shot the laptop faces away from the camera, so the script's "the laptop
shows the Library" cannot be seen there: cut to the real Library
recording before or after it instead.

## Pipeline

1. **Keyframes**: `gpt-image-2.5-sunburst`, 16:9, two takes each; the
   chosen take is the one whose screen faces the camera most squarely
   (desk) and whose window and gaze read clearest (window).
2. **Motion**: `kling-3-pro`, 1080p, silent, start frame only, camera
   locked.

### Keyframe 01 — desk, morning

> Film still, opening shot of a calm brand film. Locked-off medium-wide
> shot of a real wooden desk by a window on an unhurried morning, warm
> early daylight from the left, 35mm lens, natural film colour, real
> textures. On the desk: an open silver laptop seen at a three-quarter
> angle, its screen a flat uniform mid-grey panel with nothing on it
> (reserved for a screen insert later); a pencil and a small paper
> notebook; a few leaves of a potted plant at the edge. A woman's hand,
> sleeve of a soft mustard cardigan, is just setting down a white ceramic
> cup of tea beside the laptop; only her hand and forearm are in frame.
> Quiet, attentive, uncluttered. No text anywhere, no logos, no screen
> content, no people's faces, no tech glow, no bokeh blur on the laptop.

### Shot 1 — 6 s

> Locked-off camera, absolutely still, calm brand film. From the start
> frame: the woman's hand finishes setting the white tea cup down gently
> on its coaster, pauses a moment, then slides back out of frame to the
> right. A thin wisp of steam rises from the tea. Morning light shifts very
> slightly as a leaf outside the window moves; the potted plant's leaves
> stir faintly. The laptop screen stays a perfectly flat, uniform mid-grey
> the whole time, unchanged. Unhurried, quiet, real-film look, natural
> colour.

Negative: `camera movement, zoom, shake, text, logos, screen content, glow
on the laptop screen, extra hands, distorted fingers, morphing, flicker`.

### Keyframe 06 — window, wide

> Film still from a calm brand film. Locked-off wide shot of the same
> room: a wooden desk by a large window in warm late-morning daylight,
> 35mm lens, natural film colour. An Indian woman in her early thirties,
> hair in a loose low bun, small gold hoop earrings, soft mustard cardigan
> over a white t-shirt, sits back from the desk holding a white ceramic
> cup of tea in both hands and looks out of the window, relaxed and
> content; seen in three-quarter profile. On the desk an open silver
> laptop at a three-quarter angle, its screen a flat uniform mid-grey
> panel with nothing on it (reserved for a screen insert later), a pencil
> and a small notebook, a potted plant. Real skin texture, no retouching
> look. Quiet, unhurried, plenty of air in the frame. No text anywhere, no
> logos, no screen content, no tech glow.

### Shot 6 — 8 s

> Locked-off camera, absolutely still, calm brand film, real-film look,
> natural colour. From the start frame: the woman sits back relaxed,
> raises the cup and takes one slow sip of tea, lowers it to rest in both
> hands, and keeps looking out of the window with a small content smile.
> Outside, leaves move gently in a light breeze; the sheer curtain stirs
> slightly; the warm late-morning light holds steady. Nothing else moves.
> Same face, hair and clothes throughout; the laptop stays exactly as it
> is. Unhurried and quiet.

Negative: `camera movement, zoom, pan, shake, text, logos, screen content,
extra people, distorted face, extra fingers, morphing, flicker`.

## Cost record

Keyframes 4 × ~190 credits; shot 1 ~4,070; shot 6 ~5,430. About 10,260
credits in all.
