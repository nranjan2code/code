# Rich responses: current protocols and native surfaces

Status: historical — primary-source research checked on 23 October 2026 to
inform the proposed [rich response contract](../design/104-rich-response-contract.md).
This is documentation/specification evidence, not live provider/client
qualification or authorization to adopt external UI dependencies.

## Finding

Current systems support rich declarative components, streaming updates,
bound data and typed actions. Their responsibilities differ: a component
language, interaction stream, embedded app and channel message API are not
interchangeable. None of the reviewed systems alone supplies Vakyartha's
durable records, result evaluation, permission boundaries and fidelity across
every supported surface.

The useful direction is a rich semantic answer with host-owned rendering.
Markdown can author familiar document structures; it cannot by itself express
every chart, bound control, durable choice or result verdict.

## What is current, and what each source establishes

| System | Verified status at research time | Applicable lesson | Boundary |
|---|---|---|---|
| Google A2UI | Production specification v0.9.1; v1.0 candidate | Known component catalog, stable component identities, separate data bindings and actions, incremental rendering | Not Vakyartha's ledger, permission engine or channel compilers |
| AG-UI | 1.0 stable announced 30 September 2026 | Normative lifecycle/order, streamed messages/tools, state snapshots and deltas | Does not define how rich components are drawn |
| json-render | Changelog lists v0.21.1, 13 October 2026 | Catalog of permitted components/actions, reusable schemas, streamed spec patches and multiple renderers | Library/protocol design reference; not an adopted dependency |
| OpenAI ChatKit | Current official widget/actions/custom-integration guides | Backend-served typed widget components, streamed updates and validated actions | Product SDK and application store contract, not a portable durable answer definition |
| MCP Apps | Stable protocol dated 2026-01-26; development draft separate | Tool-associated UI resources, isolated app embedding, negotiated host capabilities | HTML application boundary; persistence/restoration is not the stable durable-business-state contract |
| Adaptive Cards | Schema 1.6 documented; actual host feature support varies | Typed content/inputs/actions, host styling and explicit feature fallback | Neither uniform host support nor Vakyartha's streaming/replay contract |

