# 49 — Live voice: full-duplex spoken conversation

Goal: replace the one-shot text-to-speech narration shipped in
docs/design/38-voice-personality.md with a real spoken conversation —
continuous mic, streaming transcription, spoken replies, barge-in — over
a provider abstraction that treats Gemini Live, OpenAI Realtime, and a
fully local stack (whisper.cpp + Piper/Kokoro) as peers, across desktop,
web, and Telegram.

Status: shipped foundation, redesigned 2026-09-23; realtime streaming and a
physical-microphone acceptance run remain open. The **Shipped contract**
section directly below is authoritative. The Architecture, Transport and
Phasing sections after it are the original proposal, kept for its reasoning;
where they disagree with the shipped contract (streaming `Listener` traits, a
`VoiceRegistry`, a silence-emitting `LocalSpeaker`, `{"t":"barge"}` frames, a
`[voice.local]` table, a shared `model` key), they describe something that was
never built or has been removed.

## Shipped contract

**One provider vocabulary.** `vak_voice::VoiceProvider`
(`crates/vak-voice/src/provider.rs`) is the closed set `gemini`, `openai`,
`local`, parsed in exactly one place. Every route resolves through it, so the
socket, `/voice/transcribe`, `/voice/speak`, `/voice/providers`, `/config`
and doctor cannot disagree about names. An unset provider is a configuration
gap reported as such, never an implicit vendor. Hosted routes require explicit
`transcription_model` / `synthesis_model` pins from the operator's discovered
catalogue (invariant 9); there is no shared `model` fallback and no realtime
model, because no realtime adapter exists.

**One server surface.** `crates/vak-server/src/voice.rs` holds every voice
route. Transcription has a single implementation used by the channel
bridges' batch route and the socket alike. Every paid call — batch
transcription, synthesis, each socket utterance — draws on one
per-process `RequestWindow` sized by `voice.max_requests_per_minute`.
`/voice/speak` records a `WorkReceipt` with real latency for every provider,
success or failure.

**The socket protocol** (`crates/vak-voice/src/protocol.rs`, version 1).
Binary frames are 16 kHz mono 16-bit PCM. Client and server controls are
separate types: a client sends only `speech_started`, `speech_stopped` and
`playback`; the server sends `ready` (with a required `protocol_version`),
`transcript`, `discarded` (`insufficient_speech`, `empty_transcript`,
`rate_limited`), `turn_completed` and `error`. A transcript is therefore
something only the server's governed route can produce — a client frame
claiming to be one fails to decode and closes the session. At most one
utterance is open; ids are never reused; audio outside an open utterance is
dropped; stopping voice mid-sentence sends no `speech_stopped`, so the
partial audio is discarded rather than submitted. A final transcript is
appended to the session ledger and started as an ordinary governed turn;
its answer returns as `turn_completed` with the same utterance id, and the
client's `playback` frame for that id is appended as a `voice_playback`
activity recording what was actually emitted.

**Speech evidence, measured by the server.** A fixed loudness threshold
cannot tell a person from a room: in a noisy room every frame clears it,
which is how the 2026-09-23 open-mic run dispatched unrelated turns.
`vak_voice::vad::SpeechEvidence` (`crates/vak-voice/src/vad.rs`) measures
each closed utterance against its own quiet floor (the 20th-percentile frame
level); a frame is voiced only when it stands 2.5× (about 8 dB) above that
floor and above an absolute minimum, and an utterance needs 240 ms of voiced
audio before it may cost a provider call. Flat noise of any loudness is
discarded with `insufficient_speech`. The browser detector
(`crates/vak-client-ui/src/voice-activity.ts`) applies the same constants
live for endpointing — calibrating its floor from the quietest moment of the
first 300 ms, adapting it while idle, learning the room's level from any
utterance that turned out to be noise, capping an utterance at 30 s, and
doubling the level and duration an onset needs while an answer is playing so
the answer's own echo does not interrupt it. A test pins the shared
constants. The client shapes latency only; the server decides.

