# Vakyartha media: sound, image and film

Status: in progress (2026-10-13). Batches 1 and 2 generated in ElevenLabs;
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

## Batch 2: the companions (2026-10-13)

The eight built-in companions are defined in
`crates/vak-client-ui/src/agentGlyph.ts`: each has a kind, a personality
and a two-note `cue` (in Hz) that the app already plays. Every sound and
voice below grows from those, so a character sounds the way it looks and
behaves.

**Direction.** Each companion is a *casting*, not a costume. They share
Vakyartha's manners (they listen first, say what they did, never act
without you), and differ in temperament. Each line is written to show that
temperament *doing real work* (a contract, a trip, a budget, a list), never
introducing itself. No catchphrases, no cartoon voices, no accents used as
jokes.

**Latest models used (checked 2026-10-13):** voice design `eleven_ttv_v3`,
speech `eleven_v4`, music `eleven_music_v2_5`, sound effects
`eleven_text_to_sound_v2` (the only one), video Kling 2.5 Turbo for the
first test (Kling 3 Pro, the best available, costs about 5,100 credits per
5 s clip versus about 2,100).

### Casting sheet

| Character | Cue | Signature sound | Voice brief | Line |
|---|---|---|---|---|
| **Vakyartha** (songbird) | G4→D5 | the brand chime (batch 1) | Warm, clear, gender-neutral adult, mid-range, close-mic. Calm and attentive, like someone who has really listened. Unhurried, gentle smile, precise, no salesy energy. | "I read the contract Priya sent. Three things changed, and I've marked each one so you can see exactly where. I've drafted a reply too, but nothing goes out until you say so. Ready when you are." |
| **Mira** (fox) | G4→C5 | felt piano, faint brushed-tail swish | Warm, slightly low female, thirties, perceptive, quietly confident, a hint of wit. Smooth, relaxed; one step ahead but never shows off. | "You mentioned the trip twice this week, so I looked into it. The Thursday train is cheaper and gets you there before dinner. I've kept both options side by side, in case you'd rather sleep in." |
| **Moss** (forest friend) | E4→G4 | wooden kalimba, leaf rustle | Deep, soft, earthy older male, slow with natural pauses, textured like bark; a gardener who never rushes anyone. | "There's no hurry. Let's take this one step at a time. First we'll gather what you already have, then we'll see what's missing. Good things grow slowly, and so do good plans." |
| **Nori** (seal) | A4→D5 | round vibraphone, water plip | Bright young adult, gender-neutral leaning female, curious and practical, slightly bubbly; a teammate who loves figuring things out. | "Ooh, this is a good puzzle. Okay, here's what I found: the numbers don't add up in March, and I think I know why. Want me to fix it, or shall we look at it together first? Either way, you've got this." |
| **Pip** (penguin) | C5→E5 | toy glockenspiel, waddle tap | Bright, quick, cheerful young voice, crisp, energetic but never shrill; neat short sentences with a spring in each. | "Got it! Your list is sorted. Groceries first, then the pharmacy, then the post office on the way home. I've put the busy hours in red so you can skip the queues. Anything else? I'm on it." |
| **Lumi** (cloud friend) | B4→F♯5 | airy pad and celesta, a breath of wind | Soft, airy, dreamy female, light breathy texture, slow floating cadence; a calm bedtime storyteller full of quiet wonder. | "Close your eyes for a second and picture it: a little shop with blue shutters and the smell of fresh bread. That's your idea. Let's sketch it together, one cloud at a time, and see where it drifts." |
| **Tavi** (owl) | F4→B♭4 | warm wooden flute, breathy hoot | Mature, measured older woman, low and resonant, deliberate pauses; a respected professor who chooses every word. | "Before we decide, consider this. Two of your sources agree, and one does not. The one that disagrees is the most recent. That is worth a closer look. I have laid out all three, so you can judge for yourself." |
| **Beni** (rabbit) | D4→G4 | mellow marimba, soft-paw thump | Calm, steady, warm male, forties, mid-low, even rhythm, soft-spoken; someone you trust to handle things while you sleep. | "Everything's in order. The backup finished at two, your reminders are set for the morning, and the bill is paid. Nothing needs you tonight. If anything changes, I'll let you know first thing." |

