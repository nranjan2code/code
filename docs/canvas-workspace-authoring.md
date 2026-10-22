# Canvas workspace authoring

Status: **reference — installed declarative workspace contract v1 and pure calculation ABI, 2026-10-22; shared browser UI/worker checks passed; installed-device qualification remains open.**

One common client hosts workspaces on web and desktop. Domain names, units,
commands and context belong in the plugin; the host supplies identity, lifecycle,
recovery, typed transactions, authority and conversation access. The architecture
and future richer surfaces are in [doc 66](design/66-immersive-artifact-canvas.md).

## Package and entry

Start with [the example](../examples/canvas-workspace/README.md). A native
`vak-plugin.json` declares `components.workspaces: ["workspaces"]`. Each JSON
file in that inspected component directory contains one definition:

- `schema: 1`, `id: "plugin-name/workspace-name"`, title and description;
- `initial` JSON state and its `state_schema`;
- `fields` with JSON pointer `path`, label, kind (`text`, `number`, `boolean`,
  `json`) and optional unit;
- `commands` with a local id/title, argument schema, and `edits` mapping state
  `target` pointers to `argument` pointers;
- `context`, the state pointers the person may explicitly select for Ask Agent.

Install and enable through existing Plugins controls; packages start disabled.
In a conversation, choose **More options → Canvas workspaces**. Start an enabled
type or reopen saved work. Workspace keys are namespaced; changing a display
label does not change identity. The live editor keeps one subject for the
resource; transactions carry its actual numeric revision, not the tab's `head`
marker. Missing plugins preserve saved work and pending fields.

Definitions are bounded to 256 KiB and 128 fields/commands/context pointers.
Supported schema keywords are `type`, `properties`, `items`, `required`,
`additionalProperties`, `enum`, `description`. Unsupported validation keywords
are refused, including constraints the shared validator does not enforce.
This is a closed subset, not a claim of full JSON Schema support. Pointers must
resolve in initial state and cannot address prototype keys. No custom HTML,
script URL, React component or native IPC is part of the package UI contract.
The owned compiled renderer registry remains available for richer first-party
renderers. Arbitrary specialized installed layouts are future work.

## Transactions and storage

Commands submit resource id, expected revision, current plugin digest and
registry generation. The server resolves current activation and validates both
arguments and resulting state. Wrong revisions return conflict; there is no
implicit semantic rebase. Disable/update shares the plugin registry OS lock
with bounded resource commits, so stale commands cannot commit after disable
returns. UI polling improves feedback; it is not the authority check.

State and before/after command history are encrypted with the conversation in
existing Documents. Each mutation records an explicit actor and admitted run.
Undo/redo moves a transaction between history stacks and increments revision
through the same CAS path. A new edit clears redo as an explicit branch. Undo
is local domain editing, not compensation for mail sends or other external
actions. Those use existing effects and supported compensation operations.

The full resource document is bounded to 2 MiB; history is not silently pruned.
A transaction exceeding the bound fails without changing saved state. Store
large media/meshes/volumes and deliverables through existing Library artifacts,
with their real version parents. Do not create another file store or grant list.
The catalog indexes resource titles with conversation audience/visibility;
trashing hides the resource, and restoring reindexes it. Conversation erasure
uses the existing scope-key lifecycle.

Pending input uses Canvas checkpoints, independent of saved state/history.
Partial commands keep other pending fields; typing during a save is preserved.
If saved state advances while edits are pending, the view requires explicit
conflict handling. Checkpoints survive remount/unavailability and protect close
and eviction. Opening a tab does not send state to any model.

## Optional pure WebAssembly calculation

A command may declare `compute: "assets/calculate.wasm"`, an inspected package
asset. Authenticated host code retrieves that active package asset; no path or
URL from a module is accepted. Each calculation runs in a fresh dedicated
worker, terminated after completion, error, cancellation, replacement, disposal
or a 30-second interactive budget. There are no module imports, host fetch,
credentials, DOM or native bridge. Module bytes are limited to 16 MiB, with
numeric function types only, one explicitly bounded nonshared 32-bit memory
(maximum 1024 pages / 64 MiB) and bounded tables/functions. GC heap types and
unbounded memory/tables are refused before instantiation.

Export `memory`, `alloc(i32 length) -> i32 pointer`, and
`evaluate(i32 pointer, i32 length) -> i64`. Input is UTF-8 JSON:
`{"state":{...},"arguments":{...},"command":"local-command"}`.
`alloc` supplies the input buffer. `evaluate` returns a packed result with the
output pointer in the upper unsigned 32 bits and length in the lower 32 bits.
The output is UTF-8 JSON command arguments. Input/output are bounded to 2 MiB;
addresses are checked against memory. The worker is destroyed afterwards, so
there is no persistent module state or separate free protocol.

The result is a proposal. The server still validates the command schema and
CAS, but does not rerun the module or prove scientific/numerical correctness.
Agent tools use the same declared command arguments without invoking this UI
calculation. Provide domain validation and evidence appropriate to the work.
For long, externally connected or higher-memory calculations use existing
brokered tools, permissions, run records and artifact outputs. These limits
bound the interactive worker contract, not what a domain extension may accomplish
through authorized execution. Browser engine memory outside linear memory is
not a total-process quota promised by this ABI.

## Agent collaboration

Ask Agent shows the allowed saved fields and shares only those checked by the
person. The server freezes an exact saved revision, validates the active
contribution, and stores the bounded projection as a conversation object. A typed
`WorkspaceContext` ledger entry names the resource, revision, plugin digest,
selected pointers and actor; message metadata links it to the normal admitted
turn. `derive_messages()` reconstructs what the model received. Pending edits
and unchecked fields are excluded from that projection. Context is data, not
an authority grant or permission to perform outside actions.

The Agent's conversation-bound `workspace_read` lists definitions/resources or
reads a current saved resource. `workspace_edit` creates, applies declared
commands, or undoes/redoes through the same core service and normal tool
permissions. The Agent cannot supply another session, Agent home, package path
or credential. Reading a resource explicitly may return all its saved state;
the selected projection does not revoke the Agent's existing conversation access.
Domain values and package descriptions remain untrusted input, as other tool
results do.

## Validation and remaining work

See [the dated acceptance](audits/canvas-plugins-2026-10-22.md). Shared browser UI and actual Worker success/cancellation pass in desktop/phone
light/dark fixtures. Installed native-shell qualification is still required. Full object/region selection, streamed artifact adapters,
stage/inspector/timeline packages and physical-device rendering benchmarks are
broader work, not features of the parameter-editing example.
