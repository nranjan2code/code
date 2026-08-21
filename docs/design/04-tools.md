# 04 — Tools (vak-tools)

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

## Output hygiene

Line-level truncation marker; total-size cap keeps head+tail with an omitted
marker and optional spill file. Models see bounded, greppable outputs.

## Unsafe policy

Exactly one `unsafe` block workspace-wide: process-group kill in bash.rs.
Everything else is safe Rust.

## Later

- permission checks before execute (Phase 3)
- LSP diagnostics tool (Phase 4+)
- resource claims metadata per tool for fan-out scheduling (Phase 6)
