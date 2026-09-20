# vak spot — prompts and edit

Everything needed to regenerate the spot on the ElevenLabs canvas. Reference order matters: the
image models cite references in the order they are wired.

## Keyframes — `gpt-image-2.5-sunburst`, 16:9, 2K, quality high

### 01 — kitchen ask (character anchor; no references)

> Photograph for a warm, upbeat lifestyle commercial. Medium shot, eye level, 35mm lens, soft morning window light from the left. Subject: an Indian woman in her early thirties, hair in a loose low bun, small gold hoop earrings, mustard-yellow cardigan over a white t-shirt, holding a laughing two-year-old on her hip. She stands at a bright kitchen counter with a bowl of cereal, a glass of orange juice and an open laptop. She is mid-sentence, looking at the laptop with an amused, confident expression. Real skin texture, natural colour, no retouching look. Clean, cheerful, modern home; pale wood and white tile. This image defines her face, hair, wardrobe and the kitchen for later shots.

### 02 — plan screen (refs: Screen 2, mascot, mark)

> UI keyframe for a product commercial, full-frame app screenshot, no device bezel. Reproduce the layout, palette, serif headline typography and card style of the first reference (the everyday plan screen) exactly: warm paper background, left sidebar with "Vak" wordmark, agent header "Mira · Here", a user bubble "Help me plan Saturday with the kids", and the plan card "Saturday with the Kids" with Morning / Afternoon / Evening rows and two venue option cards on the right with buttons "Adjust plan" and "Add to calendar". Composer at the bottom reads "Ask or tell Mira anything" with a green microphone button. Replace the small wave glyph next to the agent name with the songbird from the second reference at 40 px, calm expression. Add one large on-screen headline in the upper-right empty area of the paper, editorial serif, ink #23211c: EXACT TEXT "vak plans it." verbatim, no extra characters. All UI text crisp and correctly spelled. The third reference is the app icon; do not place it here. No shadows, no glow, flat tonal depth.

### 03 — result screen (refs: Screen 1, mascot)

> UI keyframe for a product commercial, full-frame app screenshot, no device bezel. Reproduce the layout, palette, serif typography and card style of the first reference (the everyday result screen) exactly: warm paper background, left sidebar with "Vak" wordmark, a user bubble "Fix the checkout total rounding issue", the agent headline "The rounding issue is fixed in a draft." and the result card "Checkout total" with Before "$28.335" and After "$28.34", a Changes column "2 files changed / 1 file added", a Checks column with two green ticks "8 passed" and "Browser flow checked", buttons "Review changes" (green) and "Open preview", and the footer line "Working on an isolated copy · Ready to review" with "Your source workspace has not been changed." on the right. Replace the small wave glyph next to the agent headline with the songbird from the second reference at 40 px, calm. Add one large on-screen headline in the empty upper-right paper area, editorial serif, ink #23211c: EXACT TEXT "Does it. On a copy." verbatim, no extra characters. All UI text crisp and correctly spelled. No shadows, no glow, flat tonal depth.

### 04 — phone accept (refs: 01 kitchen, Screen 3, mascot)

> Photograph for an upbeat lifestyle commercial. Over-the-shoulder close-up, 50mm, shallow depth of field, bright café daylight. The same woman as the first reference (same face, low bun, gold hoops, mustard cardigan) stands in a short coffee-shop queue holding her phone at chest height; the phone screen fills the right half of the frame, sharp. On the phone: the vak mobile UI on warm paper, in the style of the second reference: at the top the songbird from the third reference at 32 px next to the name "Mira", a headline in serif "Two things ready" and two stacked cards. Card one: "Saturday with the Kids" with a small calendar icon and a green button EXACT TEXT "Add to calendar". Card two: "Checkout rounding fix" with two green ticks and a green button EXACT TEXT "Accept into workspace". Beneath, small grey text "Nothing changes until you say so." Her thumb hovers just above the second button. All phone text crisp and correctly spelled, verbatim, no extra characters. Café background softly blurred, warm and cheerful.

