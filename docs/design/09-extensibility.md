# 09 — Extensibility (skills · workers · hooks · MCP · custom commands)
Status: implemented in 2.0.0

## Skills (shipped)

Markdown packages: `.vak/skills/<name>/SKILL.md` (project) and
`data_home()/skills/<name>/SKILL.md` (user; `~/Library/Application Support/
vak/skills` on macOS, `~/.local/share/vak/skills` on Linux).
Frontmatter `name:` +
`description:`. Names must be lowercase kebab-case, 1–64 characters; descriptions
are required and capped at 1024 characters. Only the name/description/
path line enters the system prompt; the model reads the file with `read`
when relevant — progressive disclosure, Claude Code-skill compatible.
Discovered skills are recorded in the frozen contract.

Optional frontmatter `serves:` declares what the skill is for, against the
domain vocabulary in `docs/design/41-capability-registry.md`:

```yaml
---
name: quarterly-report
description: Assemble the quarterly report from the finance export
serves: documents, live-data
---
```

Accepts `a, b` or `[a, b]`. Omitting it leaves the skill *undeclared*, which
is never narrowed away by the per-turn capability slice — skills are already
progressively disclosed by their loader, so this is advisory context rather
than a gate.

`vak skills validate [PATH]` validates every `SKILL.md` below the supplied
file or directory; without a path it validates both project and user roots.
`allowed-tools` is advisory only and never grants authorization.

Same-name precedence: project shadows user. Runtime discovery uses the winner,
while inventory APIs retain losing entries with `shadowed` and provenance so
the admin and desktop surfaces can explain why a skill is inactive.

## Workers (shipped, blocking + parallel fan-out + attach/steer)

The `task` tool delegates a self-contained prompt to a child agent:
- child session JSONL linked via `parent_session_id`; greppable lineage
- narrowed contract: child tools exclude `task` (depth-1 by construction)
- inherits permission engine/mode/approver/sandbox/model from parent core
- final text returns as the tool result; abort/failure/turn-limit become
  typed error results
- config: `workers = false` disables

### Live registry (attach/steer/stop)

While a child runs it registers its steering queues and cancel token in a
per-Core `WorkerRegistry` (`vak_agent`), keyed by the unique child
session id. UIs enumerate live children, push steering/follow-up text into
a specific child, or cancel just that child. The TUI surfaces this as
`Alt-S` / `/workers`: attaching retargets the composer (Enter steers the
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
enabled = true                # optional, default true (absent = enabled,
                               # for every [[hooks]] entry written before
                               # this field existed)
failure_mode = "open"        # optional; "closed" blocks on spawn/timeout errors
```

`enabled = false` keeps the entry in config rather than removing it —
`vak_core::build_hooks_from` (`crates/vak-core/src/lib.rs`) skips a
disabled entry when it builds the live `HookDef` list, so "disable" and
"delete" are genuinely different operations. This matters because
`PUT /config/hooks` (vak-server) replaces the whole project-layer list on
every write: before `enabled` existed there was nowhere to record "off,"
so the admin console's only way to represent a disabled hook was to drop
it from the file outright — a checkbox that silently deleted the hook it
unchecked. See AGENTS.md rule 21.

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
- `failure_mode = "open"` reports spawn, timeout, wait, and non-zero exit
  failures without blocking; `failure_mode = "closed"` blocks the operation
  on those failures. Explicit hook decisions retain precedence in either mode.
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
# Optional. What this server is for, so the per-turn capability slice can
# match it (docs/design/41-capability-registry.md). Omitting it leaves the
# server *undeclared*, which is never narrowed away — declaring domains only
# ever makes the slice tighter, so this is a context optimisation you opt
# into, never a requirement for the server to work.
serves = ["vcs", "documents"]
```

Design choices:
- **Lazy connect**: servers spawn on first use; nothing runs when unused.
- **Pooled with a liveness check and an idle TTL**: a cached connection is
  validated before reuse, so a server whose child has exited is replaced
  rather than dispatched into, and one left idle is shut down and respawned
  on demand instead of held for the life of the daemon.
- **Catalog changes are observed, not polled blindly**:
  `notifications/tools/list_changed` is routed into the capability reconcile
  loop. A server that does not declare `tools.listChanged` is re-probed on a
  short rhythm instead, because silence carries no information.
- **A failed probe is a state, never a catalog**: it carries a reason, a
  remedy, and a `retry_at` with exponential backoff, so a server that was
  down at boot rejoins on its own with no restart. It never becomes a
  callable tool and never blocks its own retry.
- **Probes are concurrent** under a 10s per-server budget, so the cost of N
  unreachable servers is the max of that budget rather than the sum.
- **One meta-tool** (`mcp`) instead of per-server tool dumps: `action=list`
  returns compact server/tool names with truncated descriptions; `action=call`
  invokes `{server, tool, arguments}`. Context cost stays ~50 tokens instead
  of 10K+.
- Server-side `isError` results and transport failures become is_error tool
  results the model can react to.
- Relative commands resolve against the workspace cwd; PATH inherited for
  npx/uvx-style launchers.
- v1 limits: no sampling/roots/elicitation; server-initiated *requests* are
  ignored (notifications are not); 60s request timeout for calls, 10s for
  discovery probes.

## Custom commands (shipped)

Markdown prompt templates invoked as slash commands, discovered from three
namespaces with project > plugin > user precedence on name collision:

| Namespace | Location | Label |
|---|---|---|
| Project | `.vak/commands/<name>.md` | `project` |
| Plugin | `.vak/plugins/<plugin>/commands/<name>.md` | `plugin:<name>` |
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
