One Capability Index
====================

Status: proposal (2026-10-16): written from the tool catalogue audit
(docs/audits/tool-catalogue-2026-10-16.md, O8); nothing is built. Open
for the maintainer's decision.

## 1. What is wrong

The prompt lists what a turn can reach in three places, each with its own
way to call:

| Index | What it lists | How it is called |
|---|---|---|
| "More tools" (`capability::tool_catalogue`) | deferred built-in tools, grounded cards, abilities | by the tool's name; its schema loads from that step |
| Skills (`skills::prompt_section_from_capabilities`) | instruction skills | `skill({"name": …})` |
| MCP servers (`mcp_config_section`) | each server and the tool names its last discovery found | `mcp({"action": "call", "server", "tool", "arguments"})` |

Each is byte-stable and changes only when an epoch publishes, so the
cache holds. Two costs remain:

- A model learns three conventions for one act: "use this capability".
  Abilities sat in two indexes until 2026-10-16 (O4).
- The MCP section names every discovered tool of every server, one line
  per server, so it grows with what the owner installs and with no bound
  the model can search. For a commercial install with many servers this
  is the part of the prefix that does not stay small.

## 2. Proposal

One index, "More tools", of every capability that can be called, one line
each, called one way: by its name.

- **Skills** stay documents, not tools: a skill line reads
  `- skill:<name> — <description>` and calling `skill:<name>` by name is
  answered as `skill({"name": …})` is now. The `skill` tool and its
  digest check stay; only the listing and the call's spelling change.
- **MCP tools** appear as `<server>/<tool>` lines, from the catalogue the
  pool last observed (invariant 25: discovery happens only on demand, so
  a server never used yet is one line, `<server>/*`, until its first
  list). Calling `<server>/<tool>` by name loads that tool's own schema
  (`Tool::call_schema`, narrowed as today) and is dispatched through the
  `mcp` broker, whose permission (`Reach::Integration`, matched as
  `server/tool`) and egress proxy are unchanged.
- **Finding** in a long index is the model's reading of one line per
  capability; when an install outgrows that, the index is ranked by the
  capability's declared `serves` domains (doc 41), never by the request's
  words, and what does not fit is reachable by `recall`-style search over
  the catalogue, not cut.

## 3. What must not break

- Invariant 25: no MCP server spawns for the index; an unused server's
  line comes from configuration alone.
- Invariant 31: an epoch publishes additions at a turn boundary;
  revocation stays immediate (`revocation_check`).
- Invariant 36: the prefix stays byte-stable within an epoch.
- Invariant 14 and doc 09: MCP calls still run in their sandboxed
  workers through the broker.
- Self-evolution (doc 93): an ability stays a tool under its name, its
  versions and turn-off unchanged.

## 4. How it would be measured

Replay captured first requests (`scripts/measure_tool_form.py replay`)
on the installed route, before and after, for: a request that needs a
skill, one that needs an MCP tool on a home with two servers, and one that
needs neither; count the first call and whether it is on task.

## 5. Open questions

- Whether `skill:<name>` as a callable name, rather than the `skill`
  tool, reads better on small models: only the replay can say.
- How an MCP server with hundreds of tools is listed before the ranking
  in §2 exists.