**Local engines.** `VAK_LOCAL_TRANSCRIBER` names an executable that reads
encoded audio (WAV from the socket) on stdin and prints the transcript;
`VAK_LOCAL_TTS` names one invoked with the requested encoding (`wav`,
`pcm16`, `ogg_opus`, `mp3`) as its only argument, reading text on stdin and
writing that encoding. Both run with an empty environment and are resolved
through `vak_config::get_var`; an unconfigured engine fails closed.

**Doctor** reports voice as a failure with a remedy when it is enabled but
has no valid provider, no credential for it, or no model pin.

**Evidence.** `crates/vak-server/tests/voice_session.rs` drives a real
WebSocket upgrade against the real router with a scripted transcriber and
model: flat noise is discarded without reaching the transcriber, speech
becomes one Agent turn whose answer returns on the socket, playback is in
the ledger, a forged client transcript is refused and never logged, and a
cross-origin upgrade is refused.

**Open.** A human-heard, physical-microphone round trip in a quiet room;
per-bot/per-chat voice routes (`VoiceConfig.provider` and its model pins are
stored but not yet applied to dispatch — only `voice_name` and `persona`
are); realtime streaming adapters; retiring the separate one-shot narration
path in the client once spoken conversation covers it.

Credential handling is intentionally outside the voice protocol: provider
keys are resolved through vak's existing secret lookup chain and are never
included in control frames, session transcripts, provider descriptors, or
health/admin responses.

Related: 38-voice-personality (the narration path this replaces and the
`VoiceConfig` inheritance tier it keeps), 41-capability-registry (the
visibility failure class this reproduces and must close), 48-web-client
(the host seam and WebSocket precedent this reuses), 31-network-resilience
(the four-plane network contract this must respect), 22-gateway (the
Telegram media-I/O gap this closes), 01-llm (`Provider`, `ProviderAuth`,
`ProviderRegistry`, receipts — the abstraction voice was built outside of
and now joins).

---

## Problem (historical baseline)

The following describes the gaps that motivated this design. It is retained
to explain the implementation decisions; statements about the old narration
path and missing diagnostics describe the pre voice-foundation release and
must not be read as the current product status.

Voice shipped once (`1380497 feat: voice & personality via Gemini Live`)
and is present in the committed bundles, but it is invisible for six
independent reasons — and even fully repaired it would not be what "live
voice" means, because **there is no microphone code anywhere in the
repo** (`getUserMedia`, `MediaRecorder`, `AudioWorklet`, WebRTC, any STT:
zero hits). Doc 38 lists full-duplex as an explicit non-goal.

1. **It is narration, not conversation.** What ships is one-shot
   text→WAV that says `"Done."` / `"Task failed."`
   (`crates/vak-client-ui/src/store.ts`) and reads permission prompts
   aloud (`crates/vak-client-ui/src/components/ChatPane.tsx`). Two
   callers, both cues.
2. **Off by default, buried.** `voiceEnabled: false`
   (`store.ts:212`), in a group under Settings → General
   (`crates/vak-client-ui/src/components/Settings.tsx`).
3. **Silent failure without a Gemini key.** `/voice/speak` 400s
   (`crates/vak-server/src/lib.rs`); the client swallows it into
   `console.error` (`store.ts:364`). Flip the toggle, hear nothing, learn
   nothing.
4. **On desktop it is CSP-blocked even with a key.** `speak()` plays a
   `blob:` URL, but `crates/vak-desktop/tauri.conf.json` declares no
   `media-src`, so it falls back to `default-src 'self'`. `img-src` was
   given `blob:` explicitly; media was missed. There is also no
   `NSMicrophoneUsageDescription`, and no `ws:` in `connect-src`.
5. **Invisible to every diagnostic.** No `doctor` check, no `/providers`
   row, no capability entry. Doc 41 names this exact failure class —
   "the person who could repair the configuration saw nothing" — as the
   thing this project set out to kill. Voice reproduces it.
6. **The documented Telegram path was never built.** Doc 38 claims the
   gateway attaches a voice note when a chat resolves to a voice. The bridge downloads bounded voice notes, transcribes through the governed endpoint, and converts synthesized output to Telegram-compatible Ogg/Opus for `sendVoice`; failures preserve the text reply.

