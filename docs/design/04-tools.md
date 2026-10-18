# 04 — Tools (vak-tools)
Status: implemented in 2.0.0; checked against the code on 2026-10-14.

## Contract

Tools never panic and never return `Err`. Every failure is a
`ToolOutput::error(fault, content)` the model reads and self-corrects. The
`Fault` (`vak_control::Fault`) says where the fault lies — the call's shape,
what it names, the tool or what it reaches, a refusal, a cancellation — and
is named by the code that fails the call, never read from its message. A
tool also declares whether a call may change the workspace
(`Tool::changes_workspace`, by default from its resource claims): the
workspace is checkpointed when the first such call of a turn is accepted.
`ToolContext` carries cwd, cancellation child-token, output limits.

## Concurrent execution claims

Every call goes through resource-conflict placement. An absent footprint
means unknown effects, so it conflicts with other unknown, path-writing
and exclusive claims. It is not an implicit permission to join the first
wave. Explicitly read-only claims retain their existing concurrency;
disjoint declared write scopes can still share a wave. MCP calls without a
trusted effect declaration remain unknown (hardening A07).

## Reading arguments

Every tool call's arguments pass through one reader before validation
(`vak_tools::read_arguments`, `crates/vak-tools/src/arguments.rs`), for
built-in, loop-owned and runtime-added tools alike. It is driven by the
tool's own schema and never by its name, and it converts only where nothing
is lost and nothing is guessed:

| Sent | Schema asks for | Read as |
|---|---|---|
| JSON text of a list or record | that list or record | the parsed value |
| one record that has the item's required fields | a list of records | a one-item list |
| text | a record with exactly one required free-text field | that field |
| a number or boolean | text | its text |
| text that is exactly a number or boolean | a number or boolean | that value |
| `null` for a field that may be absent | anything | absent |

It never cuts one text into two fields, never joins a list into one text,
never reads lines out of a text as a list's items (found live: broken JSON
read that way became checklist items the person was shown),
and never fills a field with a value the call did not send (a `plan`
deliverable sent as a bare name still lacks its `form`). Whatever it cannot
read is left as sent, and validation refuses it by name.

Text sent for a list or record that is written as JSON but does not parse
is not repaired. Some wires carry every argument as text and their
provider reads it; what the provider cannot read arrives as that text.
The refusal says where it breaks (`vak_tools::json_fault`): the parser's
own complaint, the value it was inside as a path (`blocks[1].rows`), and
the text just before and after the fault. `validate_input` uses it for
every tool. Told only "must be
array", or a column number in a long line, a model sent the same text
again (found live 2026-10-11, docs/design/15-reliability.md).

A shape only one tool can read (a table's nested rows, a list for a field
that tool splits itself) belongs to that tool's `Tool::fill_defaults`,
which runs after the reader and before validation. There is no third
place: a conversion is never added to the agent loop or to a builder.

## Built-ins

| tool | notes |
|---|---|
| read | 1-based offset/limit paging, numbered lines, binary + image detection, continuation hint |
| write | mkdir -p parents, full overwrite |
| edit | array of `{old_text,new_text}`, sequential application, **atomic** (any failure ⇒ no write), uniqueness enforced, BOM preserved, CRLF detected & normalized for matching, unified diff in output |
| bash | process-group spawn (`setpgid`), tree kill on timeout/cancel (a call that names no `timeout_ms` runs under `[control.limits] command_secs`, filled by the loop from `Tool::time_limit_argument`), stdout/stderr labeled + capped at 1MB each, exit code surfaced, non-zero ⇒ is_error |
| glob | globset with literal_separator, ignores .git/node_modules/target |
| grep | regex over text files, include-glob filter, match cap |
| webfetch | GET-only bounded fetch (docs/design/29 P4): SSRF guard fail-closed pre-connect + per-redirect re-screen, ≤3 hops, 15s timeout, 8 MiB download cap, content-type filter, no credentials; classified network-capable ⇒ Ask outside FullAccess. An HTML page comes back as its readable text (`format`: `text`, the default; `links`; or `source` for the HTML), read by `vak_intake::html::readable` in the worker (`WorkerTask::HtmlRead`, invariant 14): the page's `<main>`, else its one `<article>`, else its `<body>`; headings, lists and table rows kept; scripts, navigation, other-language links, forms and hidden elements left out and counted in the first line. Raw page source once reached the model whole (384 KB of Wikipedia markup) and the turn ended with no answer. A status of 400 or above is an error value naming only the status, never the page: a 403 page read as a result counted as a retrieval that found nothing, and its text could read as a correctable argument fault |
| webfetch `render` | (no tool of its own since 2026-10-16: `browse` folded into `webfetch`, offered while `[tools] browse` is on; docs/audits/tool-catalogue-2026-10-16.md O7) headless local Chrome-family `--dump-dom` (JS-rendered DOM) from inside the worker: browser discovery fail-closed (`VAK_BROWSER` override), same SSRF guard on the target, file-based DOM/stderr capture with closed-html sentinel, direct-pid SIGKILL (macOS re-exec), 20s deadline, the browser's own default wait; the DOM comes back through the same page reader and `format` |
| commitments | read-only view of the durable commitments this agent holds (`docs/design/47-commitment-kernel.md`): what is open, what is blocked or waiting on a person, what evidence each still needs before it can close, and how the closed ones closed. Registered when `[commitment] enabled`, and part of the capability slice's orientation floor — an agent that cannot see its own obligations will re-open one. **No write verb by design**: the model may propose criteria and discuss a commitment, but the runtime evaluates satisfaction and the model never marks one passed (`AGENTS.md` invariant 33) |

A capability rather than a gateway slash command on purpose. "What are you
working on", "what is blocked", "did that ever finish" arrive on Telegram, the
desktop and a cron check-in alike; a command layer would answer them on one
transport and add a second dispatch path beside the tool broker. As a tool it
reaches every surface, crosses the same permission boundary, and lands in the
ledger like any other call.

## Output hygiene

A tool returns its whole result and never shortens it; the only bounds are
each tool's own capture limits (bash keeps 1 MiB, webfetch 512 KiB, read a
line range). The agent loop records the whole result in the ledger and
decides how much a request carries (`vak_tools::window`,
docs/design/68-context-engine.md §3): up to 30,000 characters verbatim, and
past that whole lines from the start and the end with one line naming the
omitted range, its size, and the `recall` call that returns it. There is no
spill file. A caller with no ledger (a flow node, a scheduled script) passes
on `vak_tools::bounded`, the same window without the recall hint.

`bash` works in the workspace (or a `cwd` inside it), where the file tools
work; temp files, caches and drafts go to the space's execution root in
the runtime directory, `<runtime>/executions/<space>/<agent_id>/`, never the
project (`vak_config::scope::execution_dir`, AGENTS.md invariant 35).

## Unsafe policy

Exactly one `unsafe` block workspace-wide: process-group kill in bash.rs.
Everything else is safe Rust.

## Later

Permission checks before execute and resource claims for fan-out
scheduling are built (docs/design/08-permissions.md,
docs/design/09-extensibility.md). An LSP diagnostics tool is not.
