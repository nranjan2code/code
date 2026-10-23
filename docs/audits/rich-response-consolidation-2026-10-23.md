# Rich response consolidation — 23 October 2026

Status: historical — implementation and local acceptance record; complete native,
model-quality and state/action qualification remains open in the tracker.

The maintainer authorized implementation and tests of doc 104, including prompt
changes, and explicitly requested clean replacement without old compatibility
because there are no users. No release, installation or external message was
requested. The source audit and current research remain dated evidence of the
pre-change tree and published protocols, respectively.

## Implemented contract

`vak_delivery::answer` selects a turn's answer from an already-authorized
OutputTimeline without changing order, original ids, outcomes or provenance.
It supplies readable, spoken and transcript forms. Delivery retains original
OutputItems instead of splitting final narration from cards or synthesizing
successful replacements. Host attachment hold/unavailable notices travel as explicit delivery metadata,
separate from accepted answer items. Actual bridge-request tests verify them.
Provider compilation reports native/fallback coverage
against original item ids. Eligible Slack category charts compile natively
before static form lowering; missing, irregular, oversized or unsupported
shapes retain complete static values and a visible limitation.

Saved-page text and channel fallbacks retain leading, interleaved and closing
prose. Shared conversations reuse the real PageView over their authorized
snapshot. They use invitation-token version reads and offer no owner edit or
review mutation; detached/shared file presentation makes no ambient owner API
request. Rich terminal reads actual typed frames and the shared reducer,
retains content and ignores duplicate/stale sequences. CLI stdout emits the
accepted answer at settlement because pipes cannot retract rejected drafts;
fatal failures retain labelled provisional text.

Conversation replay and snapshots share typed discarded-draft exclusion.
Heartbeat classifies its original reply protocol before adding human-facing
result notices: warnings cannot become findings or turn a `nothing` response
into an alert. Artifact labels share the established inbox display-name helper
with returned attachments, while original paths and ids remain unchanged.
Canonical turn identity is applied inside projection, so endpoints no longer
need independent remapping. The stream reducer lives in vak-delivery, and a
live discard removes the current provisional draft. Source-selection, result
state and ordering stay separate from layout. Existing grants, artifact
versions, effects and action services remain the authority.

The message-only transcript renderer, separate HTML transcoder, split channel
card attachment and special card-followup metadata are removed. HTML export
uses the established Markdown parser with raw HTML inert, checked link schemes,
media as links and a restrictive no-network/no-script policy. Five admitted
rich tools plus Markdown remain authorship. The seed clarifies that rich cards
and surrounding prose are one answer; tool schemas are unchanged.

## Acceptance

- The independently specified JSON corpus covers leading/interleaved prose,
  rich chart, sources, lexical identifier `0007`, exact decimal
  `12345678901234567890.001`, original presentation identity and a partial result
  requiring review. Rust answer/delivery/export, actual channel request bodies,
  terminal frames and Node composition/hydration consume that corpus.
- The production PageView fixture passes at 1440×900 and 390×844 in both themes.
  It verifies a rendered chart, exact source order/scalars, result caution,
  viewport fit, no owner edit/review mutation and no owner API file requests.
- Existing composed-answer/live-placement/result-state checks pass sequentially
  in the real renderer at desktop size.
- Both client bundles build and typecheck. All 47 client Node regression
  scripts pass. The regular gate includes composition, hydration and corpus
  checks so these consumers cannot remain outside Rust-only verification.
- The broad run caught a real heartbeat protocol/display integration regression,
  plus stale expectations about flattened attachment message counts, suppressed
  review notices and positional turn ids. The protocol fix keeps findings and
  their count/urgency separate from human notices; expectations now assert the
  original state, ordered file notice/answer/attachment and stable turn address.
  A broad rerun also exposed a last-cost-row race in an existing gateway
  assertion; the check now selects the admitted RunId rather than a global
  latest row. The clean final full gate includes these fixes and passes.
- The clean final `bash scripts/gate.sh` passes with source frozen throughout:
  formatting, Clippy, workspace tests (722 seconds), answer composition,
  presentation hydration, rich-response corpus, versions, documentation paths,
  records and advisories. Log: `/tmp/vak-rich-response-clean-gate.log`.
- Additional/focused passes: workspace formatting/Clippy; 95 delivery tests; 200 server
  integration tests, 444 server unit tests and the other server targets;
  session/storage/telemetry/terminal/tools/voice suites; workspace doc tests;
  all 47 client scripts; client builds/typechecks; record/doc/version/whitespace
  checks and advisories. Existing ignored/manual tests remain ignored.

## Limits

No native Telegram/Slack/Discord visual, installed WebKit, live-model semantic
quality or real voice acceptance is claimed. No external channel message was
sent. The complete all-form disposition/state/action/erasure/access matrix and
remaining native capability expansion are open under doc 104 R3–R5; current
fallback support is explicit rather than a claim of native interaction.
No parallel answer ledger, provider/UI SDK, action authority or retry owner was
introduced. This is an implementation in the working tree, not a release.