Structurally, the root cause of 3–6 was one thing: `google_live::speak()`
is a **free function called straight from an axum handler**, not an impl
of `Provider` (`crates/vak-llm/src/lib.rs`). It is absent from
`ProviderRegistry`, from the circuit breaker, from the route ladder, from
key management, and from `doctor`. The current implementation routes through
the configured provider, requires an explicit discovered model where the
provider needs one, and obtains voice identifiers from capability discovery.

---

## The design decision that shapes everything

**vak's agent loop stays the brain. Realtime providers are used as
streaming transcription and streaming synthesis, never as the
conversational agent itself.**

Gemini Live and OpenAI Realtime are conversational agents by default:
stream audio in, *they* decide the turn ended, *they* generate the
reply. Wiring that up directly would put a second, un-audited agent in
front of the user — no session entries, no permission engine, no
capability registry, no frozen route ladder, no receipts. That is a
direct breach of invariant 1 ("model-visible means logged") and of the
whole premise of doc 41. A voice turn must be the *same* turn a typed
prompt produces, with the same receipts.

Both providers make this practical rather than merely correct in
principle: each has a transcription-only mode — OpenAI's Realtime
session opened with `?intent=transcription`, Gemini Live's
`inputAudioTranscription` — that gives streaming ASR with server-side
VAD and no model turn. That is the mode this design uses.
`crates/vak-agent` remains the only thing that answers.

The latency cost is small for this product specifically: vak is a coding
agent whose turns take seconds of real tool work, so the agent loop, not
the speech round-trip, is the long pole — but it is not zero. A real
agent turn can run tool calls for 20–30 seconds, and a voice UI that goes
silent for all of it reads as broken in a way text never does. **Spoken
progress acknowledgement** ("Looking at the test suite…") off tool-start
events, not just an outcome sentence at the end, is load-bearing here,
not polish — see client design below.

The payoff of keeping the agent loop as the only brain: one trait covers
hosted and local providers identically, and local stops being a
second-class citizen. Provider-as-brain (bridging vak's tools through a
provider's own function-calling, every call still routed back through
the permission broker) stays a legitimate future extension; the trait
below is shaped so it can be added as a third implementation rather than
a rewrite.

---

## Architecture

### 1. New crate: `crates/vak-voice`

Add to `Cargo.toml` members and `[workspace.dependencies]`. Modelled on
`vak-delivery`: deliberately thin dependencies (`vak-llm` for
`LlmError`/`WorkReceipt`/`WorkPurpose`, `tokio`, `tokio-tungstenite`,
`futures`, `serde`), **no `vak-server`, no `vak-core`**. It owns the
audio primitives that have no business in `vak-llm`'s text-streaming
vocabulary.

```
crates/vak-voice/src/
  lib.rs        VoiceError, VoiceRegistry, VoiceDescriptor, default_registry()
  audio.rs      PCM framing, i16<->f32, resampling, WAV wrap/parse
                (absorbs wrap_wav from google_live.rs)
  vad.rs        Energy + hangover endpointing for client-side turn-taking
  listen.rs     Listener / ListenStream / ListenEvent — streaming ASR
  speak.rs      Speaker / SpeakStream / synthesize_all() — streaming TTS
  transcribe.rs Transcriber — batch, container-in (Telegram voice notes)
  gemini.rs     Gemini Live, transcription-only mode (absorbs google_live.rs)
  openai.rs     OpenAI Realtime transcription session + /v1/audio/speech
```

`local` is not a fourth implementation module: it is the same
OpenAI-compatible client (`openai.rs`) pointed at a different
`base_url`, exactly the way `ollama` is just `openai`-shaped with a
different default endpoint in `crates/vak-llm/src/registry.rs`. Because
`ProviderAuth` already carries `base_url`, "local" needs no special
case — `whisper.cpp --server` speaks the same
`/v1/audio/transcriptions` shape OpenAI does, and Piper/Kokoro speak the
same `/v1/audio/speech` shape.

