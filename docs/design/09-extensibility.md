# 09 — Extensibility (skills · subagents · hooks · MCP · custom commands)

## Skills (shipped)

Markdown packages: `.vakcoder/skills/<name>/SKILL.md` (project) and
`data_home()/skills/<name>/SKILL.md` (user; `~/Library/Application Support/
vakcoder/skills` on macOS, `~/.local/share/vakcoder/skills` on Linux).
Frontmatter `name:` +
`description:` (name falls back to directory). Only the name/description/
path line enters the system prompt; the model reads the file with `read`
when relevant — progressive disclosure, Claude Code-skill compatible.
Discovered skills are recorded in the frozen contract.

## Subagents (shipped, blocking + parallel fan-out + attach/steer)

The `task` tool delegates a self-contained prompt to a child agent:
- child session JSONL linked via `parent_session_id`; greppable lineage
- narrowed contract: child tools exclude `task` (depth-1 by construction)
- inherits permission engine/mode/approver/sandbox/model from parent core
- final text returns as the tool result; abort/failure/turn-limit become
  typed error results
- config: `subagents = false` disables

### Live registry (attach/steer/stop)

While a child runs it registers its steering queues and cancel token in a
per-Core `SubagentRegistry` (`vak_agent`), keyed by the unique child
session id. UIs enumerate live children, push steering/follow-up text into
a specific child, or cancel just that child. The TUI surfaces this as
`Alt-S` / `/subagents`: attaching retargets the composer (Enter steers the
child, Tab queues its follow-up, Ctrl-C stops only it, Esc detaches).
Registration is removed when the tool call returns, so a finished child can
never be steered.

### Resource-claim scheduling

Tools declare `claims(args) -> ResourceClaims { exclusive, read_only, paths }`
(default: unclaimed = schedule freely). The batch scheduler greedily groups
calls into **waves**: unclaimed and read-only calls share wave 0; claimed
calls join the first wave with no conflicting claims, else open a new wave.
Waves execute sequentially; within a wave everything runs concurrently.
Results are always re-ordered into assistant source order.

`task` claims:
- `readonly: true` → read_only claim + child gets read/glob/grep tools and
  ReadOnly permission mode — fans out freely
- `paths: ["src/auth/**", …]` → write scope; scopes conflict when one
  normalized prefix contains the other (`src/**` ⊃ `src/auth/**`)
- no `paths` → exclusive (unknown scope serializes against other writers)

Conflict test is conservative: it may over-serialize, never under-serialize.

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

## MCP client (shipped)

Hand-rolled stdio transport (newline-delimited JSON-RPC 2.0, zero new
dependencies). Config:

```toml
[mcp.servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "…" }
```

Design choices:
- **Lazy connect**: servers spawn on first use; nothing runs when unused.
- **One meta-tool** (`mcp`) instead of per-server tool dumps: `action=list`
  returns compact server/tool names with truncated descriptions; `action=call`
  invokes `{server, tool, arguments}`. Context cost stays ~50 tokens instead
  of 10K+.
- Server-side `isError` results and transport failures become is_error tool
  results the model can react to.
- Relative commands resolve against the workspace cwd; PATH inherited for
  npx/uvx-style launchers.
- v1 limits: no sampling/roots/elicitation; server-initiated requests are
  ignored; 60s request timeout.

## Custom commands (shipped)

Markdown prompt templates invoked as slash commands, discovered from three
namespaces with project > plugin > user precedence on name collision:

| Namespace | Location | Label |
|---|---|---|
| Project | `.vakcoder/commands/<name>.md` | `project` |
| Plugin | `.vakcoder/plugins/<plugin>/commands/<name>.md` | `plugin:<name>` |
| User | `<home>/commands/<name>.md` | `user` |

- File stem is the command name (ascii alnum, `-`, `_` only); names must be
  unique after precedence dedup.
- Optional frontmatter `description:`; falls back to the first non-empty
  markdown line. The body is the prompt template.
- `$ARGUMENTS` substitutes the invocation args; when the template has no
  placeholder and args are present they are appended (`Arguments: …`) so
  the payload is never silently dropped.
- Expanded prompts flow through the normal submit path (mention expansion,
  checkpointing, ledger entry — model-visible means logged).
- Surfaced everywhere commands are: slash completion, the Ctrl-P palette
  (plugin-contributed palette actions), and `/help`.