**Audience: India first, then the world (decided 2026-10-13).** The first
round above was cast neutral-American and is superseded. Every companion is
recast in Indian English: an educated, pan-Indian accent, never a regional
caricature, a call-centre script or a newsreader's delivery, and clear to a
listener abroad. Each brief above keeps its age, pitch and temperament and
adds that accent; ages are made explicit (Moss sixties, Tavi late fifties,
Beni forties, Mira thirties, Vakyartha early thirties, Nori and Pip
twenties). Lines are lightly localised, never translated for flavour: Mira's
trip is to Pune, Moss says *beta*, Pip's list starts with *sabzi* and the
*chemist*, Lumi's café has chai on the stove, Beni pays the *electricity*
bill. A voice is chosen by listening at low volume on a phone speaker, and
it must stay intelligible to someone who has never heard an Indian accent.

**Saved voices (2026-10-13).** In the owner's ElevenLabs voice library,
each labelled `brand=vakyartha`, `character`, `kind`, `accent=indian-english`,
`age`, `gender`, `role` and `casting=2026-10-13`:

| Character | Library name | Voice id | How it was chosen |
|---|---|---|---|
| Vakyartha | Vakyartha – Vakyartha (songbird) | `FQKmle1kREMgn23tqx5q` | pace (15.4 s) |
| Mira | Vakyartha – Mira (fox) | `CwhHeaJVU9ZN4RMoaabk` | pace (14.6 s) |
| Moss | Male Voice 1 (rename to "Vakyartha – Moss (forest friend)") | `sRPGZSnzBPol5Pjt6JFD` | the owner, by ear |
| Nori | Vakyartha – Nori (seal) | `wUQmDsGn7RMvveESnUd8` | pace (13.2 s) |
| Pip | Vakyartha – Pip (penguin) | `IORPyjxKm2m9lE40qnBG` | pace (10.7 s) |
| Lumi | Vakyartha – Lumi (cloud friend) | `fyGNdOr9n4ggf2kF4hNa` | pace (21.7 s) |
| Tavi | Vakyartha – Tavi (owl) | `BBfXBXyyyYUJQ4q3gTUP` | pace (21.6 s) |
| Beni | Vakyartha – Beni (rabbit) | `w9t4Ig8oukL09KRjPaIW` | pace (14.4 s) |

Only Moss was chosen by listening. The other seven were chosen by the one
thing measurable without hearing them, the take's length against the
brief's pacing (quick for Pip and Nori, middle for the steady ones,
unhurried but not dragging for Tavi and Lumi). They are provisional until
the owner has listened; a voice that fails the listening test is replaced
by designing again, not by keeping it.

Voice design returns three previews per character; choose one and save it
to the ElevenLabs voice library (the preview links expire after two hours,
the saved voice does not). Download a WAV of the chosen preview for the
repo either way.

**Signature-sound prompt template** (`eleven_text_to_sound_v2`, 2
variations each):

> Character signature sound, 1 second: a {kind} {one-word mood}. Two notes
> {cue note 1} then {cue note 2} on a {instrument}, with a {tiny foley
> detail}. {Personality}. Dry, close, no voice.

### Animation

Idle loop for each companion, from its 512 px portrait
(`docs/brand/characters/<id>.png`, attached by its public raw GitHub URL):

> Seamless looping idle animation of this exact character, unchanged
> design, flat style and colours preserved, plain background kept. The
> {kind} breathes softly, tilts its head once as if listening, blinks,
> {one signature gesture}, then settles back to the starting pose. Locked
> camera, 5 seconds, gentle and calm.

Signature gestures: Vakyartha lifts its three-feather crest; Mira flicks
her tail; Moss's leaves stir; Nori's whiskers twitch; Pip flaps once;
Lumi drifts up and back; Tavi turns its head slowly; Beni's ears lift.
Batch 2 ran Vakyartha only (Kling 2.5 Turbo, about 2,100 credits).

### Theme music

`eleven_music_v2_5`, 2 variations, about 900 credits each:

> 30-second instrumental brand theme for Vakyartha, a calm and trustworthy
> AI companion. Opens with a solo glass marimba playing a rising three-note
> songbird motif (G4, B4, D5), answered by soft felt piano. Warm strings
> and a gentle celesta enter, light brushed percussion, a hopeful lift at
> 20 seconds, then resolves back to the three-note motif and ends softly.
> Mood: attentive, warm, quietly confident, morning light. Modern cinematic
> minimal, 84 BPM, no vocals.

The same three-note motif is the brand chime, so the film's end card, the
app's launch sound and the theme are one idea at three lengths.

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