`crates/vak-llm/src/google_live.rs` **moves** into `crates/vak-voice`
rather than being duplicated — it has exactly one caller today
(`voice_speak`), so this is cheap, and it removes the free-function
bypass around the provider abstraction described in the Problem section.
`pub mod google_live;` comes out of `crates/vak-llm/src/lib.rs`, and the
AGENTS.md crate map is updated in the same commit.

### 2. The trait set

Three traits, because there are genuinely three shapes and collapsing
them costs a fake shim in one direction or the other: streaming ASR,
batch ASR (a Telegram voice note is a *file*, not a stream), and
streaming TTS. A provider that is duplex (Gemini Live, OpenAI Realtime)
is one struct implementing two of these traits; that is an
implementation detail, not a fourth shape.

```rust
/// What a provider says about itself, so no surface hardcodes a
/// catalogue (invariant 9) — this is what deletes both the hardcoded
/// Gemini model string and the hardcoded Settings.tsx voice list.
pub struct VoiceDescriptor {
    pub name: &'static str,
    pub env_var: Option<&'static str>,
    pub default_base_url: Option<&'static str>,
    pub endpointing: Endpointing,     // Server | Client
    pub formats: &'static [SpeakFormat],  // Pcm16 | OggOpus | Mp3
    pub voices: &'static [&'static str],  // empty ⇒ discovered at runtime
}

#[async_trait::async_trait]
pub trait Listener: Send + Sync {
    fn name(&self) -> &str;
    fn endpointing(&self) -> Endpointing;
    fn circuit_key(&self) -> String { self.name().into() } // mirrors Provider::circuit_key
    async fn open(&self, spec: ListenSpec, cancel: CancellationToken)
        -> Result<Box<dyn ListenStream>, VoiceError>;
}

#[async_trait::async_trait]
pub trait ListenStream: Send {
    async fn push(&mut self, frame: &[i16]) -> Result<(), VoiceError>;
    /// Client-endpointed engines only: "the utterance ended".
    async fn flush(&mut self) -> Result<(), VoiceError>;
    async fn next(&mut self) -> Option<ListenEvent>;
    /// Consumes the stream; returns the transcript accumulated so far so a
    /// cancelled listen still yields what the user said (invariant 5).
    async fn close(self: Box<Self>) -> Option<String>;
}

pub enum ListenEvent {
    SpeechStarted,
    Partial { delta: String, text: String },  // delta AND snapshot (invariant 4)
    Final { text: String, confidence: Option<f32> },
    SpeechStopped,
    Failed(VoiceError),
}

#[async_trait::async_trait]
pub trait Speaker: Send + Sync {
    fn name(&self) -> &str;
    fn output_rate_hz(&self) -> u32;
    fn supports(&self, format: SpeakFormat) -> bool;
    async fn speak(&self, spec: SpeakSpec, cancel: CancellationToken)
        -> Result<Box<dyn SpeakStream>, VoiceError>;
}

#[async_trait::async_trait]
pub trait SpeakStream: Send {
    async fn next(&mut self) -> Option<Result<Vec<u8>, VoiceError>>;
    /// What has been emitted so far. On barge-in this is what the user
    /// actually heard, and it is what the ledger records.
    fn emitted_ms(&self) -> u64;
}

#[async_trait::async_trait]
pub trait Transcriber: Send + Sync {
    fn name(&self) -> &str;
    /// A whole recording, container included. We do not decode Opus
    /// in-tree: every OpenAI-compatible /v1/audio/transcriptions endpoint
    /// and Gemini both take the container directly.
    async fn transcribe(&self, audio: AudioBlob, spec: ListenSpec, cancel: &CancellationToken)
        -> Result<String, VoiceError>;
}
```

`VoiceRegistry` mirrors `vak_llm::registry::ProviderRegistry` exactly:
name→factory map, `ProviderAuth`-keyed cache where a different key or
base URL is a different provider instance.

**Cancellation (invariant 5).** `CancellationToken` threads through
`open`/`speak`/`transcribe`, raced in `tokio::select!` exactly as
`google_live.rs` already does. Two partial-output contracts follow: a
cancelled listen returns the transcript heard so far, never silently
nothing; a cancelled speak (barge-in) reports `emitted_ms()`, which is
recorded as the truth about what the user actually heard — the
alternative would let the ledger claim the agent said something the
user never heard.

