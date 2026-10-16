# 89 — Constrained tool arguments

Status: in progress: stages 1 to 4 built 2026-10-12 (the capability, strict tools,
schema-bound output on every wire, regeneration of unreadable calls, the
ladder by fault class); stage 5 (richer schemas under a constraint) in
progress. Written at the maintainer's request after cards stopped reaching
the screen and tool calls kept failing: a design, not a spot fix. Builds on doc 88 (the control plane: step
contracts, model capabilities, the correction ladder) and
docs/audits/cards-in-answers-2026-10-11.md (the measurements). No provider,
model or tool is first-class: every mechanism below is a capability an
adapter declares and the runtime learns per wire and model.

## 1. What fails

A tool call's arguments are text the model writes. Vak reads that text, and
when it cannot, it tells the model and asks again: the correction ladder
(docs/design/15-reliability.md "Staged repair"). Measured on 2026-10-11 and
2026-10-12 against Vak's own captured requests (system prompt, tools and
conversation exactly as sent), eight runs per case, on the dev route:

| How a card's arguments were asked for | routine step | note-taking step |
|---|---|---|
| one nested call of every block (`answer`, retired) | 5–6 of 8 valid | 2 of 8 |
| one block per call, one union schema | 0 of 8 | 1 of 8 |
| one call per card, each its own precise schema | 8 of 8 | 6 of 8 |
| the card's schema enforced by the wire (constrained output) | — | 16 of 16 (table and checklist, 8 each) |
| a broken call regenerated under its schema, given the call (§4.3) | — | 4 of 4 valid, intent kept; rows not cells |

The failures are not misunderstandings. They are syntax: a stray `}`, a raw
newline inside a string, characters after the end, a list written as text.
The ladder answers every failure the same way: it says what broke, shows the
schema, then withdraws the tool. Showing a schema cannot fix a slip of
syntax, so the same model makes the same class of slip again; a live turn on
2026-10-12 (`run_01a12d82-35a3-7ef0-ba55-53404cec3a4e`) needed six refused
calls to show three cards, and another failed outright when the wire could
not parse the model's tool call (`run_01a12d8b-f0b1-76c3-8849-0d468db9cb2c`).
Retry with feedback lowers the failure rate; it cannot remove it.

## 2. How other systems solve it

The established fix is to make invalid output impossible to generate, not
to repair it afterwards: the provider compiles the JSON Schema into a
grammar and masks every token that would break it (constrained decoding).
Every provider family Vak supports offers it, in different places and with
different limits.

| Provider family | Tool arguments constrained | Output constrained | Switch | Limits that matter |
|---|---|---|---|---|
| Anthropic (Messages, and through Bedrock, Vertex, Foundry) | yes: strict tool use | yes: JSON outputs | `tools[].strict: true`; `output_config.format` `{type: json_schema}` | `additionalProperties: false`; at most 20 strict tools and 24 optional parameters per request; no numeric or string-length constraints; schema too complex is a 400; changing a tool's schema invalidates the compiled grammar cache |
| OpenAI Chat Completions | yes: strict function calling | yes | `tools[].function.strict: true`; `response_format` `{type: json_schema, strict: true}` | every property in `required`, optional as a `null` union, `additionalProperties: false` |
| OpenAI Responses | yes, normalized to strict by default when the schema allows | yes | `tools[].strict`; `text.format` | as above; a non-conforming schema silently stays non-strict unless `strict: true` is explicit |
| Google Gemini | function calling mode `VALIDATED` (schema adherence; to verify live) | yes | `toolConfig.functionCallingConfig.mode`; `responseMimeType` + `responseJsonSchema` | an OpenAPI subset; large or deeply nested schemas may be refused in `ANY` mode |
| Amazon Bedrock (Converse) | yes: strict tool use | yes | `toolConfig.tools[].toolSpec` with `strict: true`; `outputConfig.textFormat` | a JSON Schema 2020-12 subset; per-model availability |
| Ollama (native) | no: tool-call arguments are parsed from the model's text | yes | `format: <schema>` | none documented beyond the schema |
| OpenAI-compatible servers (vLLM, llama.cpp, LM Studio, OpenRouter and others) | varies by server and model | usually | `response_format` `{type: json_schema}`; vLLM also `guided_json` | llama.cpp ignores unsupported keywords and can refuse a large total tool schema; OpenRouter's support is per model (`supported_parameters`) |

Where no constraint is offered, libraries read leniently instead of
refusing: schema-aligned parsing and JSON-repair recipes fix unescaped
newlines, missing brackets and similar slips before validating, and one
coding assistant chooses an output format per model from measured results
instead of one format for every model. The same three ideas carry
over: enforce where the wire can, read leniently only where the fix is
lossless, and choose per model from measurement.

Every source stresses the same limit: a grammar guarantees structure, never
content. In the constrained measurement the table's rows came back as
sentences rather than `A | B` cells, and `deliverable` held the plan's JSON
instead of a deliverable's name. Content stays the builders' and the
runtime's checks.

