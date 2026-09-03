# Capability registry

VAK treats every extension as a typed capability. A capability has one name,
kind, invocation mode, source, content digest, and provenance. The admitted
capability list in the session contract is the authority for prompt
advertisement, tool visibility, dispatch, subagent inheritance, and audit
surfaces. Each live agent snapshots the definitions of that admitted tool set
once for all provider requests in its run.

Admission is a **ceiling**, not a floor. Three layers narrow it per turn and
none of them may add: channel visibility, `reach` (which drops a capability
whose every use is a foregone denial), and the intent kernel's capability
slice (`docs/design/47-commitment-kernel.md`). The packet in the header is
unchanged by any of them, so a turn narrowed today is restored tomorrow
without a new session.

## Kinds and invocation

| Kind | Invocation | Runtime boundary |
| --- | --- | --- |
| Tool | model tool call | broker, permission engine, hooks, sandbox |
| Skill | `skill({name})` or `/skill:name` | read-only loader, admitted digest |
| MCP server | `mcp` broker | lazy connection, server/tool policy |
| Hook | automatic lifecycle event | isolated hook runner and failure mode |
| Command | user slash command | host expansion before model dispatch |

A skill is still a document. `skill` is the only model-callable loader; a
skill name is never registered as a tool name. Explicit `/skill:name` input is
expanded before provider dispatch, while natural-language selection uses the
same admitted skill set through the loader schema. Relative references are
resolved from the skill directory. A changed digest makes the capability
stale and requires a new session rather than silently changing instructions
mid-run.

MCP remains lazy so an optional integration cannot block turn admission. The
session's admitted MCP server entries are converted into an exact broker
allowlist, preventing a newly configured server from appearing inside an
already frozen session. MCP calls still cross the ordinary permission engine.

Hooks and commands are not tool-shaped. Hooks run at declared lifecycle
events; commands are expanded at the input boundary. Plugin contributions use
the same kinds and carry plugin provenance instead of creating a parallel
execution path.

## Packet invariants

1. The model may call only schemas from the tool set admitted by `Tool`
   entries; one agent run reuses one snapshot of those schemas.
2. Runtime objects not present in the admitted packet are filtered before a
   request is built.
3. Skill and MCP loaders may resolve only admitted entries.
4. Subagents inherit the parent's admitted packet, narrowed to their actual
   tools and mode; they never rediscover a wider environment.
5. Unknown names and kind mismatches are structured error values. They never
   panic the loop and never trigger an effect before authorization.
6. Capability changes apply to new sessions. Existing sessions do not mix an
   old prompt with a new registry. Gateway bindings compare the frozen packet
   with the current Core admission contract and rotate stale sessions while
   retaining their append-only ledgers.
7. A host may add run-scoped direct-write paths. They are evaluated before a
   `write` or `edit` tool is dispatched and never depend on model compliance
   with a natural-language scope instruction.
8. Per-turn narrowing is a subset operation, never a set operation. A slice
   naming a capability that was never admitted is ignored rather than
   conjuring it, and only `Tool` entries are sliced: a skill is already
   progressively disclosed by its loader and an MCP server is already lazy,
   so slicing those would spend risk for no context saving. When a slice
   removes a tool the turn actually needed, the model asks for it and that
   escalation is recorded as a *measured* misread rather than a guess.

## Why the previous design failed

The old skill protocol put a skill name and absolute file path in prose and
asked the model to call generic `read`. That protocol differed from common
agent harnesses, competed with tool-call training, exposed paths, and was
validated only after a hallucinated call reached dispatch. The session header
also recorded separate tool and skill name vectors while the runtime rebuilt
its prompt and tools from live Core state. Those parallel representations
could disagree.

The registry removes the parallel vectors. The frozen system prompt, runtime
filter, skill loader, MCP allowlist, and subagent contract now consume the same
typed descriptors.