**Receipts.** `WorkPurpose` gains `SpeechRecognition` (additive;
`VoiceSynthesis` is unchanged). Every `Listener::open`,
`Transcriber::transcribe`, and `Speaker::speak` produces a `WorkReceipt`
through the same `record`/`classify_error`/`settle_cancelled` path
`voice_speak` already uses. A duplex socket produces one receipt per
utterance, not one per socket connection.

### 3. Transport: `WS /voice/session`

Copy the shape of `pty_socket` (`crates/vak-server/src/web.rs`) — it
is the established precedent for a bidirectional socket on this server,
including its refusal gate.

- **Frames.** Binary = raw mono i16 PCM, 20 ms frames. Text = JSON
  control (`{"t":"transcript",...}`, `{"t":"barge"}`,
  `{"t":"error","message","remedy"}`, …). Nothing a caller can say is
  mistakable for a control message, mirroring `PtyControl`.
- **Rates.** The client's capture worklet is the only place the true
  device sample rate is known, so it opens the capture context at 16 kHz
  directly (Chrome/Firefox do the anti-aliased conversion in the audio
  thread) and downsamples in software only where the platform won't
  honour that. The server declares its output rate in the session's
  `ready` frame and never resamples; the client's playback graph
  resamples for free during output.
- **Auth at upgrade** reuses the existing split: same-origin cookie on
  web, `?token=` on desktop.
- **Refusal gate.** A `voice_refusal(&state, &headers)` mirroring
  `terminal_refusal`: off unless `[voice] enabled`. Deliberately **not**
  loopback-pinned the way the terminal is — the terminal's pin exists
  because a shell is the one effect not mediated by the permission
  engine, and a voice session only ever produces a *prompt*, which goes
  through the ordinary permission engine, hooks, broker and sandbox.
  What the socket does need, because it is an authenticated endpoint
  spending a paid per-minute API, is a concurrency cap
  (`max_concurrent`) and a wall-clock ceiling per session
  (`max_session_secs`) — the same reasoning `MAX_SPEAK_TEXT_CHARS`
  already documents for `/voice/speak`, generalized from a character cap
  to a session cap.
- **A live security gap to close in the same change.**
  `crates/vak-server/src/lib.rs` computes `mutating` — whether the
  `Origin` header is checked — from the HTTP method, and a WebSocket
  upgrade is a GET, so `/pty` skips the Origin check entirely today. A
  `/voice/session` upgrade opens a mic relay that spends provider money;
  that is state-changing in every sense that matters even though it is a
  GET. `voice_refusal` must do its own `Origin` check, and the same fix
  belongs on `/pty` in the same change — shipping one checked WebSocket
  route and one unchecked one would be a worse inconsistency than either
  alone.
- **Why relay through the server at all**, rather than letting the
  browser talk to Gemini/OpenAI directly with an ephemeral client token:
  the API key must never reach a webview (invariant 8), and the relay is
  where receipts, FinOps admission, and the session ledger attach. A
  browser-direct dispatch produces no `WorkReceipt` and no ledger entry.

### 4. Turn integration

A final transcript is fed to the **existing** `sendPrompt` path — a
voice turn is an ordinary turn with an ordinary session entry, marked
with a `source: "voice"` field. The agent's streamed reply is chunked
into `Speaker::speak()` at sentence boundaries so playback starts before
the turn finishes. Speech detected during playback triggers an
interrupt plus a playback-buffer flush; a mid-run barge-in also invokes
the existing `cancel_run`.

The modality gate is inert today only for want of one branch.
`Modality::Audio` exists (`crates/vak-intent/src/axes.rs`),
`leg_supports_modalities` already gates on it, and
`crates/vak-intent/src/signals.rs` already folds any attachment's
modality into `Reading::input_modalities` generically — but
`crates/vak-core/src/lib.rs` is the only place an `Attachment` is
ever constructed, and it hardcodes `Modality::Image`. Adding the audio
branch there brings the whole gate alive with no further plumbing.

### 5. Client (`crates/vak-client-ui`, SolidJS)

