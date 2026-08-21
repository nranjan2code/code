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

## Hooks (planned, Phase 5 slice 2)

Lifecycle events (`PreToolUse`, `PostToolUse`, `Stop`, `SessionStart`,
`PreCompact`) with matcher syntax (`Bash(git *)`), script handlers receiving
JSON on stdin. Durable session events stay separate from live hook events.

## MCP client (planned, Phase 5 slice 3)

Lazy connect + deferred tool listing to avoid context dumps; tools appear
behind the same Tool trait with a `mcp__<server>__<tool>` naming scheme.
