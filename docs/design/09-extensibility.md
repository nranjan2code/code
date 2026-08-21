# 09 — Extensibility (skills · subagents · hooks · MCP)

## Skills (shipped)

Markdown packages: `.vakcoder/skills/<name>/SKILL.md` (project) and
`~/.vakcoder/skills/<name>/SKILL.md` (user). Frontmatter `name:` +
`description:` (name falls back to directory). Only the name/description/
path line enters the system prompt; the model reads the file with `read`
when relevant — progressive disclosure, Claude Code-skill compatible.
Discovered skills are recorded in the frozen contract.

## Subagents (shipped, blocking)

The `task` tool delegates a self-contained prompt to a child agent:
- child session JSONL linked via `parent_session_id`; greppable lineage
- narrowed contract: child tools exclude `task` (depth-1 by construction)
- inherits permission engine/mode/approver/sandbox/model from parent core
- final text returns as the tool result; abort/failure/turn-limit become
  typed error results
- config: `subagents = false` disables

## Hooks (shipped)

Config-declared script handlers at lifecycle points:

```toml
[[hooks]]
event = "pre-tool-use"        # session-start | pre-tool-use | post-tool-use | stop
match = "Bash(git push *)"    # optional; same rule syntax as permissions
command = "scripts/guard.sh"
timeout_ms = 5000             # optional, default 10000
```

Contract: handler receives JSON on stdin
(`{event, session_id, cwd, tool?: {name, input}, text?}`); answers via stdout
JSON `{"decision":"block"|"approve","reason":"..."}` or exit code 2 (stderr =
reason); silent exit 0 = no opinion. First block wins.

Semantics:
- `pre-tool-use` block ⇒ tool never executes; reason becomes an is_error
  result the model adapts to.
- `post-tool-use` block ⇒ annotates the tool result with the reason.
- `stop` block ⇒ appends a logged `[stop-hook]` continuation message and the
  loop continues (bounded by max_turns).
- Hooks run in process groups, killed on timeout/cancel; hooks are live
  events, never persisted as session entries (their *effects* are visible in
  the transcript).

## MCP client (planned, Phase 5 slice 3)

Lazy connect + deferred tool listing to avoid context dumps; tools appear
behind the same Tool trait with a `mcp__<server>__<tool>` naming scheme.
