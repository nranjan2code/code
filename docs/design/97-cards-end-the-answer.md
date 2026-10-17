# 97 — Cards end the answer, on every provider

Status: in progress (2026-10-16): stages A to C built and verified live
on the dev route; the OpenAI route not yet measured with `then`; §6 is
open. Supersedes the interim rule in docs/design/87-composed-answers.md
§2a. Builds on docs/design/86-model-plans-runtime-checks.md (the plan),
docs/design/87-composed-answers.md (composition),
docs/design/89-constrained-tool-arguments.md (schemas the wire enforces),
docs/design/92-card-forms.md (the forms) and
docs/design/68-context-engine.md §5 (the tool surface).

## 1. What is wrong

An answer shown as cards was shown twice. After a step whose cards were
all accepted, the loop asked the model for another step, and on the dev
route (Ollama, `qwen3.5:4b-mlx`) that step wrote the cards' content again
as Markdown or sent the cards again; the chat drew the text under the
cards. The same turns on the OpenAI route (`gpt-6-luna`) ended that step
with no text. A plain "explain X" got no plan and, on the small route,
one card per step.

Our contract relied on silence: the prompt says final text after cards is
optional and never a summary, and the only way to end was a reply with
nothing in it. No provider has a notion of a call whose output the person
already saw; every chat format treats a tool result as input for the
assistant's answer, which is what the small model wrote.

## 2. Evidence (2026-10-16)

Captured requests of real turns, replayed against the model directly
(`scripts/measure_tool_form.py capture`), 4 to 6 times per variant.

The step after an accepted card (dev route):

| variant | restated or re-sent |
|---|---|
| as captured | 2 of 4 |
| the step's own thinking restored | 2 of 4 |
| result says the card is the answer | 3 of 6 |
| the table written into the assistant's own reply | 4 of 6 |
| no system prompt | 3 of 6 |
| only the table tool | 5 of 6 |
| precise sampling (temperature 0.6, presence 0) | 6 of 6 |

Same installed home, same turns, both routes: luna ended after its cards
with no text 3 of 3 and loaded the `answer-layouts` skill; the 4b model
summarized or repeated its cards 3 of 3 while its own thinking said they
were shown ("I've already provided this information as a table card"),
and never loaded a skill.

Planning (dev route, first step of "explain LLM to a standard 9 student"):
the plan tool alone or with six card tools and no system prompt, 4 of 4
plans; our system prompt with seven tools, 1 of 4; no system prompt with
all 25 tools, 0 of 4; the request as sent, 0 of 6; our system prompt with
no card tools, 0 of 12 (it wrote Markdown). The prompt asks for a plan
only when the request asks for more than one thing.

A declared ending (dev route): every card schema given a required
`then` (`more` or `done`):

| first step | card calls |
|---|---|
| a table of the continents | table `done` 5 of 5 |
| pour-over steps | steps `done` 4 of 4 (one `write`) |
| explain LLM | a lone details card `more` 2 of 2; a five-card step ends `done` |

The step after a `more` card: another card 5 of 5, restatement 0 of 5.

What each request carries (captured): the system prompt about 10.7k
characters, byte-stable; tool schemas about 42k, of which the 14 card
tools are 23k (54%) and `mail_calendar` 7.5k (18%) with no account
connected; messages the open turn plus the compact card of the turn
before (docs/design/68). On the native Ollama route the model reads every
tool schema before our system prompt (the `qwen3.5` renderer writes the
tool block first).

## 3. What providers offer

| | deferral | constrained arguments | forced call | caching |
|---|---|---|---|---|
| Anthropic | tool search, `defer_loading` | `strict` (20 strict tools, 24 optional parameters) | refused by the newest models | tools first; a changed tool set invalidates all |
| OpenAI Responses | `tool_search` (newer models) | `strict` (every field required) | `allowed_tools` | whole prefix; changed tools break it |
| OpenAI Chat Completions, OpenRouter, Bedrock, NIM, OpenCode Zen | none native (OpenRouter has a server tool) | `strict` where the model has it | named or `allowed_tools`, varies | prefix |
| Google Gemini | none | `VALIDATED` mode, response schema | `ANY` with allowed names | tools in the prefix |
| Ollama native | none | `format` only | none | token-prefix reuse |