Sources: Anthropic, [Structured outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs);
OpenAI, [Function calling](https://developers.openai.com/api/docs/guides/function-calling);
Google, [Function calling](https://ai.google.dev/gemini-api/docs/function-calling);
AWS, [Get validated JSON results from models](https://docs.aws.amazon.com/bedrock/latest/userguide/structured-output.html);
Ollama, [Structured outputs](https://registry.ollama.ai/blog/structured-outputs);
vLLM, [Structured outputs](https://docs.vllm.ai/en/v0.8.1/features/structured_outputs.html);
llama.cpp, [llama-server](https://www.mintlify.com/ggml-org/llama.cpp/api/tools/llama-server);
LM Studio, [Structured output](https://lmstudio.ai/docs/developer/openai-compat/structured-output);
OpenRouter, [API reference](https://openrouter.ai/docs/api/reference/overview);
and the published notes of the parsing libraries and coding assistants
described above.

## 3. Where Vak stands

- No adapter sends any constraint: no `strict` tool, no schema-bound output,
  no `format`. Every call's arguments are free text on every wire.
- The card schemas follow rules measured for unconstrained output (flat, no
  list of records in a list, no data in keys, doc 30 §30.1). They are right
  for that case and some are stricter than a constrained wire needs.
- The ladder has one response for every failure, whatever its class. The
  typed fault (`vak_control::Fault`) does not yet separate a syntax fault
  (the text is not JSON) from a shape fault (valid JSON, wrong fields) or a
  refusal (the tool's own rule).
- `ModelCapabilities` already records what a wire and model accept, by
  declaration and by learned refusal (`tool_choice`, reasoning, sampling):
  the place a constraint capability belongs.

## 4. Design

### 4.1 A capability, declared and learned

`ModelCapabilities` gains what the wire can constrain, for this model:
tool arguments, output, both or neither. Each adapter declares its reach
from the provider's published API (§2), and the runtime learns a refusal
the same way it learns one for `tool_choice`: a request refused for its
constraint (a schema too complex to compile, a model without the feature)
is recorded for that wire and model, and the next request leaves it out
rather than failing the turn. Nothing is assumed from a provider's name.

### 4.2 Strict tools where the wire enforces them

Where the wire constrains tool arguments, every admitted tool whose schema
fits that wire's limits is sent strict. Vak keeps one canonical schema per
tool; the adapter renders the wire's strict form from it (OpenAI: every
property required, optional ones as a `null` union, `additionalProperties:
false`; Anthropic and Bedrock: `additionalProperties: false`, within 20
strict tools and 24 optional parameters, the rest sent as they are). The
receipt records which tools went strict (`Applied`), so a turn can be
explained. A call on these wires cannot arrive as broken JSON.

### 4.3 Regenerating arguments that cannot be read

Where the wire constrains output but not tool arguments, and as the general
response to a syntax fault on any wire: when a call's arguments cannot be
read, the runtime regenerates them once, in a step of purpose `repair`
whose output is constrained to that tool's schema (the wire's output switch
from §2). The step is given the call as the model wrote it and the reader's
fault, so the intent (which tool, what content) is the model's own and only
the encoding is corrected. The regenerated arguments are recorded in the
ledger as their own entry naming the call they replace (invariant 1: what
reaches a model is reconstructable), and the call then runs as normal. A
wire with neither constraint gets the same step unconstrained, which is the
ladder's first rung as it is today.

Before regeneration, a lossless reading is tried: a raw control character
inside a JSON string is escaped, because there is exactly one valid reading
of it. No other repair is guessed (a stray `}` has two readings; that is the
regeneration's job).

### 4.4 The ladder follows the fault's class

> **Changed 2026-10-14.** A shape fault was told first and regenerated only
> when the same tool failed again in the turn. Live, an `emit_steps_card`
> call with no `steps` was told and the model never called the tool again,
> so the plan's steps were lost without a decision. Every call its schema
> refuses is now regenerated at once, as an unreadable one is, except a card
> kind another card tool owns, whose rewrite under this tool's schema would
> change what the call is.

Tools report a typed fault; the ladder answers by class, once each:

| Fault | First response | Then |
|---|---|---|
| syntax: the arguments are not JSON | regenerate under the schema (§4.3) | tell, with the place it breaks |
| shape: valid JSON, wrong or missing fields | regenerate under the schema | tell, naming the fields and the schema |
| shape: a card kind another card tool owns | tell, naming the tool that owns it | the answer stands without it |
| refusal: the tool's own rule | tell, with the rule | the answer stands without it |

Measured 2026-10-13 (admin › How the models did): `office_apply`, deferred
on the dev route and known to the model only from the "More tools" line,
was refused by its schema in 5 of 15 calls, each a first call written as
`{operation, content, title}` before any schema was sent. The next call,
with the schema loaded, was right every time.

A tool is withdrawn only when a regenerated call fails again: the frame has
changed and still failed, and asking a third time would be the loop that
withdrawal exists to end. Every step is a recorded decision (doc 88).

### 4.5 What a constraint makes possible

Measured on 2026-10-12: a broken table call regenerated under its schema
came back valid every time, with the call's `deliverable`, label and type kept,
but each cell as a row of its own: "cells separated by `|`" is a convention
inside a string, which no grammar enforces. Under a constraint the cells
belong in the schema itself.

The first regeneration attempt with no cap on its output produced nothing
in fifteen minutes: a constrained step is given an output budget (the step
contract's `max_tokens`), as every step is.

Rules that exist only to keep unconstrained JSON writable can be relaxed on
a wire that enforces the schema, per capability: a table's rows as lists of
cells rather than `A | B` strings, and enums built for the turn (a card's
`deliverable` limited to the plan's deliverable names, its `semantic_type` to
the admitted kinds). The canonical schema stays the portable one; a wire
that enforces gets the richer form, measured before it is switched on.

### 4.6 Measured per route, not assumed

The method used for §1 is a script, `scripts/measure_tool_form.py`
(built 2026-10-13): `capture` is a logging proxy in front of the route
(on the dev route, `VAK_OLLAMA_BASE_URL` pointed at it), and `replay`
sends one captured request N times per form (`free` as captured,
`schema` with the chosen tool's schema as the output format) and reports
whether each call's arguments, as written, fit the tool's schema. It
measures what the model writes, before Vak's argument reader; an error
names the type that was sent, so a list written as its JSON text shows as
such. Strict tools on a hosted wire are a form still to add. A form is switched on for a route when it is
measured there, and the numbers go in this document. The dev route is one
row; hosted routes are measured with one-off checks, as the maintainer
allows.

## 4.7 Where it applies: every structured output, not only cards

| Where the model writes data | How it is constrained now |
|---|---|
| Every tool call in the agent loop: built-in, plugin, skill and loop-owned tools (`plan`, `task`, `recall`), cards included | strict tools where the wire enforces them (§4.2); regeneration under the tool's schema when unreadable (§4.3) |
| An MCP server's tool, called through the `mcp` broker | regeneration against the tool's discovered `inputSchema` (`Tool::call_schema`); the broker's own schema has a free `arguments` object, so it has no strict form |
| The completion judge's verdicts (`goal::audit_request`) | schema-bound output; the lenient reader stays |
| A managed-work contract (`/until-done`, `AuthoredContract`) | not yet: its schema would be a second copy of the session's work types; to derive from the types themselves |
| The flow planner, compaction and handoff | read no JSON from the model's text; nothing to constrain |

## 4.8 Built

- `vak_control::ModelCapabilities::{strict_tools, schema_output}` and
  `StepSettings::{strict_tools, schema_output}`: declared per adapter
  (`REACH`), honoured per dispatch, and learned from a refused request by
  sending it again without the constraints first
  (`DispatchSettings::retry_without_settings`); only a refusal the
  constraints alone explain is recorded against them.
- `vak_llm::strict`: the OpenAI and Anthropic strict forms of a canonical
  schema and the choice of strict tools within Anthropic's published limits
  (`ToolDefinition::prefer_strict`, set for card tools).
- `ChatRequest::output_schema`, rendered by every adapter in its own form.
- `vak_tools::unreadable` (the syntax fault), `Tool::call_schema`,
  `WorkPurpose::Repair`, and the loop's `regenerate_unreadable` with its
  `arguments-regenerated` activity keeping what the model first wrote.

## 5. What it replaces

- The single response to every tool failure (tell, show the schema,
  withdraw), with the fault-class ladder of §4.4.
- Free-text tool arguments on wires that can enforce a schema.
- The rule set of doc 30 §30.1 as a universal constraint: it stays the
  portable form, and a wire that enforces may get the richer one (§4.5).

## 6. Stages

1. The capability (§4.1), declared per adapter, learned from refusal, with
   the measurement script (§4.6) run on the dev route and one hosted route.
2. Strict tools on the wires that enforce them (§4.2), with the per-wire
   schema rendering and its limits; hosted checks for each family.
3. Typed syntax and shape faults, the lossless reading, and regeneration
   (§4.3) with its ledger entry.
4. The fault-class ladder (§4.4), replacing today's single response.
5. The richer schemas a constraint allows (§4.5), each measured first.

## 7. Open questions for the maintainer

- Strict tools change the request's tool definitions; some wires invalidate
  a compiled-grammar cache when a schema changes. Turn-built enums (§4.5)
  would change the schema every turn. Accept that cost, or keep enums out
  of strict schemas?
- Anthropic allows 20 strict tools per request. With more admitted, which
  go strict: the card tools first, or the tools that failed this run?
- Regeneration costs one extra model step per unreadable call. Accept it on
  every route, or only where the wire constrains output?

### Stage 5 measurement (2026-10-12, not built)

The captured note-taking request, replayed 8 times on the dev route under
Ollama `format` with the table card's `rows` as a list of cell lists
(`array` of `array` of `string`): 5 of 8 came back as a grid (a header and
rows of one width, at least 2 cells and 3 rows), 3 of 8 did not. Under the
same constraint with `A | B` row strings, every reply was valid JSON but its
rows were single cells. A richer schema under a constraint helps without
settling it; stage 5 waits on a measurement on a hosted strict route before
it is built.