**Ship the browser first, desktop second — not the reverse.** The
Tauri webview is the one host where microphone access is genuinely
unproven: wry 0.55.1's WKWebView delegate unconditionally grants
`getUserMedia` (`wry_web_view_ui_delegate.rs:126`), so the webview layer
is not the risk, but `getUserMedia` also requires a secure context and
the desktop origin is `tauri://localhost` — a custom scheme, not
`http://localhost` — and whether WebKit treats that as
potentially-trustworthy cannot be settled by reading code. That is a
half-day spike that must run before any desktop voice work, not
discovered partway through it.

**Browser (Phase 3):**
- New `HostFeature: "microphone"` in `host/port.ts`, plus a
  `VoiceTransport` interface modelled on the existing
  `TerminalTransport`. A host that cannot capture renders differently,
  not brokenly — the rule the file already enforces.
- **Capture and playback are `AudioWorklet` modules shipped as real
  files** (not inline/blob) — required so the desktop CSP's `script-src`
  need not be widened to `blob:`. Capture downsamples to 16 kHz and
  frames to 20 ms; playback is a ring buffer fed from WS binary frames,
  explicitly **not** `<audio>` + object URL — that path is what CSP
  blocks today, cannot stream, and cannot be flushed instantly for
  barge-in. Echo cancellation (`echoCancellation: true` on capture) is
  mandatory, not optional — without it the mic hears the agent's own
  TTS through the speakers and false-triggers barge-in.
- **Control** in the `composer-actions` div
  (`components/Composer.tsx:537`), beside send. Interim transcript
  renders live so the user can see what it heard; the final transcript
  submits through the existing `sendPrompt`.
- **Spoken progress, not just spoken outcome** — per the design
  decision above, a short cue on tool-start events, not only on
  `RunFinished`.
- **Settings**: promote voice from a group under General to its own
  page, built against a `GET /voice/providers` status endpoint —
  provider, voice, model selects populated by discovery, never a
  hardcoded list — and a **Test voice** round-trip button (the admin
  console's `VoiceConfigEditor` Preview button is the existing idiom to
  copy).
- **Errors become toasts**, never `console.error`.

**Desktop (Phase 4, after the secure-context spike):**
- `crates/vak-desktop/tauri.conf.json` CSP: add `media-src 'self' blob:
  data:` and explicit `ws://127.0.0.1:* ws://localhost:*` in
  `connect-src` (CSP3 scheme-matching technically already permits `ws`
  wherever `http` is listed, but that subtlety is exactly the kind of
  thing a future cleanup pass "fixes" by removing — write it explicitly).
- Add `NSMicrophoneUsageDescription` for macOS.
- If the secure-context spike fails, `can("microphone")` is `false` on
  desktop and Settings states in one sentence that spoken conversation
  needs a browser tab, with the loopback URL to open. An absent
  capability with an explanation is a design decision; a dead mic button
  is not.
- Delete the old narration path (`store.ts:334-366`,
  `registerVoiceAudioElement`, the hidden `<audio>` in `App.tsx:1123`).

### 6. Gateway / Telegram

- **Inbound**: parse `voice`/`audio` updates in
  `crates/vak-server/src/surfaces/telegram.rs` (~line 279, alongside the
  existing photo/document branch), download, and transcribe via
  `Transcriber` before `compose_prompt` — no content-block shape accepts
  raw Opus, so this must resolve to text first. Push the container bytes
  straight to the ASR provider (no Opus decoder enters the tree; every
  `/v1/audio/transcriptions`-shaped endpoint and Gemini both accept the
  container). Degrade with a visible note
  (`[voice note, 0:07 — could not be transcribed: ...]`) on failure,
  never drop the message silently.
- **Fix while in here**: the inbound loop `continue`s on unroutable
  updates without advancing the update offset, so a trailing
  voice/sticker message is re-fetched every poll until a newer routable
  message arrives — contradicting the comment above it.
- **Outbound**: add an audio field to `GatewayReply` (line 224) and call
  `sendVoice` when the resolved `VoiceConfig`
  (`resolve_voice`, `crates/vak-server/src/gateway.rs`) names a
  voice and the chosen `Speaker` supports a Telegram-compatible format.
  This is the point at which doc 38's Telegram claim stops being false.