Version/status sources: [A2UI](https://a2ui.org/),
[AG-UI announcement](https://www.copilotkit.ai/blog/ag-ui-1.0),
[json-render changelog](https://json-render.dev/docs/changelog),
[ChatKit widgets](https://developers.openai.com/api/docs/guides/chatkit-widgets),
[stable MCP Apps specification](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx),
[Microsoft host guidance](https://learn.microsoft.com/en-us/microsoft-copilot-studio/adaptive-cards-overview).
The OpenAI documentation was checked using the OpenAI Docs workflow.

### Declarative rich components

A2UI separates component definitions from a bound data model. A catalog
defines accepted components and functions; the host draws them. Explicit
actions report resolved input context. Its current protocol includes create,
component-update, data-update and delete operations. This is a useful model
for preserving a chart's identity while its inputs change.
[Current protocol](https://a2ui.org/specification/v0.9.1-a2ui/).

Its renderer matrix also demonstrates why the word portable is insufficient:
framework/version support differs. Catalog fallback guidance can allow an
unsupported node to be skipped; Vakyartha's stronger no-silent-loss contract
must remain. [Renderer matrix](https://a2ui.org/reference/renderers/),
[catalogs and fallback](https://a2ui.org/concepts/catalogs/).

json-render similarly defines a catalog of components, actions and functions.
The catalog controls what is expressible; renderers bind it to host code.
Its SpecStream uses JSON Patch to update a spec progressively. These are
comparison patterns, not permission for arbitrary model-authored effects.
[Catalog](https://json-render.dev/docs/catalog),
[streaming](https://json-render.dev/docs/streaming).

The Vercel AI SDK's UI guides also model custom data as identified message
parts that can update in place; transient parts are deliberately excluded
from message history. This reinforces an explicit distinction between
accepted content and transient progress.
[Streaming custom data](https://ai-sdk.dev/docs/ai-sdk-ui/streaming-data).

### Interaction protocols and application interfaces

AG-UI 1.0 separates authoritative schema structure from normative lifecycle
behavior. Its stream ordering, truncation and atomic state-delta requirements
are useful comparisons for Vakyartha's live/settled contract. Its message
snapshot semantics preserve the positions of existing IDs, however; they
are not identical to Vakyartha's authoritative ordered replacement snapshot.
[Specification](https://docs.ag-ui.com/spec/1.0),
[state semantics](https://docs.ag-ui.com/spec/1.0/events/state),
[transport requirements](https://docs.ag-ui.com/spec/1.0/basic/transports).

OpenAI's current UI guidance directs integrations toward standard MCP Apps
methods and distinguishes authoritative server data, temporary widget state
and durable application storage. Widget restoration state is not business
storage. ChatKit actions may update a widget or stream additional items;
the application must still validate their payloads and authorize effects.
[OpenAI UI guidance](https://developers.openai.com/plugins/build/chatgpt-ui),
[ChatKit actions](https://developers.openai.com/api/docs/guides/chatkit-actions),
[custom integration](https://developers.openai.com/api/docs/guides/custom-chatkit).

MCP Apps embeds a tool-associated, predeclared HTML resource in an isolated
iframe and brokers communication with the host. That is a useful boundary
for an application interface, rather than a replacement for native rich
answer semantics. Vakyartha's repository contract explicitly excludes
adopting third-party agent UI SDKs, runtimes and protocol implementations,
including MCP Apps bridges. This research changes no such rule.
[MCP Apps overview](https://apps.extensions.modelcontextprotocol.io/api/documents/overview.html).

Adaptive Cards illustrates capability-aware rendering: host configuration
owns appearance/interactivity; typed actions return input for application
processing, and feature requirements/fallback address host differences.
Do not infer every feature from an advertised schema number.
[Host configuration](https://learn.microsoft.com/en-us/adaptive-cards/rendering-cards/host-config),
[submit actions](https://learn.microsoft.com/en-us/adaptive-cards/schema-explorer/action-submit),
[host matrix](https://learn.microsoft.com/en-us/agents/design-guidelines/adaptive-cards-for-agent-design).

## Native channel capabilities have moved

| Surface | Verified native richness | Proposal consequence |
|---|---|---|
| Slack | Rich text, basic tables, data tables, native charts, containers, carousels, task cards and block streaming | Preserve chart/table/task semantics until Slack compilation; qualify each feature rather than flatten everything to text |
| Telegram | Rich messages with tables, disclosure, maps, media and buttons; streamed drafts | Use rich blocks where equivalent; keep final durable send separate from an ephemeral draft |
| Discord | Components V2 supports ordered text, containers, sections, media, files and controls; embeds remain supported | Choose one declared message representation and preserve answer order within it |
| Microsoft hosts, if added | Adaptive Cards with host-specific charts, responsive layout and actions | A future adapter needs a host feature profile and action lifecycle |

Slack's chart reference explicitly supports line/bar/area/pie and limits
visualizations per message. Its newer data table adds pagination, sorting,
filtering and action cells, with accessibility requirements. These capabilities
are absent from a heading/prose/table-only intermediate vocabulary.
[Charts](https://docs.slack.dev/reference/block-kit/blocks/data-visualization-block/),
[data tables](https://docs.slack.dev/reference/block-kit/blocks/data-table-block/),
[containers](https://docs.slack.dev/reference/block-kit/blocks/container-block/),
[carousels](https://docs.slack.dev/reference/block-kit/blocks/carousel-block/),
[task cards](https://docs.slack.dev/reference/block-kit/blocks/task-card-block/).

Slack accessible/notification text and each block type have distinct limits.
The top-level 40,000-character truncation rule is not evidence of a universal
sum-of-block-JSON budget. Native streaming and action acknowledgements also
have their own lifecycle.
[Message accessibility/limits](https://docs.slack.dev/reference/methods/chat.postMessage/),
[block streaming](https://docs.slack.dev/reference/methods/chat.appendStream/),
[interaction handling](https://docs.slack.dev/interactivity/handling-user-interaction/).

Telegram's inspected API lists version 10.3, dated 24 August 2026, and rich
message support introduced earlier that year. Its draft preview is ephemeral
and requires a final rich send. Native maps represent a center/zoom, which
does not establish equivalence to Vakyartha's multi-place route or area.
[Bot API rich messages](https://core.telegram.org/bots/api#rich-messages).

Discord's Components V2 mode has different constraints from embeds: its flag
is permanent for that message and it disables normal content/embeds. Its
current component limit is documented; unverified secondary-guide text limits
must not become compiler constants. Interactions require prompt acknowledgement
and bounded continuation tokens; these are not durable task ownership.
[Components](https://docs.discord.com/developers/components/reference),
[interaction lifecycle](https://docs.discord.com/developers/interactions/receiving-and-responding).

## What this changes in the repository diagnosis

The [source review](../audits/response-projection-across-surfaces-2026-10-23.md)
established divergent response consumers. Research reveals a second dimension:
premature loss of rich semantics. In
[`channel/forms.rs`](../../crates/vak-delivery/src/channel/forms.rs), the chart
branch lowers to descriptions/tables and the map branch to named coordinates
before the target provider is selected. Therefore the later native compiler
cannot choose a Slack chart or equivalent Telegram map.

The proposal should preserve typed forms through target selection and move
degradation to the target compiler. Equivalence still needs checking: a map
thumbnail is not a route, and a provider chart with different scale/gap
semantics may need the exact data table as well as the native visualization.

## Evidence limits

Only primary documentation/specifications were used for technical claims.
Dated releases above precede or equal 23 October 2026; undated pages represent
the current documentation inspected on that date. Documentation establishes
the published contract, not availability for the owner's account, permission
scopes, deployment or installed client version. No outbound messages, provider
validation call, iframe execution or live native-client tests were performed.

The recommendation to retain Vakyartha-owned contracts and use the patterns
above is an architectural inference. It does not claim these projects provide
a complete replacement or authorize their adoption.

The source/protocol and native-channel reviews supported the proposal after
clarifying refresh records versus artifact versions and adding mounted-view
erasure/access-revocation acceptance. Record/index, documentation-path,
version, whitespace and subsystem-map syntax checks passed. Blueprint views
were regenerated without needs-review findings. Product/runtime code was
not changed; no build, full Rust gate, release or live acceptance is claimed.
