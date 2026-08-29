# 38 — Voice & Personality (Gemini Live)

## Problem

Every bot/chat already has an inheritance chain for policy, permission
mode, and model route (`Workspace → Bot → Chat`, docs/design/34). There
was no equivalent for *how a bot sounds* — no per-bot spoken voice, no
persona/style override, and no way to hear the difference before
committing to it. Desktop had no audio capability at all.

## Design

One mechanism serves both consumers (chat-bot replies and desktop
narration) rather than two parallel integrations:

### `VoiceConfig` — a new tier alongside `route`/`permission_mode`

```rust
pub struct VoiceConfig {
    pub voice_name: Option<String>, // Gemini prebuilt voice, e.g. "Kore"
    pub persona: Option<String>,    // style directive -> systemInstruction
}
```

Lives as `voice: Option<VoiceConfig>` on both `Bot` and `AllowlistEntry`
(`crates/vak-server/src/gateway.rs`), resolved by
`GatewayState::resolve_voice` exactly the way `effective_route_override`
resolves `route`: the chat's own override wins; otherwise, if
`inherit_bot_policy` hasn't broken the chain, its bound bot's voice;
otherwise no voice (caller falls back to a built-in default, `"Kore"`).
No new "break inheritance" flag — `inherit_bot_policy` already governs
policy and route together, and voice rides the same switch, since all
three describe "this chat's relationship to its bot."

Wire format matches the established `route`/`permission_mode` idiom:
absent/`null` = inherit, an object = override, an explicit `null` on
PATCH clears back to inherit (`deserialize_present`,
`crates/vak-server/src/admin.rs`, `crates/vak-server/src/gateway.rs`).

### `crates/vak-llm/src/google_live.rs` — Live API session wrapper

A from-scratch `BidiGenerateContent` WebSocket client (Gemini's Live API
has no SSE/REST form), mirroring `google.rs`'s conventions: pure
`build_live_config` builder, `x-goog-api-key`-style auth (query-string
`key=`, per the wire protocol), the same status→`LlmError` mapping shape,
and `tokio::select!` cancellation racing the caller's
`CancellationToken`.

Two protections a cancellation token alone doesn't provide, since nothing
upstream ever triggers it on a stall:

- **`LIVE_SESSION_TIMEOUT` (25s)** wraps the whole call. Without it, a
  socket that stalls without erroring or closing blocks the request task
  forever — the concurrent-request version of that is a resource-exhaustion
  path on an authenticated endpoint, not just a UX gap.
- **`MAX_SPEAK_TEXT_CHARS` (2,000)** rejects oversized input before any
  network call. This is a paid, per-call external API; an unbounded
  `text` field lets any bearer-holder run up billing with one request.

Output is always 24kHz/16-bit/mono PCM, hand-wrapped into a 44-byte WAV
container (no extra crate needed for a canonical header) so nothing
downstream has to know the wire format.

Every call produces a `WorkReceipt` (`WorkPurpose::VoiceSynthesis`,
`crates/vak-llm/src/work.rs`) via the same `record`/`classify_error`/
`settle_cancelled` path text dispatches use — voice gets the same
auditability as every other provider call, per AGENTS.md's "every
provider dispatch produces a receipt."

### `POST /voice/speak`

One endpoint, three callers, behind the existing global `require_bearer`
middleware (no new auth code):

- **Gateway reply path** calls it in-process to attach a voice-note when
  a chat/bot resolves to a non-null voice.
- **Desktop narration** calls it over the same HTTP+bearer connection the
  webview already uses for everything else (`crates/vak-desktop/ui/src/
  api.ts`) — the desktop app needed no Gemini client of its own.
- **Admin console Preview button** calls it with an in-progress, unsaved
  config so an operator can hear a voice/persona before committing.

Resolution order: request's explicit `voice_override` > the named
chat's/bot's resolved voice > built-in default (`"Kore"`, no persona).

### Admin console (`crates/vak-admin-ui`)

`VoiceConfigEditor` follows the exact `ChannelPermissionPicker`/
`ChannelAccessEditor` idiom already established for route/permission-mode
overrides (`.inherit-toggle` checkbox revealing controls via `<Show>`,
busy-signal + toast + refetch on save, no optimistic updates) — it reads
as one more row in the same inheritance panel, not a bolted-on feature.
Slotted into both the per-bot editor and the per-chat
`ChannelAccessEditor`.

### Desktop (`crates/vak-desktop`)

No native audio crate — the webview plays a `Blob` returned from
`/voice/speak` through a single shared `<audio>` element, consistent with
the desktop app's existing shape (a thin Tauri shell over HTTP + webview,
not a place that owns domain logic). Narrates turn completion (a short
"Done."/"Task failed." cue, not the full transcript) and permission-gate
prompts (tool name + primary arg, not the full reasoning), gated by a
Settings toggle with a voice/persona picker — deliberately terse, so
narration never becomes the bottleneck on a verbose turn.

## Non-goals (this pass)

- Full-duplex live conversation (mic input) — this is one-shot
  text-in/audio-out per turn, not a phone call. `google_live.rs` opens
  and tears down a session per `speak()` call.
- A general per-bot/per-chat system-prompt override mechanism. `persona`
  here only ever feeds the Live API's `systemInstruction` for voice
  synthesis; it is not a substitute for `Core::system_prompt()`.