- **Codec.** `sendVoice` requires OGG/Opus. Rather than adding an Opus
  encoder dependency, use the OpenAI-compatible `/v1/audio/speech`
  endpoint's native `response_format: "opus"` (available to `openai` and
  any local Piper/Kokoro server speaking that shape); a `Speaker` that
  cannot produce Opus returns `VoiceError::UnsupportedFormat` and the
  gateway degrades to text-only with a diagnostic, never silently. This
  is the one dependency decision in the whole plan and it resolves to
  "add no dependency."
- **Renderer**: `media.audio` is already an allowed delivery block type
  (`crates/vak-delivery/src/skills.rs`) backed by `MediaOutput` with
  `duration_ms`, but `PresentationRenderer.tsx` (~line 271) has no case
  for it. Add one.

### 7. Visibility

- A `voice` check in `crates/vak-core/src/health.rs`, using the
  `HealthCheck { label, detail: Result<String,String> }` shape. Honours
  commit `9a65ec1`: a deliberately disabled voice stack reports
  `Ok("disabled")`, never `Err`; an enabled-but-unconfigured stack
  reports an `Err` naming the missing env var, not a bare failure.
- `GET /voice/providers` — its own endpoint, not a row folded into
  `/providers`: voice has three independent slots (listen, transcribe,
  speak) where `/providers` reports one route. Lists each registered
  provider, whether its credential resolves, and its discovered
  models/voices.
- **Discovery closes both live invariant-9 violations at once.** Google's
  `ListModels` already returns `supportedGenerationMethods`; filtering on
  `bidiGenerateContent` is real discovery, not a table, and the same
  `VoiceDescriptor.voices` mechanism that provides this also deletes the
  hardcoded `Settings.tsx` voice `<select>`.

### 8. Config

The `[voice]` section (`VoiceSettings`) is shipped alongside the
per-bot/per-chat `VoiceConfig`. Its implemented shape follows
`ServerSettings`/`WebSettings`/`ServerResolved`
(`crates/vak-config/src/lib.rs`) field-for-field in style —
`WebSettings::terminal` is the closest analogue to `[voice] enabled` in
both shape and risk.

```toml
[voice]
enabled                 = false
provider                = "openai"      # gemini | openai | local
transcription_model     = "…"           # from the key's discovered catalogue
synthesis_model         = "…"           # from the key's discovered catalogue
max_concurrent          = 2             # a paid per-minute API on an authenticated endpoint
max_session_secs        = 900           # the LIVE_SESSION_TIMEOUT lesson, generalized
max_audio_bytes         = 16777216      # inbound audio per socket session
max_requests_per_minute = 60            # every paid voice call, shared
max_text_chars          = 100000        # synthesis input per request
```

`[voice]` is privileged and is reset in `load_with_trust` next to the
existing `fc.server = ServerSettings::default()` line
(`crates/vak-config/src/lib.rs`) and named in
`PRIVILEGED_KEYS_NOTICE` (line 2234) — the same reasoning already
written there for `web.terminal` applies verbatim: a cloned repository
must not be able to open a microphone on the machine that cloned it.
`voice` and `intent` are registered in `KNOWN_TOP_KEYS`; valid settings no
longer produce false unknown-key warnings.

Keys are resolved through the configured credential boundary before a remote
adapter is called. The handler only reads the canonical project/shared
credential lookup (`vak_config::get_var`) after selecting a non-local route;
local synthesis and capture never require a cloud key, and keys are never
returned by discovery or admin APIs.

### 9. Migration

- **Keep** `POST /voice/speak` and the `VoiceConfig` wire format (the
  admin console and shipped configs depend on both) but reimplement the
  handler over `Speaker`. Extend `VoiceConfig` with optional `provider`
  and `model`; absent stays "inherit", matching the existing tri-state
  idiom.
- **Delete** the `<audio>` + blob narration path
  (`store.ts:334-366`, `App.tsx:1123-1127`) in favour of the streaming
  player. The `"Done."`/`"Task failed."` cues survive as an option on
  the new stack; the permission-prompt narration
  (`ChatPane.tsx:233`) becomes a spoken turn inside a live voice session
  rather than a separate mechanism.
