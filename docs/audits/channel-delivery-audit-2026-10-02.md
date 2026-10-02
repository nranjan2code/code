# Channel delivery and formatting audit — 2026-10-02

Status: audit complete; implementation repairs are in progress on `main`.

The pre-fix probe output below is a baseline, not a current result. This repair
pass changes production behavior for C01-C03, C05-C12, C15-C17, and the
immediate/proactive card duplication and provider-result paths. It does not
close durable reply replay (C04), complete Slack pagination/cold-start recovery
(C13), per-chunk outbox receipts/pacing (C14), or the broader feature-parity
items in C18. The remaining transport and live-client acceptance work is listed
at the end; do not treat the baseline probe output as validation of the new
code.

The channel output problems are real implementation defects, not just weak model
formatting. The most serious failures remove answer content, lose replies, repeat
work, or report rejected messages as delivered. Native rendering also lags the
capabilities of the providers.

## Scope and evidence

Reviewed the current working tree, initially at
`8cd42679dcb7e193f38af2c7b37f98efef96f1bf`. Other work was already changing this
checkout; this audit did not modify those files. Inspected the gateway final-answer
projection, delivery renderer, structured cards, all three bridges, proactive
adapters, outbox/replay, files, voice, threading, and existing tests. Read the
status/contract sections of design docs 30-output-engineering, 30-render-architecture,
31-network-resilience, 34-channel-onboarding, 57-adaptive-presentation-runtime,
67-presentation-renderer-guide, 64-agent-owned-platform, 15-reliability and
24-agent-security. Provider behavior was checked against official documentation.

Evidence labels below distinguish **executed** local probes from **source/API**
findings. No message was sent to a real chat, no credentials were read, and no
live-provider or client screenshot acceptance is claimed.

Reproduction source and output are in
[`../research/channel-delivery-audit-2026-10-02/`](../research/channel-delivery-audit-2026-10-02/).
Run `python3 docs/research/channel-delivery-audit-2026-10-02/run-probes.py` to
reproduce the pre-fix baseline. The script builds the actual delivery library
and links probes against it; it does not reimplement the renderers. It
deliberately exits nonzero for unmet expectations: **13 probes, 12 baseline
failures, one passing basic-formatting control**. It has not been rerun against
the repair pass.

Before the repair pass, `cargo test -p vak-delivery --tests` passed: **163 tests**,
including the tests containing the large audit sweeps. Its success does not
establish correct channel output: see the verification section below. The
evidence folder includes its result summary and hashes of inspected source files.

## Actual delivery paths

Immediate replies take this path:

`session → projection::text_with_run_cards + run_cards → gateway → render_response → bridge POST`

Proactive notifications take a different path:

`DeliveryContent → delivery::deliver → Outbox → render → adapter POST → mark_delivered`

`render_response` only renders and attaches cards; it does not enqueue a reply.
Slack/Discord bridges have native-card branches, but their proactive adapters do
not. Telegram's production reply path uses the rendered text chunks; the separately
tested `structured_card_chunks` helper is not invoked by `send_message`.
Consequently, a successful helper test does not prove the actual channel message
contains the card's facts.

## Findings, in repair priority order

### C01 — P1: structured answers lose their facts before reaching the channel

**Executed + source.** `crates/vak-delivery/src/skills.rs:1418` builds readable
summaries for only a few types; unhandled types receive a heading plus a JSON
appendix. `crates/vak-server/src/projection.rs:132` removes that appendix for
channel delivery. A validated `data.grid` with an APAC row therefore becomes
only `### data.grid`. Research sources/takeaways, recipe ingredients, and other
nested payloads have the same structural problem when they hit the default arm.
Telegram sends this incomplete text. Slack/Discord card projectors also discard
arrays and objects, and their fallback uses the same incomplete summary.

This breaks the complete-answer contract even though full data still exists in
the ledger/packet. Fix the semantic-to-readable projection for all supported
shapes, preferably through the existing shared primitive lowering. Never remove
the only representation of a payload field until a readable replacement exists.

