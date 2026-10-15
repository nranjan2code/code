# Grounded cards: measured before and after (2026-10-15)

Status: reference

What changed: a card that shows only what a call produced
(`Tool::grounded_in`: terminal and test report on `Ran`, diff and file
preview on `Wrote`, sources on `Retrieved`, the two ability cards on
`Answered`) is loaded once the conversation holds such a call, and
promoted in place at the step one appears (docs/design/68-context-engine.md
§5). Every other card is loaded from the first call, as before.

## Method

Route: Ollama `qwen3.5:0.8b-mlx` (the installed route), a dev server on a
fresh throwaway home behind the capture proxy
(`scripts/measure_tool_form.py capture`). Eight scenarios, the same words
before and after: "remember that" (my name), "whats my name?" (same
conversation), compare two note apps, a beach packing list, "what is a
LLM", masala chai step by step, a table of three planets, write and run
`hello.py`. The first request of each turn was captured, then replayed
unchanged 16 times (`replay --forms free`), so the only difference between
the two sets is the code under test. Before is `e82f4f08b`.

## Results

Request size: 68.8K to 69.0K characters and 35 tools before; 57.2K to
57.4K characters and 28 tools after (17% smaller). The six grounded cards
are listed under "More tools" instead.

First-step choice, pooled over 16 replays (on-task: the tool or plain
answer the request calls for; lookup: `recall`, `session_search` or
`find_tools`; bad: invalid arguments or an unknown tool):

| scenario | before on/lookup/bad/other | after on/lookup/bad/other |
|---|---|---|
| remember | 6 / 8 / 0 / 2 | 6 / 6 / 0 / 4 |
| name | 2 / 14 / 0 / 0 | 0 / 16 / 0 / 0 |
| compare | 1 / 8 / 4 / 3 | 0 / 15 / 1 / 0 |
| list | 7 / 9 / 0 / 0 | 1 / 13 / 2 / 0 |
| define | 6 / 6 / 1 / 3 | 8 / 3 / 3 / 2 |
| steps | 4 / 6 / 2 / 4 | 3 / 8 / 1 / 4 |
| table | 0 / 8 / 2 / 6 | 0 / 9 / 2 / 5 |
| code | 10 / 3 / 1 / 2 | 15 / 0 / 0 / 1 |
| all | 36 / 62 / 10 / 20 | 33 / 70 / 9 / 16 |

The list difference was checked by editing the captured requests: the
after request without the six catalogue lines, and the before request with
only the seven schemas removed, each scored 3 of 16 on task. The true rate
there is near 3 in 16; 7 and 1 are the two ends of the noise.

Live, in the code scenario: step 1 offered 28 tools and no grounded card;
the model called `write`; step 2 of the same turn offered 30, with the diff
and file preview cards promoted in place. The terminal card stayed
deferred, because the `bash` call that followed was declined at approval
and no process ran.

## Reading

The change does what it was for: smaller requests, no file preview or
terminal card offered to a conversation that has made nothing (the
installed route's 23-step loop began with one), and code turns chose
`write` first more often. It did not change the first step overall. On
this route about half of all first steps are a history or discovery lookup
on a fresh request, in every variant; that is the next contract question
(the prompt's history and discovery wording), recorded in
docs/plans/open-items.md.

Not measured here: a hosted route, and multi-turn conversations where a
grounded card is loaded from an earlier turn.
