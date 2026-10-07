# 04 — Tools (vak-tools)
Status: implemented in 2.0.0

## Contract

Tools never panic and never return `Err`. Every failure is a
`ToolOutput { content, is_error: true }` the model reads and self-corrects.
`ToolContext` carries cwd, cancellation child-token, output limits.

## Built-ins

| tool | notes |
|---|---|
| read | 1-based offset/limit paging, numbered lines, binary + image detection, continuation hint |
| write | mkdir -p parents, full overwrite |
| edit | array of `{old_text,new_text}`, sequential application, **atomic** (any failure ⇒ no write), uniqueness enforced, BOM preserved, CRLF detected & normalized for matching, unified diff in output |
| bash | process-group spawn (`setpgid`), tree kill on timeout/cancel, stdout/stderr labeled + capped at 1MB each, exit code surfaced, non-zero ⇒ is_error |
| glob | globset with literal_separator, ignores .git/node_modules/target |
| grep | regex over text files, include-glob filter, match cap |
| webfetch | GET-only bounded fetch (docs/design/29 P4): SSRF guard fail-closed pre-connect + per-redirect re-screen, ≤3 hops, 15s timeout, 8 MiB download cap, content-type filter, no credentials; classified network-capable ⇒ Ask outside FullAccess. An HTML page comes back as its readable text (`format`: `text`, the default; `links`; or `source` for the HTML), read by `vak_intake::html::readable` in the worker (`WorkerTask::HtmlRead`, invariant 14): the page's `<main>`, else its one `<article>`, else its `<body>`; headings, lists and table rows kept; scripts, navigation, other-language links, forms and hidden elements left out and counted in the first line. Raw page source once reached the model whole (384 KB of Wikipedia markup) and the turn ended with no answer. A status of 400 or above is an error value naming only the status, never the page: a 403 page read as a result counted as a retrieval that found nothing, and its text could read as a correctable argument fault |
| browse | headless local Chrome-family `--dump-dom` (JS-rendered DOM) from inside the worker: browser discovery fail-closed (`VAK_BROWSER` override), same SSRF guard on the target, file-based DOM/stderr capture with closed-html sentinel, direct-pid SIGKILL (macOS re-exec), 20s deadline; the DOM comes back through the same page reader and `format` as webfetch; same permission posture as webfetch |
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

- permission checks before execute (Phase 3)
- LSP diagnostics tool (Phase 4+)
- resource claims metadata per tool for fan-out scheduling (Phase 6)