Every vendor advises a small active tool set (fewer than about 20), and
studies find small models lose most as tools grow. Our own audit
(docs/audits/history-lookup-2026-10-16.md) found a discovery step was
chosen needlessly on the smallest route; native tool search is that step.
No provider documents how to end after a display call.

## 4. Design

1. **Each card says whether the answer goes on.** Every card tool takes a
   required `then`: `more` when another card or text follows it, `done`
   when it completes the answer. It is one of the shared fields the card
   path owns (beside `deliverable`), typed, schema-enforced where the wire
   enforces schemas, and read by the loop, never by a builder.
2. **The runtime ends on the declaration.** After a step whose calls were
   all accepted cards (with or without `plan`), the turn ends when the last
   card in issue order said `done` and the plan, if one was made, has
   nothing left to deliver (the plan check's receipts). It ends through the
   end-of-turn checks a step with no call goes through, and no further
   request is made. A card that said `more`, a refused card, a planned part
   still missing, any other call, or a message the person sent meanwhile
   keeps the turn going. A delegated worker's run is never ended this way:
   its text is its report. Silence still ends a turn as before; nothing
   relies on it.
3. **The turn closes on its recorded ending.** A turn whose answer ended
   on its cards has no assistant message after its last step; the turn
   graph closes it on the `EndTurn(Answered)` decision the loop records
   before it writes the turn's card, and the completing step is logged
   once. A reader that shows a run's last words (a routine's delivered
   answer) reads the recorded messages, not the model-facing projection,
   which leaves out text written beside a call once the turn has closed.
4. **The tool surface is smaller.** The card path's shared fields carry
   one-line descriptions, stated the same on every card. Deferral stays the one mechanism every provider
   gets: the "More tools" catalogue, called by name (invariant 30). Native
   tool search is not used: three routes have it, and a discovery step
   measurably hurt the smallest route.
5. **Nothing is decided by words.** `then` is the model's declaration,
   checked against receipts like the plan; no reading of the text after a
   card decides anything (AGENTS.md, "No word lists, no tuned limits").

## 5. Stages

- **A** `then` on every card, the ending, the turn close, the worker
  exception; tests in vak-agent and vak-session; doc 87 §2a points here.
- **B** shared card fields compacted; the request size measured again.
- **C** live on both routes: single-card, multi-card and explain turns,
  with no restatement and no truncated answer; AGENTS.md and
  docs/decisions.md updated.

## 6. Results (dev route, live, 2026-10-16)

Three turns through the server: pour-over steps (steps `done`), a table
of the continents (table `done`), and "explain LLM to a standard 9
student in india, with good UI" (details `more`, then comparison `done`).
Each turn ended on its last card; no text restated a card and no card was
sent twice. The tool schemas went from about 42k to 41k characters (cards
22.9k to 20.8k): the shared descriptions shrank, and `then` was added to
every card.

## 7. Open

- `mail_calendar` (7.5k characters) stays loaded on every turn. Deferring
  it lost the calendar question (16 of 16 reached it loaded, 3 of 16
  deferred; reverted in `c0dcc4439`), and loading it only when an account
  is connected needs a connected-accounts rollup: reading the connection
  ledger at turn start would replay a chain on the turn path, which the
  data rules forbid. Decided 2026-10-17 (docs/decisions.md): build that
  rollup, and load `mail_calendar` only when it names a connected account.
  Built 2026-10-17: the connection ledger is a record chain read through a
  rollup, and `scoped_tools` adds the tool only when
  `MailCalendarTool::has_account` finds an account serving the turn's
  Agent and conversation.

- Composition guidance (the `answer-layouts` skills) reaches only a model
  that calls the skill loader; luna does, the small route does not.
- Planning is asked for multi-item requests only; whether the plan should
  be the answer's outline is left until stage C's measurements.
