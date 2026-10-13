# Vakyartha media: sound, image and film

Status: in progress (2026-10-13). First assets generated in ElevenLabs;
choices and downloads pending.

This folder records how Vakyartha's media is made, so anyone can make the
next piece in the same voice: the creative direction, the exact prompts, the
tools and settings, and what was chosen and why. The visual identity itself
(the Songbird mark, lockups, icons) is `docs/brand/README.md`; nothing here
replaces it. Generated media never redraws the mark: where the Songbird
appears in a finished piece, it is placed from the SVG master.

## Creative direction

**The one idea.** Vakyartha is the meaning carried by a sentence. Everything
we make shows a moment of *being understood*: you say something plainly, and
it is heard, understood and carried into work that you can see and check.

**The character.** The Songbird listens before it sings. Every sound, image
and scene is calm, attentive and exact. It does not show off. It never
shouts, and it never looks like magic: what it does is visible.

**What we never do.** No glowing brains, no robots, no neon circuitry, no
"AI is thinking" particle swirls, no stock people pointing at screens, no
claims the product cannot back up (see the brand README's Meaning section).

**Palette.** Indigo ink `#101D3D` carries the weight; saffron `#F5A400` is
the one accent, used only for the moment something is live or answered;
everything else is warm off-white and soft neutral grey (`DESIGN.md`).

**Sound language.** One instrument family across every sound: soft glass
marimba and celesta, dry, close and warm. The brand chime is a short rising
three-note phrase, the "song" of the Songbird; each interface sound is a
fragment of that phrase, so the app sounds like one voice:

| Moment | Shape | Feeling |
|---|---|---|
| Brand chime (launch, film end card) | three rising notes + short shimmer, ~2 s | "I'm here, and I'm listening" |
| Turn finished | two rising notes, short decay, < 1 s | "done, and it's right there" |
| Approval needed | two notes, the second higher and held | a polite question, never an alarm |

Interface sounds are quiet enough to hear fifty times a day without
irritation. No reverb tails; they must end before the next thing happens.

## Workflow

1. Write the brief in this file first: the moment, the feeling, the shape.
2. Generate in one ElevenLabs flow per batch (ElevenCreative canvas), four
   variations per prompt, so there is a real choice.
3. Choose by listening on laptop speakers *and* headphones, at low volume,
   next to the app. For images, choose at 390 px wide as well as full size.
4. Download the chosen take. Audio: normalise to about −16 LUFS for the
   chime and −22 LUFS for interface sounds, trim silence to under 20 ms at
   the start, export WAV (master) and OGG/MP3 (web). Images: export PNG
   master and a WebP for the site.
5. Commit the master and its web copies here, and add a row to the log
   below with the prompt, model and which variation was chosen.

Credits are read in the ElevenLabs app (Profile → Subscription); the
connector cannot read them.

## Batch 1 (2026-10-13)

Flow: **Vakyartha brand assets 2026-10** (`uutJRnMAgX5LIOXroskb`) in the
owner's ElevenLabs workspace. Spent about 960 credits (12 sound effects at
~17 each, 4 images at ~190 each).

### Brand chime

Model `eleven_text_to_sound_v2`, 4 variations.

> Premium software brand sonic logo, 2 seconds. A warm, bright three-note
> rising songbird-like chirp played on soft glass marimba and celesta,
> followed by a gentle sustained shimmer that fades out. Calm, confident,
> intelligent, friendly. Clean studio mix, no reverb wash, no drums, no
> voice, ends in silence.

### Turn finished

Model `eleven_text_to_sound_v2`, 4 variations.

> Subtle app notification sound, under 1 second: task complete. Two soft
> ascending glass marimba notes, warm and reassuring, very short decay.
> Minimal, modern, quiet enough for repeated use in a desktop app. No reverb
> tail, no voice.

### Approval needed

Model `eleven_text_to_sound_v2`, 4 variations.

> Gentle attention sound for a desktop app, under 1 second: your approval is
> needed. A soft two-note questioning chime, second note slightly higher and
> held briefly, soft bell and marimba timbre. Polite, noticeable but never
> alarming. Clean, dry, no voice.

### Website hero illustration

Model `gpt-image-2.5-sunburst`, 4 variations, 16:9.

> Website hero illustration for an AI agent product called Vakyartha. Flat
> geometric editorial illustration: a stylised crested songbird in deep
> indigo (#101D3D) with a saffron (#F5A400) throat, perched calmly on a thin
> line that flows into abstract shapes of documents, calendar cards, chat
> bubbles and a small shield, suggesting safe, transparent help with
> everyday work. Generous negative space, warm off-white background, indigo
> and saffron only with soft neutral greys. Calm, premium, modern, minimal,
> wide 16:9 composition with empty space on the left for a headline. No
> text, no letters, no logos, no gradients mesh, no 3D.

The bird here is an illustration, not the logo; the site places the real
mark from the SVG master.

### Chosen

| Asset | Variation | Notes |
|---|---|---|
| Brand chime | — | pending the owner's listen |
| Turn finished | — | pending |
| Approval needed | — | pending |
| Hero illustration | — | pending |

## The film: "Heard" (60 seconds, script)

The film for vakyartha.com. One continuous idea: a person speaks plainly,
and the work happens where they can see it. No voice-of-God narrator
explaining features; the product's own calm voice speaks only twice.

**Look.** Locked-off, unhurried shots. Warm daylight on a real desk.
Screens are shot straight on, readable, never blurred into "tech bokeh".
Saffron appears only when something is live.

**Sound.** Room tone, a pencil, a kettle; the interface sounds above are the
only music until the end card, where the brand chime resolves the film.

| Time | Picture | Sound |
|---|---|---|
| 0–6 s | Morning. A desk, a laptop open on the Vakyartha app, cursor blinking. A hand puts down a cup. | Room tone. Birds outside, faint. |
| 6–14 s | She types, unhurried: *"Read the contract Priya sent, tell me what changed, and draft my reply."* | Keys. Then silence: the Songbird listens. |
| 14–26 s | The answer arrives as cards: three changes, each quoting the clause; a draft reply beside it. Saffron marks the one card still live. | *Turn finished* sound. |
| 26–36 s | She narrows one change in Review, edits a sentence of the reply herself. Nothing is sent. | Pencil-quiet. |
| 36–44 s | A prompt: *Send this reply to Priya?* Her finger hovers, then confirms. | *Approval needed* sound, then a soft click. |
| 44–52 s | Wide shot: she picks up the cup, looks out of the window. The laptop shows the Library: the contract, its versions, the reply. | Kettle, room tone. |
| 52–60 s | End card on indigo: the Songbird lockup. Line: **"Say it plainly. See it done."** | Brand chime, then silence. |

**Voice (optional, two lines only).** If the film needs a voice, the
product's voice says, warmly and close to the mic, at 14 s: *"Here's what
changed."* and at 36 s: *"Ready when you are."* Generate with
`eleven_v3`, a calm, mid-range, unaccented-neutral voice, stability high.

**Taglines considered.** "Say it plainly. See it done." (chosen: it is the
product's promise and is true of what it does) · "Heard, then done." ·
"Meaning, carried." (too abstract for a first visit).

## Next batches

- Sounds: *error* (one low soft note, falling), *message received from a
  channel* (a single high celesta tap), *voice listening start/stop*.
- Images: Open Graph card (1200 × 630) with the lockup placed from the
  master; three feature illustrations (Review, Library, Automations) in the
  hero's style.
- Film: produce "Heard" from real screen recordings of the app; generated
  video only for the desk and window shots, never for the product screens.
