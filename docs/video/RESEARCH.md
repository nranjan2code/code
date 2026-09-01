# Remotion production research

Research date: 2026-09-01

## Recommendation

Use Remotion as the source-of-truth animation and rendering layer. Keep VAK architecture facts in TypeScript scene data, generate narration as a separate audio asset, generate word-timestamp captions, and let Remotion compose the visual timeline, voice, music, captions, and final MP4.

Remotion is a good fit because a composition is a React component with explicit width, height, FPS, duration, and serializable props; `Sequence` can stage scenes; timing helpers such as `interpolate()` and `spring()` can animate diagram elements; and the renderer supports local CLI, Node/SSR, GitHub Actions, and cloud options. See the [project creation docs](https://www.remotion.dev/docs/), [Composition](https://www.remotion.dev/docs/composition), [Sequence](https://www.remotion.dev/docs/sequence), [interpolate](https://www.remotion.dev/docs/interpolate), [spring](https://www.remotion.dev/docs/spring), and [rendering](https://www.remotion.dev/docs/render).

## Audio and voice

Remotion supports importing, delaying, trimming, mixing, changing volume/speed/pitch, visualizing, and exporting audio. See [Using audio](https://www.remotion.dev/docs/using-audio). For this project, use a generated narration track plus low-volume music and optional transition sound effects.

The voice-generation script uses the OpenAI Speech API because it supports text-to-speech with selectable voices, style instructions, and MP3/WAV output. The API accepts up to 4096 characters per speech request, so longer narrations should be chunked if the script grows. See the [Create speech API reference](https://platform.openai.com/docs/api-reference/audio/createSpeech).

For captions, Remotion supports importing `.srt`, generating captions from audio, rendering captions, and exporting them. The `@remotion/captions` format uses `text`, `startMs`, `endMs`, `timestampMs`, and `confidence`; Remotion’s caption guidance also supports word highlighting. See [Remotion Captions](https://www.remotion.dev/docs/captions) and the official [caption display guide](https://github.com/remotion-dev/remotion/blob/main/packages/skills/skills/remotion-captions/display-captions.md).

The repository script uses OpenAI transcription word timestamps. An alternative is Remotion’s Whisper workflow through [`@remotion/install-whisper-cpp`](https://github.com/remotion-dev/skills/blob/main/skills/remotion-best-practices/remotion-captions/transcribe-captions.md). Remotion’s [`@remotion/elevenlabs`](https://www.remotion.dev/docs/elevenlabs) package converts ElevenLabs Speech-to-Text output into Remotion caption objects; it is a caption integration, not the voice-generation step itself.

## Motion language

Use a small, intentional vocabulary: spring-in cards, traced arrows, fades, slides, and controlled zooms. Remotion’s transition package provides `TransitionSeries`, spring/linear timings, and presentations such as fade, slide, wipe, and dissolve; see [Transitions](https://www.remotion.dev/docs/transitions). Avoid 3D and decorative motion because architecture videos need stable spatial relationships.

## Rendering and delivery

Start with Remotion Studio for previews, then use the CLI for a local MP4. For repeatable CI renders, use the Node/SSR API or GitHub Actions. See [Render your video](https://www.remotion.dev/docs/render) and [Server-side rendering](https://www.remotion.dev/docs/ssr).

## Licensing note

Remotion’s licensing depends on whether the user is an individual, small company, nonprofit, or another organization, and on whether the project is local creator work or an automated rendering product. Check the current [Remotion license](https://www.remotion.dev/license) before publishing or automating production renders. Do not assume the local free-use terms cover every future distribution model.

## Research limits

This research covers the current official Remotion workflow and the chosen OpenAI audio path. It does not select a music library or clear third-party music licenses. Use original, properly licensed, or public-domain music for the final production.