- **Correct** doc 38 (full-duplex is no longer a non-goal; give it a
  `Status:` line — it has none today, which is how its false Telegram
  claim survived unnoticed) and doc 22 line 57 ("No media I/O
  (images/voice/TTS) yet"). Add voice to README and CHANGELOG — it
  shipped undocumented the first time.

---

## Verification

- `cargo test -p vak-voice` — unit tests on the pure pieces, following
  the repo's in-module convention (`build_live_config`'s existing tests
  move here): resampler round-trip and anti-aliasing, i16/f32
  conversion, WAV header bytes, frame codec round-trip, VAD endpointing
  against a deterministic frame sequence.
- `crates/vak-server/src/web.rs` — exercise `/voice/session`
  end-to-end with a scripted fake provider, against the real secured
  router (the refusal and Origin checks are middleware properties and
  must be tested as real upgrade requests, not unit-called). Pin:
  refusal when `[voice] enabled = false`, naming the setting; a
  cross-origin upgrade is refused; a final transcript creates a real
  session entry; barge-in flushes and records `emitted_ms`; cancel still
  yields a receipt; concurrent sessions are capped; a session past
  `max_session_secs` is closed with a reason.
- `cargo test -p vak-core` — the new `voice` doctor check reports
  `Ok("disabled")` when off and an `Err` with a remedy when enabled
  without a resolvable credential.
- `crates/vak-server/tests/telegram_bridge.rs` — a voice update reaches
  a transcript reaches `/gateway/inbound`; `sendVoice` fires only when
  the speaker supports Opus, and text-only degrades visibly when it
  doesn't.
- `vak doctor` — voice appears as a row in all three states.
- Manual, local-only first (no key needed): run `whisper.cpp --server`
  and Piper, set `provider = "local"`, hold the mic control in the web
  client, confirm transcript → turn → spoken reply → barge-in.
- Manual, desktop: after the secure-context spike passes, `npm run
  build:tauri` and confirm capture/playback under the new CSP and that
  macOS prompts for microphone access.
- Manual, Telegram: send a voice note to a bot, get a spoken reply back.

## Phasing

Each phase is independently shippable and leaves nothing worse than
today.

1. **Foundation.** `vak-voice` crate: traits, `audio.rs`, `vad.rs`,
   `VoiceRegistry`, `google_live.rs` absorbed in. `[voice]` config,
   privileged and warned-on. `/voice/speak` reimplemented over `Speaker`
   with a byte-identical wire contract. `WorkPurpose::SpeechRecognition`.
   No surfaces yet; fully unit-tested.
2. **The server can hear.** `Transcriber` + the OpenAI-compatible client
   (covers OpenAI, Groq, and local whisper.cpp through `base_url`);
   `GET /voice/providers`; the doctor check; capability-filtered model
   discovery closing both invariant-9 violations.
3. **The socket.** `WS /voice/session`, the refusal gate, the
   Origin-at-upgrade fix (voice *and* `/pty`), session cap and
   wall-clock ceiling, `Listener` for OpenAI Realtime transcription-mode
   and Gemini Live, streaming `Speaker`, barge-in with `emitted_ms`.
4. **The browser hears and speaks.** `HostFeature "microphone"`, the
   worklets, the composer control, spoken progress acknowledgement,
   Settings rebuilt on `/voice/providers`, "Test voice", error toasts.
5. **Desktop**, gated on the secure-context spike running first. CSP and
   `Info.plist` fixes, `host/tauri.ts` transport, narration path deleted.
6. **Channels.** Telegram voice in/out via the Opus-native TTS format
   (no new dependency), the offset-advance fix, the `media.audio`
   renderer, doc 22 corrected.
7. **Local, and the tail.** whisper.cpp + Piper/Kokoro documented as
   first-class `base_url` targets in `docs/hosting.md`; admin console's
   `VoiceConfigEditor` gains listener/speaker selects; a fully offline
   voice conversation proven end-to-end on a box with no provider key at
   all.
