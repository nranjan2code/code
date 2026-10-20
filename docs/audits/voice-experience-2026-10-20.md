# Voice experience and current providers — 2026-10-20

Status: historical. Investigation, implementation and acceptance record for the
voice setup pass on 20 October 2026. Focused implementation and browser checks
are complete; physical microphone and real-channel acceptance remain open.

## Scope and date

The requested experience is a complete provider/listening/speaking/voice setup,
with existing credentials and usable Settings/Admin. The maintainer selected
faster everyday conversation, current affordable options across providers, and
multilingual use including other Indian languages. They explicitly required
current research rather than remembered model advice. The clock returned 2026-10-20 03:27:44 UTC
(08:57:44 Asia/Kolkata). Provider catalogues were read through an authenticated
server API using the existing shared credential store. No credential file or
real session ledger was opened, no data home was copied, and no key was printed.
The installed service stayed on port 8901. Development ran without gateway
bridges on port 8931 in `/tmp/vak-voice-review/workspace` against the normal
supported data home. The installed version was used for the initial comparison;
current source was built for the changed-route checks.

## Current primary sources

Read on 20 October 2026:

- [OpenAI file transcription](https://developers.openai.com/api/docs/guides/speech-to-text)
  and [transcription overview](https://developers.openai.com/api/docs/guides/transcription):
  `gpt-transcribe` is a file-transcription route, while `gpt-live-transcribe`
  needs a streaming transcription session. These are different transports.
- [OpenAI speech](https://developers.openai.com/api/docs/guides/text-to-speech):
  GPT speech supports speaking directions through `instructions`; older TTS
  engines have different capabilities.
- [OpenAI deprecations](https://developers.openai.com/api/docs/deprecations):
  the older Whisper/4o transcription family has a notified retirement window;
  continuing to list a model does not make it the current recommendation.
- [Gemini transcription](https://ai.google.dev/gemini-api/docs/transcribe):
  dedicated transcription uses Interactions; verbatim mode preserves the words
  that govern the Agent turn. Smart transcription rewrites disfluencies and
  self-corrections, so it is not selected for steering.
- [Gemini speech](https://ai.google.dev/gemini-api/docs/speech-generation):
  Flash-Lite TTS is positioned for conversational cascades. Current speech uses
  structured style metadata and can return a complete WAV rather than raw PCM.
- [Gemini Interactions retention](https://ai.google.dev/gemini-api/docs/interactions-overview):
  explicit `store: false` requests stateless processing for these calls.

- [OpenRouter speech](https://openrouter.ai/docs/guides/overview/multimodal/tts)
  and [transcription](https://openrouter.ai/docs/guides/overview/multimodal/stt):
  dedicated modality catalogues include model-specific voices. Generic format
  compatibility does not imply every upstream speaker accepts MP3: live calls
  found Gemini requires PCM16 at 24 kHz and HeyGen at 44.1 kHz, wrapped as WAV.
- [Qwen's model card](https://huggingface.co/Qwen/Qwen3-ASR-0.6B): Hindi is in its
  list; Tamil, Telugu, Bangla and other Indian languages are absent. A cheap
  Hindi recognizer is not a general Indian-language recommendation.
- [Microsoft's voice list](https://learn.microsoft.com/azure/ai-services/speech-service/mai-voices):
  MAI voices use locale-specific names. This account's Flash catalogue exposes
  Hindi and Indian English, not a full set of Indian-language voices.

These are dated observations, not an evergreen catalogue in runtime code.
Both live accounts returned the current model ids tested below. Account listing
alone does not prove invocation access, quota, cost or latency.

## Findings and changes

The existing microphone, evidence gate, governed turns, speech adapters, shared
credentials and channel overrides were retained. The account picker had treated
almost every non-TTS Gemini model as a listening model, including music, video,
research and realtime routes. All surfaces now consume batch speech choices
from one provider vocabulary. Unknown models remain configurable explicitly.

An everyday suggestion uses only discovered ids. It prefers stable aliases,
dedicated transcription and compact speech families; it is a convenience rather
than a performance guarantee. Provider and both pins are saved in one existing
configuration operation. No provider SDK, second conversation agent, credential
store, durable file or scheduling mechanism was introduced.

Saved Agent voice and speaking style now govern both narration and spoken
replies. OpenAI GPT speech receives style directions. Previews resolve the
selected Agent and work before microphone conversations are enabled; they do
not turn the saved switch on. Missing conversation handles are refused. Changing
provider clears model/voice ids at the Agent and bot/chat overlay boundaries.
Admin's shared editor reads the saved shared voice layer rather than showing an
Agent override as though it were the shared value. Connected-key status no
longer claims a tested voice service is ready.

Dedicated Gemini transcription sends bounded inline audio through Interactions,
requests verbatim output, and reads model text. Both Gemini Interactions speech
operations request `store: false`. Speech playback object URLs are replaced and
released when their Settings/editor closes.

## Current cost and language selection

For the requested broad Indian-language experience, the recommended tested
starting pair is `gemini-3.5-transcribe` plus `gemini-3.8-flash-tts`
(direct or their discovered OpenRouter counterparts), with Flash-Lite kept as
the economy option. This is a provisional selection based on coverage and the
small sample described below, not a language-quality benchmark. The current
[transcription guide](https://ai.google.dev/gemini-api/docs/transcribe) describes
automatic language detection and code-switching across 85+ locales; the
[speech guide](https://ai.google.dev/gemini-api/docs/speech-generation) lists
Flash-Lite coverage for Hindi, Tamil, Telugu, Bangla, Marathi, Malayalam,
Kannada, Gujarati and more. Listing is a coverage claim by the provider,
not a native speaker's acceptance test.

At the [current Google rates](https://ai.google.dev/gemini-api/docs/pricing),
Flash-Lite speech is $0.50 per million input text tokens and $6 per million
output audio tokens through 31 December 2026; the announced rates double on
1 January 2027. Dedicated recognition is roughly $0.005 per audio minute under
the provider's stated input/output assumptions. Free-tier availability does
not mean unlimited requests: the live key returned a 3-request/minute speech
limit during the multilingual test. Production setup must account for its
actual quota.

The English/Hindi economy alternative is
[Qwen3 ASR 0.6B](https://openrouter.ai/qwen/qwen3-asr-0.6b), with a published
API rate of $0.00000333 per audio second (about $0.00020/minute), plus
[Kokoro](https://openrouter.ai/hexgrad/kokoro-82m), starting at $0.62 per million
characters. It passed an English round trip but is not selected for broad
Indian-language use. Kokoro is current within its family, not newly released
in October. [HeyGen Turbo](https://openrouter.ai/heygen/heygen-voice-1-turbo)
is a newer low-latency option released 13 October, currently listed at $4/M
characters under a promotion; that is a dated offer, not a permanent rate.
The ElevenLabs launch discount ended 19 October and is not used in comparisons.

OpenRouter returned 35 speech and 26 transcription entries through the connected
account. Twelve speech vendors publish voices usable for a no-reference setup.
All remain selectable; presets favor latest compact peers, not a fabricated
cross-unit price ranking. Reference-only/default-only models without published
voices remain available individually without an invented preset voice. No new
provider account or key was created.

## Live evidence

Synthetic text: “Please reply with the number four only.” A real synthesis call
produced audio. ffmpeg resampled it to 16 kHz mono PCM with quiet boundaries;
the real voice WebSocket accepted it, server speech evidence admitted it,
transcription returned the exact sentence, and the ordinary configured Agent
answered `4`. No microphone permission was requested for these automated runs.

| Service | Listening | Speaking | Observation |
|---|---|---|---|
| OpenAI | `gpt-transcribe` | `gpt-4o-mini-tts` | Exact transcript and governed answer; initial speech generation about 2.2–3.1 s across short samples |
| Gemini | `gemini-3.5-transcribe` | `gemini-3.8-flash-lite-tts` | Exact transcript and governed answer; initial speech generation about 7.1 s in one sample |

One OpenAI recognition sample took about 1.4 s; one Gemini sample about 5.3 s.
These are tiny, sequential, warm-account synthetic tests, not a benchmark of
language coverage, accents, quality, noisy rooms or provider guarantees. The
text Agent route was the installation's configured route, independently of the
speech provider. Provider-reported rated speech usage is absent; no zero-cost
estimate is fabricated. Full realtime adapters have not been built by this pass.


All twelve generated OpenRouter vendor presets completed an English synthetic
round trip after fixing the PCM-only adapters. The transcript preserved the
request (punctuation varied); every governed reply expressed four. These times
measure a complete short synthesis request, not first streamed audio.

| Vendor | Listening | Speaking | Synthesis sample |
|---|---|---|---|
| canopylabs | `assemblyai/universal-3-6-pro` | `canopylabs/orpheus-3b-0.1-ft` | 2.1 s |
| deepgram | `deepgram/nova-3` | `deepgram/flux-tts` | 4.0 s |
| elevenlabs | `elevenlabs/scribe-v2` | `elevenlabs/eleven-v4-turbo` | 1.1 s |
| google | `google/gemini-3.5-transcribe` | `google/gemini-3.8-flash-lite-tts` | 3.4 s |
| hexgrad | `assemblyai/universal-3-6-pro` | `hexgrad/kokoro-82m` | 2.4 s |
| heygen | `assemblyai/universal-3-6-pro` | `heygen/heygen-voice-1-turbo` | 4.1 s |
| microsoft | `microsoft/mai-transcribe-2` | `microsoft/mai-voice-2.1-flash` | 3.0 s |
| minimax | `assemblyai/universal-3-6-pro` | `minimax/speech-2.8-turbo` | 3.2 s |
| mistralai | `mistralai/voxtral-mini-3b-2507` | `mistralai/voxtral-mini-tts-2603` | 1.6 s |
| qwen | `qwen/qwen3-asr-1.7b` | `qwen/qwen-audio-3.0-tts-flash` | 2.3 s |
| sesame | `assemblyai/universal-3-6-pro` | `sesame/csm-1b` | 7.7 s |
| x-ai | `x-ai/grok-stt-1.0` | `x-ai/grok-voice-tts-1.0` | 1.2 s |

The first batch hit the server's session-creation limit; subsequent checks
reused their disposable conversation rather than weakening the limit. A
separate Qwen 0.6B + Kokoro economy pair also completed the English round trip.

## Indian-language samples

Synthetic native-script samples exercised Hindi, Tamil, Telugu, Bangla,
Marathi, Malayalam, Kannada and Gujarati, plus Hinglish. Every admitted sample
produced a transcript and ordinary governed turn. This verifies the plumbing;
it is not a claim that every answer or pronunciation was correct.

- Hindi, Telugu, Bangla, Malayalam and Gujarati transcripts preserved the
  submitted sentence; Tamil punctuation/orthography varied slightly. Hinglish
  rendered the English words in Devanagari. Marathi's first synthesis hit the
  direct key's free-tier limit; the same sentence through OpenRouter completed
  and preserved the text.
- One Kannada Flash-Lite sample changed the intended four into 24 and dropped
  the opening word. A shorter Kannada sample retained four but mixed a Latin
  character into its script. With full Flash TTS, the original Kannada sentence
  was transcribed exactly and the Agent answered `೪`. The full model through
  OpenRouter also preserved that Kannada sentence and answered `4`. That is the reason to
  prefer full Flash for the initial broad-language setup rather than claiming
  the cheapest speaker is good enough for every language.
- These test phrases are not a controlled translation corpus. Reused synthetic
  conversations also yielded imperfect text-Agent answers (a list of digits in
  Marathi, a four-digit answer in Gujarati). Those are recorded as ordinary
  Agent behavior, not speech recognition successes. Native-speaker acceptance,
  human microphone input and natural code-switching remain open.

## Verification and limits

Validation completed: 101 config, 167 LLM and 22 voice library tests passed
(two unrelated LLM tests remain ignored); 18 server voice-focused tests, four
socket/channel integration tests and the selected-Agent setup/audition test
passed. Clippy with warnings denied passed for voice/config/LLM/core/server;
both frontend builds/typechecks and the Admin fixture typecheck passed.

The real Settings UI was inspected at 1440×900 and 390×844 in light and dark,
with no horizontal overflow and voice controls inside the viewport. The Admin
fixture renders the real Settings component, covering direct, routed and failed
discovery cases at both sizes/themes: 108 assertions passed, including atomic
setup, correct async selection, scoped preview, no writes on open, no implicit
microphone enablement and retry/exact-ID recovery. Saved screenshots and raw
synthetic reports are under `/tmp/vak-voice-review`. The running browser
speaking test returned playable audio; this does not establish what a person
heard in the room. Build output includes the existing macOS linker unwind-size
warning and frontend chunk-size notice; focused checks pass. Before committing, the complete `scripts/gate.sh` passed: workspace formatting,
clippy with warnings denied, all workspace tests, version, documentation paths,
records and advisories. This is not a released build.

Physical-microphone round trip, echo/barge-in behavior in a real room, a real
Telegram/Discord/Slack voice note, realtime streaming adapters, and rated speech
billing remain open in the tracker. The review workspace finishes with OpenRouter Google recognition/full Flash
speech and Kore selected, with microphone conversations off; preview works
while off. No release, installation replacement or change to the installed
service's saved voice defaults is claimed.