The inline `vak`-fence path has the opposite presentation problem:
`project_structured_fences_with` inserts `structured_markdown` including its JSON
appendix into ordinary channel text. The same semantic answer can therefore be
a title-only tool card or a verbose technical JSON block depending on how it was
emitted. A single complete readable projection should serve both entry paths.

### C02 — P1: Slack's native-card branch hides the complete first text chunk

**Executed + source/API.** `crates/vak-server/src/surfaces/slack.rs:321` posts
`text: chunks[0]` alongside `blocks`, then sends only subsequent text chunks.
Slack uses top-level text as a notification/accessibility fallback when blocks
are present; it is not an additional visible message body.
[`chat.postMessage`](https://docs.slack.dev/reference/methods/chat.postMessage/)

`crates/vak-delivery/src/slack.rs:405` selects only scalar fields. A validated
timeline with `summary: Day plan` and an item `Museum: Arrive at noon` produces
blocks containing the summary but no Museum. The complete timeline can exist in
`chunks[0]` yet be hidden in the ordinary Slack view. Supplemental notes and
completion/evidence warnings carried only in that chunk can also disappear.
The 10-card/50-block caps and truncation compound the problem without diagnostics.

Build visible blocks that cover the entire answer, with explicit continuation
messages for overflow. Do not use notification text as the sole preservation
mechanism for facts omitted from blocks. Discord has a different issue: it shows
both content and embeds, so scalar cards can be duplicated visually.

### C03 — P1: Slack/Discord chunking does not enforce its own limit

**Executed.** `crates/vak-delivery/src/lib.rs:1375` flushes a previous chunk but
appends the entire next line even if that line exceeds the cap. A 5,000-character
Discord paragraph yields one 5,000-character chunk with a 1,900 cap. A Slack
5,000-character code line yields chunk lengths **8, 5009, 7** with a 3,900 cap.
Discord's message content limit is 2,000 characters.
[`Message API`](https://docs.discord.com/developers/resources/message)

Hard-split oversized runs with syntax overhead reserved, avoid empty code-only
chunks, and validate the final serialized message. Slack's configured cap is not
its absolute text limit; the probe demonstrates a contract violation there, not
that every 5,009-character Slack message is rejected.

### C04 — P1: failed Slack/Discord replies are discarded from the delivery path

**Source.** Slack `surfaces/slack.rs:160` and Discord
`surfaces/discord.rs:166` advance cursors before processing. Their send failures
only call `eprintln!`, after which the tick returns success. A 429, outage, invalid
rich payload, or partial multi-chunk send leaves the answer missing or incomplete
in the chat with no outgoing reply job to replay. Session persistence is not a
substitute for delivery recovery.

Persist the finished reply and its audience before acknowledging input. Retry
delivery of that reply without running the model again, with a bounded fallback
for definitively rejected rich payloads.

### C05 — P1: Telegram send failure can re-run completed model/tool work

**Source.** `surfaces/telegram.rs:393` processes the update, then propagates a
send failure. `tick` returns no updated offset; `run` retains its old offset.
Retry therefore processes the upstream update again, including earlier updates
from the same batch. The bridge does not use the upstream update ID with
`InboundRequest::with_request_id`; its request construction at line 495 leaves
the gateway to mint a fresh admission ID.

Bind admission to a stable bot/update identity and retain a delivery receipt
separate from execution. Simply advancing the offset early would replace repeat
execution with reply loss and would not solve the problem.

### C06 — P1: Telegram's ownership probe deletes pending backlog

**Source/API.** `surfaces/telegram.rs:905` calls `getUpdates` with `offset=-1`
and describes it as non-acknowledging. Telegram specifies that negative offsets
retrieve from the queue's end and forget preceding updates. Startup invokes this
probe at line 963; takeover also invokes it. If multiple messages accumulated
during downtime, older pending messages can be lost before normal polling begins.
[`getUpdates`](https://core.telegram.org/bots/api#getupdates)

Use a non-destructive ownership strategy. Add a provider-faithful mock that models
negative-offset queue deletion; a mock that simply returns the last update cannot
catch this bug.

### C07 — P1: proactive Slack delivery records API rejection as success

**Source/API.** `delivery.rs:1030` sends Slack through `post_chunks`; line 1080
checks only HTTP status. `deliver_record` marks the job delivered at line 306.
An HTTP 200 body such as `{"ok":false,"error":"channel_not_found"}` is therefore
recorded as successful and removed from pending replay. The immediate Slack
bridge does check `ok`, so the behavior differs by entry path.
[`chat.postMessage` errors](https://docs.slack.dev/reference/methods/chat.postMessage/)

Parse the provider result and retain its message receipt before marking delivery
successful. Apply the same result contract to text, rich messages and uploads.

### C08 — P1: Telegram HTML conversion changes link destinations and breaks entities

**Executed.** `telegram.rs:105` uppercases already-rendered HTML for H1/H2.
`# [Docs](https://example.com/CaseSensitive)` becomes
`<b><A HREF="HTTPS://EXAMPLE.COM/CASESENSITIVE">DOCS</A></b>`.
It changes a case-sensitive path, tag spelling, code text and any entities inside
that heading. Even if a client tolerated tag case, the URL is no longer exact.

`telegram.rs:478` splits text tokens character-by-character, including HTML
entities. At cap 32, 31 `a` characters followed by `&amp;z` become a first chunk
ending in `&` and a second beginning `amp;z`. The same boundary defect exists at
the production cap. This can trigger HTML rejection or show corrupted text.
Preserve content and destinations verbatim; split parsed text/entity runs, then
serialize each valid message.

### C09 — P2: Telegram code and its plain fallback expose escaped markup

**Executed.** Inline text is escaped before code-span extraction and escaped
again when code placeholders are restored (`telegram.rs:301`). Input
`` `a < b & c` `` produces `<code>a &amp;lt; b &amp;amp; c</code>`.
`strip_html` at line 568 removes tags but does not decode entities: `A &amp; B`
remains escaped in a plain retry. Removing anchor tags also loses destinations
when the label itself is not a URL.

Escape once during serialization and derive plain fallback from the semantic
source, retaining explicit link destinations where needed.

### C10 — P2: Discord formatting regresses native Markdown and mutates code

**Executed + API.** `discord.rs:85` turns `# Title` into literal HTML
`<b>Title</b>`. `replace_links` expands masked links and is applied inside inline
code: `` `[x](https://example.com)` `` becomes
`` `x (<https://example.com>)` ``. The comment claiming plain messages cannot
render masked links is obsolete: Discord documents both native headings and
masked links. [`Discord formatting`](https://support.discord.com/hc/en-us/articles/210298617-Markdown-Text-101-Chat-Formatting-Bold-Italic-Underline)

Preserve supported Markdown and code nodes. Convert only unsupported structures.
The current unit test actually asserts that a Discord heading contains `<b>`.

### C11 — P2: Slack H2–H6 headings remain raw Markdown

**Executed.** `slack.rs:73` recognizes only a single `#` immediately followed
by a space. `## Summary` remains `## Summary`, and semantic card fallbacks use
`###` headings. Parse heading level rather than a single prefix, and emit valid
Slack formatting for all heading levels.

### C12 — P1: renderer failure discards an otherwise completed Slack/Discord answer

**Source.** The gateway intentionally returns successful final text alongside
`delivery_error` when rendering fails (`gateway.rs:3190`). Slack at
`surfaces/slack.rs:282` and Discord at `surfaces/discord.rs:314` replace a missing
or incompatible packet with `(delivery failed: …)` and ignore that text.
Telegram preserves text in this case but sends it without length-aware splitting.

Use safely escaped, bounded original text as the recovery path. A rendering
failure must not replace a valid answer with a transport diagnostic.

### C13 — P1: Slack polling loses bursts; both polling bridges can ignore the first message

**Source/API.** Slack fetches one page of 25 history entries at
`surfaces/slack.rs:185`, ignores `has_more`/`next_cursor`, and advances to the
newest returned human timestamp. Slack returns newest entries first, so older
entries beyond that page are permanently skipped. Bot filtering can reduce the
useful batch further. [`Slack history pagination`](https://docs.slack.dev/reference/methods/conversations.history/)

Both Slack and Discord initialize cursors only from a routable human message.
An empty channel, or a cold-start page containing only a bot message, leaves no
cursor. The next real user message is treated as cold-start history and skipped.
Their cursors also live only in memory. Establish the poll watermark independently
of whether any row is routable, and drain history pages before advancing it.
Discord burst ordering requires a separate provider-faithful test; this audit
does not assume its `after` pagination behaves identically to Slack's.

### C14 — P2: outbox replay repeats accepted chunks and lacks provider-aware pacing

**Source.** `delivery.rs:290`, `:923`, `:1065` and
`vak-delivery/src/outbox.rs:12` retain job-level success/failure, not individual
provider message receipts. If chunk 1 succeeds and chunk 2 fails, replay sends
chunk 1 again. Lost responses after provider acceptance are also ambiguous.
The generic replay interval is 30 seconds with ten attempts; it does not honor
`Retry-After` or classify permanent payload/auth failures. Proactive adapters use
`reqwest::Client::new()` without an explicit total request deadline, while the
delivery serial lock is held across sending.

Record chunk receipts and unknown outcomes, honor provider retry delays, and
bound each send. Do not promise exactly-once delivery where a provider offers no
usable idempotency mechanism. Integrate with the existing outbox and future
effect records; do not create a second schedule/trigger model.

### C15 — P2: Slack audio upload calls an SDK helper as if it were an HTTP API

**Source/API.** `surfaces/slack.rs:421` POSTs a multipart file directly to
`/files.uploadV2` and checks only HTTP status. `uploadV2` is a convenience method
in Slack SDKs, not the documented upload endpoint. The HTTP flow is
`files.getUploadURLExternal`, upload bytes to the returned URL, then
`files.completeUploadExternal`. [`Slack file upload contract`](https://docs.slack.dev/messaging/working-with-files/)

Implement that protocol with typed results. Also derive speech from readable
semantic text: Slack/Discord currently pass joined surface-formatted chunks to
TTS, including markup, while Telegram passes plain reply text. Discord's current
WAV file attachment is not evidence of native Discord voice-message support.

### C16 — P1: Discord mention controls depend on whether an answer has cards

**Source/API.** The card branch sets `allowed_mentions: {parse: []}`. The
ordinary reply branch (`surfaces/discord.rs:390`) and proactive branch
(`delivery.rs:999`) omit it. Model-produced or quoted `@everyone`, role or user
mentions can therefore notify people subject to the bot's permissions. Discord
documents parsing mentions by default when the field is absent.
[`Allowed mentions`](https://docs.discord.com/developers/resources/message#allowed-mentions-object)

Use one explicit mention policy for all messages and continuations, independent
of presentation shape. This is a transport-side safeguard, not a model prompt.

### C17 — P2: a Telegram formatting error writes conversation text to logs

**Source.** `surfaces/telegram.rs:715` logs the first 80 characters of the
rejected message. The very formatting failures above activate that branch.
This violates the repository's content-free logging requirement. Retain status,
job/message correlation and a structural error code instead of a content prefix.

### C18 — P2: thread/topic provenance, files and long-running replies are incomplete

**Source; capability gaps, not all new regressions.**

- Slack message state carries no `thread_ts`, reads channel history without a
  replies traversal, and posts without a thread destination. Telegram drops
  topic/reply metadata and sends only to `chat_id`. Discord does not set a reply
  `message_reference`. A configured Discord thread ID can still function as a
  channel ID; automatic parent-message/thread following is what is absent.
- Only Telegram calls `accepting_files()` and handles returned Office/PDF drafts.
  Slack/Discord do not request or upload those files. Their inbound attachment
  handling is primarily audio; it does not provide Telegram's image/document
  parity. This is missing integration, not evidence the providers cannot do it.
- Card projectors use scalar key/value dumps rather than native image, table,
  source/citation or action elements. Media often becomes text or loses nested
  content through C01. Proactive adapters send text even where immediate bridges
  construct native cards.
- Poll loops await an entire model turn serially. There is no partial answer
  streaming or typing-status implementation in these bridges. Their 202 handlers
  say queued but do not poll for the eventual reply. The gateway timeout directs
  callers to a transcript, but the bridges do not implement that follow-up.
  This audit did not fault-inject the complete queued-turn lifecycle.

Preserve transport destination metadata through admission and delivery. Make
capabilities explicit, and provide honest, usable file and long-run fallbacks.
Do not silently broaden the Agent's authorized channel audience to add threading.

## Provider capability assumptions need updating

This is separate from correcting the defects above. As checked on 2026-10-02:

| Surface | Current Vakyartha approach | Available provider capability |
| --- | --- | --- |
| Telegram | HTML `sendMessage`; tables as preformatted text; comments say no generic rich object | Rich messages, table/media blocks and rich-message drafts are documented in Bot API 10.x. Integration and client compatibility still need acceptance. |
| Slack | Plain mrkdwn and scalar Block Kit fields; tables as code | Block Kit table blocks and a streaming message API are documented. |
| Discord | Text plus scalar embeds; masked links rewritten | Native headings/masked links; embeds and attachments support richer rendering than current field dumps. |

Sources: [Telegram rich messages](https://core.telegram.org/bots/api#sendrichmessage),
[Slack tables](https://docs.slack.dev/reference/block-kit/blocks/table-block/),
[Slack streaming](https://docs.slack.dev/reference/methods/chat.startStream/),
[Discord message API](https://docs.discord.com/developers/resources/message).
These are enhancement opportunities, not permission to claim they already ship.
Slack's three-second polling also needs distribution-aware rate-limit review:
some non-Marketplace commercial installations have a one-request-per-minute
history limit; internal apps have different limits.
[Slack history limits](https://docs.slack.dev/reference/methods/conversations.history/)

## Why existing verification missed this

- `discord::tests::preserves_headings_and_blockquotes_and_lists` asserts `<b>`
  is present instead of validating Discord syntax.
- `tests/audit_final_output_10k.rs:927`, despite its `max_chars` test name, checks
  only that chunks are nonempty. It never asserts their length is within the cap.
- Large audit sweeps mostly prove fallback retention, serialization, and string
  presence. Exact fallback in a packet is not proof that a person receives or
  sees it in a Slack block message.
- Native-card fixtures use small flat metric objects; they do not exercise nested
  plans, grids, citations, multiple cards, overflow or a supplemental warning.
- Telegram card-helper tests exercise a helper the production sender does not
  call. Small HTML examples do not cross entity boundaries or contain formatted
  links in H1/H2 headings.
- The local probes cover deterministic rendering. Failure recovery, HTTP result
  semantics, and actual native-client visibility need additional transport mocks
  and controlled live acceptance; they were not executed by this audit.

## Repair sequence and exit criteria

1. **Preserve facts and valid payloads.** Fix C01–C03 and C08–C12. Use the existing
   parsed document/primitive model rather than growing three line-based Markdown
   parsers. Keep an exact source for audit plus a complete readable fallback for
   people. Add these probes as ordinary regression tests once fixes land.
2. **Make delivery outcomes truthful.** Fix C04–C07, C13–C17. Share provider result
   parsing, bounded sends and delivery receipts between replies and notifications;
   preserve stable upstream admission identities. Respect the data-architecture
   milestone boundary and existing outbox rather than starting M4 implicitly.
3. **Add native richness deliberately.** Address C18 and refreshed capabilities:
   readable cards, tables, linked citations, media, actual returned files, thread
   placement, and progress. Keep one semantic source and deterministic surface
   lowering, with explicit overflow and omission diagnostics.
4. **Accept against real clients.** For each channel, check desktop and mobile:
   headings, nested lists, code with `<>&`, links with parentheses/underscores,
   escaped Markdown, tables, citations, emoji, images, Office/PDF files, ten-plus
   cards, supplemental warnings, thread replies and multi-chunk answers. Save
   screenshots and exact sanitized request/response fixtures in a disposable
   acceptance workspace. Use dedicated test destinations with explicit send
   authorization; this audit did not contact user chats.

Transport acceptance must additionally cover HTTP 200/`ok:false`, 400 invalid rich
payload, 429/Retry-After, network loss after acceptance, failure on chunk 2, process
restart, empty-channel cold start, history larger than one page, and Telegram
startup with several pending updates. Assert both the visible outcome and durable
receipt state, and prove that retry does not re-run tools.