### 05 — coworking screen (refs: Screen 4, Screen 2, mascot)

> UI keyframe for a product commercial, full-frame app screenshot, no device bezel. Use the split layout, palette and typography of the first reference (conversation on the left, live artifact on the right) but the artifact on the right is the "Saturday with the Kids" plan card from the second reference, not a shop. Left conversation column: agent header with the songbird from the third reference at 40 px and the name "Mira"; a message from "Priya" (an invited person, small avatar with initial P) reading "Swap the museum for the garden?"; then Mira's reply "On it — updating the afternoon." Top of the right pane shows a small chip "Shared with Priya" and a "Review changes" button. Add one large on-screen headline in the empty upper area of the right pane, editorial serif, ink #23211c: EXACT TEXT "Bring a friend." verbatim, no extra characters. All UI text crisp and correctly spelled. Warm paper, flat, no shadows, no glow.

### 06 — end card (refs: mascot, mark)

> End card for a commercial, flat graphic on a uniform warm paper background #f4f1ea, nothing else in the scene. Left of centre: the songbird from the first reference, large, standing, calm attentive expression, eyes open, facing slightly right toward the mark, with a soft contact shadow only. Right of centre, at the bird's eye height: the glyph from the second reference reproduced exactly — the navy Devanagari letter with the small right chevron, the amber bar above and the short blue dash below — drawn directly on the paper with no tile, box or background behind it, about the same height as the bird's body. Below both, centred, one line in an editorial serif, ink #23211c: EXACT TEXT "Ask. Then go live your day." verbatim, no extra characters. Generous whitespace, no other elements, no gradients, no glow.

(The committed `06-end-card.png` has a URL line painted out; do not add a URL until a domain is live.)

## Clips — `kling-3-pro`, 10 s, 16:9, 1080p, generate_audio off

Shared negative prompt: `extra people, distorted face, extra fingers, morphing, flicker, warped letters, misspelled text, invented UI, dark theme, neon, glow`

### Clip A — start `01`, end `02`

> Upbeat lifestyle commercial, bright warm daylight, real-film look, fast cuts. Shot 1 (0-4s): medium shot in the kitchen from the start frame; the woman with the toddler on her hip turns to the laptop and speaks one brisk cheerful sentence to it, gesturing with her free hand; the toddler laughs. Shot 2 (4-6s): quick cut to her hand grabbing keys off a hook, then the front door swinging open into sunlight as she steps out with the toddler. Shot 3 (6-10s): hard cut to a full-screen flat app interface on warm paper: the sidebar, header and empty conversation are already there; the "Saturday with the Kids" plan card slides up into place, then the two venue option cards slide in from the right one after another, then the large serif headline "vak plans it." pops in at the top right, and everything settles exactly into the end frame and holds. Consistent character, no camera shake in the UI shot, crisp text.

### Clip B — start `03`, end `04`

> Upbeat lifestyle commercial, fast cuts. Shot 1 (0-5s): the flat app interface from the start frame, on warm paper, held perfectly still by the camera: the "Checkout total" result card lands with a small bounce, the Before and After values appear, the two check marks in the Checks column tick green one after the other, and the large serif headline "Does it. On a copy." pops in at the top right; the footer line stays readable. Shot 2 (5-10s): hard cut to a bright café, over-the-shoulder close-up of the same woman in the mustard cardigan holding her phone, exactly the end frame's framing: the phone shows two cards; her thumb presses the green "Accept into workspace" button, the button flashes to a green tick, she smiles, and the shot settles exactly into the end frame. Crisp, correctly spelled text throughout; consistent character.

### Clip C — start `05`, end `06`

