# 102 — Native channel cards

Status: implemented: shared form projection, native provider compilers and
shared direct/proactive request plans are built and tested locally. Real
provider/client visual qualification remains open; no historical card shape or
provider downgrade path is kept. Approved on 2026-10-22.

## Problem and boundary

The installed investigation in
[the audit](../audits/channel-card-rendering-2026-10-22.md) found correctly
emitted summary/sources cards rendered as a document in Telegram, raw nested
payloads in Slack, and missing nested content in Discord. The model contract
is already correct. Rendering fixes do not add a prompt, classifier, topic rule,
new card kind, schedule, persistence model or delivery retry owner.

A card's `StructuredOutput.block` owns its information shape. Its semantic kind
may select a pack; its title, subject, prose and payload key iteration never
select a renderer. Each current form in docs/design/92-card-forms.md has one
explicit channel projection. Unknown forms fail explicitly; they are never
repaired by guessing an older schema. Validate before compiling the whole
request plan, so a bad later card cannot cause an avoidable partial send.

## One pipeline

`vak_delivery::channel::card` validates and projects a form to presentation
`Element`s: heading, prose, context, labelled fact, list, table, code, explicit
link and disclosure. They are transient values, not a new stored or model-visible
contract. Built-in forms exhaust that vocabulary; no provider sees raw payload
keys, serialized arrays, semantic identifiers or arbitrary UI definitions.

`channel::messages` compiles the complete ordered provider request plan from
cards followed by their trailing narration. The gateway already separates the
accepted cards from followup text; it must not turn that text back into a
second copy of the cards. Both direct bridge replies and effect-backed pushes
consume this function. Transport code adds only route, auth, provider response
handling and existing effect identity. A queued/error status has no answer
packet and uses `channel::notice` for literal, bounded basic text. It is not
rendered as an answer card or compatibility with a missing old packet.

No extra persistence or retry exists here. The existing effect dispatcher owns
proactive delivery. Provider refusal is a failure; it never automatically sends
an alternative representation. Network ambiguity keeps the existing unknown
outcome classification. Discord nonces identify the complete effect/message
index pair before bounding to its 25-character protocol limit.

## Native rendering

- Telegram: `sendRichMessage` with explicit rich blocks. Summary headlines
  are headings, labels are footers, lists and tables retain structure, and
  findings are kept in expandable source details. No Markdown-to-HTML card
  stage, text-art divider, or basic-message downgrade.
- Slack: native `rich_text` sections/lists/preformatted blocks and native
  tables. Explicit links are link elements. No generic JSON fields. Slack
  receives the complete plain accessible/notification fallback for each
  message's blocks, with mention detection and Markdown interpretation disabled.
  The conversation shows the native blocks; the fallback is never truncated.
- Discord: embeds, with authored headlines and complete descriptions. Nested
  lists, fields and source findings remain present. Tables become labelled
  records because embeds have no table cells. No semantic type is shown as
  a fallback title. Allowed mentions are empty.

Authored prose syntax is parsed once into flat styled runs: emphasis, code
and explicit links become native entities. HTML remains literal and inline
images become inert links. Flattening styles bounds provider nesting; no prose
syntax is interpreted as another card or as authority.

Source label and title are separate elements, not concatenated into a repeated
heading. A summary's observation label survives. A source's URL, publisher,
date and finding remain attached; no similarity or word matching decides a
finding is redundant. Telegram disclosure reduces default density; other
surfaces preserve the finding visibly rather than claiming unsupported hiding.

Charts retain descriptions and all measured values as tables. Maps retain
named coordinates and connection semantics. Controls retain recorded inputs,
computed outputs and expressions. Checklists are explicitly static. Active
previews require the app, with source/path retained. Media remain links:
rendering never fetches a remote image, executes HTML or grants a control.
Limitations are explicit presentation context, not silent content removal.

## Limits and continuation

Provider limits are wire constraints, not tuned readability thresholds:

| Provider | Published constraints used |
|---|---|
| Telegram rich messages | 32,768 characters; 500 blocks including nested rows/items; nesting 16; 20 table columns |
| Slack | 50 blocks/message; 40,000 accessible-message text characters; table 100 rows, 20 columns, 10,000 aggregate cell characters |
| Discord | embed title 256; description 4,096; aggregate embed text 6,000; message content 2,000; nonce 25 |

Text splitting uses Unicode boundaries and conservatively counts UTF-16 units.
Rows and columns continue in provider-sized panels with repeated headers;
a row that cannot fit becomes labelled cell records. A disclosure that cannot
fit continues visibly. Message/block/character limits cause continuation,
never `.take()` truncation, a maximum card count or loss of a nested tail.
Discord uses one bounded embed per request to satisfy its aggregate limit.
Routing fields, full URLs and authentication are never shortened to fit.

Primary references checked on 2026-10-22:
[Telegram rich messages](https://core.telegram.org/bots/api#rich-message-formatting-options),
[Telegram sendRichMessage](https://core.telegram.org/bots/api#sendrichmessage),
[Slack rich text](https://docs.slack.dev/reference/block-kit/blocks/rich-text-block/),
[Slack tables](https://docs.slack.dev/reference/block-kit/blocks/table-block/),
[Slack accessibility](https://docs.slack.dev/reference/methods/chat.postMessage/),
[Discord embeds](https://docs.discord.com/developers/resources/message#embed-object).

## Verification

Use current-form fixtures for every form on all three channels. Verify the
screenshot's summary/sources shape, kind independence, nested tails, source
provenance, labels, Unicode and exact lexical table scalars, more than ten
cards, long cells and wide/long tables, no blank sends, safe links, inert media,
and actions after the final continuation. Transport tests must assert real
HTTP bodies/methods, application-level refusal and partial failure; compiler
snapshots alone do not prove provider acceptance. Live provider/client visual
qualification is a separate measured result, not inferred from a mock or build.