> Upbeat lifestyle commercial, fast cuts. Shot 1 (0-4s): the flat split-view app interface from the start frame on warm paper, camera locked: Priya's message bubble "Swap the museum for the garden?" slides in on the left, Mira's reply appears beneath it, the afternoon option cards on the right swap places with a quick smooth reorder, and the large serif headline "Bring a friend." pops in. Shot 2 (4-7s): hard cut to a sunny botanic garden path: the same woman in the mustard cardigan walks with her toddler, laughs, glances at her phone and slips it into her pocket. Shot 3 (7-10s): hard cut to plain warm paper: the indigo-and-saffron songbird drops in from above and lands with a tiny bounce on the left, blinks once with calm open eyes; beside it the navy Devanagari mark with its amber bar and blue dash fades in crisply with no box behind it; below, the serif line "Ask. Then go live your day." settles in; everything holds perfectly still, matching the end frame exactly.

## Music — `eleven_music_v2`, 30 s, instrumental

> Peppy, feel-good indie-pop instrumental for a 30-second lifestyle commercial, 120 bpm, in a bright major key. Starts immediately with a snappy hand-clap and bright acoustic guitar strum, plucky synth bass and a light kick; a glockenspiel hook enters at 4 seconds. Clear, punchy downbeats every two bars so cuts can land on them. Builds with layered claps and a shaker at 12 seconds, lifts again at 20 seconds, then a clean full stop on the downbeat at 26 seconds followed by one warm sustained chord that resolves and fades by 30 seconds. Sunny, modern, confident, a little playful; the feel of a Google or Apple lifestyle ad. No vocals, no risers, no cinematic drums.

## SFX — `eleven_text_to_sound_v2`, prompt influence 0.7–0.8

| File | Duration | Prompt |
|---|---|---|
| sfx-chirp | 0.8 s | A single short, bright, friendly two-note songbird chirp, rising then settling, clean and close, no background, no reverb, cartoon-cute but natural |
| sfx-pencil-line | 1.5 s | Soft pencil stroke on smooth paper, one single continuous line, close mic, quiet, dry, no room reverb |
| sfx-tick | 0.6 s | One single small wooden tick, like a light switch on a desk instrument, dry, no reverb, quiet, short |
| sfx-thud | 1.0 s | Soft low felt thud, like a thick book set down gently on a wooden desk, dry, short decay, no reverb |

## Edit (ffmpeg)

Inputs in order: clipA.mp4, clipB.mp4, clipC.mp4, music-take-1.mp3, sfx-pencil-line.mp3, sfx-tick.mp3, sfx-chirp.mp3, sfx-thud.mp3.

```text
[0:v]fps=24,trim=0:10,setpts=PTS-STARTPTS[a];
[1:v]fps=24,trim=0:2.5,setpts=PTS-STARTPTS[b1];
[1:v]fps=24,trim=6:10,setpts=PTS-STARTPTS[b2];
[2:v]fps=24,trim=0:2,setpts=PTS-STARTPTS[c1];
[2:v]fps=24,trim=5:10,setpts=PTS-STARTPTS,tpad=stop_mode=clone:stop_duration=1.5[c2];
[a][b1][b2][c1][c2]concat=n=5:v=1:a=0[vout];
[3:a]volume=0.79,afade=t=out:st=23.2:d=1.8,atrim=0:25[mu];
[4:a]adelay=6500|6500,volume=0.22[l1]; [4:a]adelay=7500|7500,volume=0.22[l2]; [4:a]adelay=8500|8500,volume=0.22[l3];
[5:a]adelay=10600|10600,volume=0.35[t1]; [5:a]adelay=11100|11100,volume=0.35[t2]; [5:a]adelay=15300|15300,volume=0.4[t3];
[7:a]adelay=20300|20300,volume=0.35[th]; [6:a]adelay=20450|20450,volume=0.45[ch];
[mu][l1][l2][l3][t1][t2][t3][th][ch]amix=inputs=9:normalize=0:duration=first,aformat=channel_layouts=stereo,alimiter=limit=0.95[aout]
```

Encode: `-t 25 -c:v libx264 -crf 18 -pix_fmt yuv420p -c:a aac -b:a 192k -movflags +faststart`.
