# 63 — Agent-first conversations

Status: **superseded and implemented — September 13, 2026; docs/design/64-agent-owned-platform.md is now authoritative for the Agent-owned contract.**

This document is the product and engineering contract for the agent-first
desktop and web experience. The user chooses an agent and talks to that agent;
the runtime owns sessions, tasks, delegation, tools, approvals, workspaces, and
sandbox execution behind the conversation.

## Product contract

Vak is always available as the built-in default agent. User-created agents are
named, persistent helpers shown in the main sidebar. Each helper has one
continuous conversation per workspace. Opening an agent is idempotent: reloads,
second clients, and returning to the sidebar resolve to the same durable
conversation instead of creating another one.

The main interaction is:

`agent in sidebar → agent conversation → plain user request → internal work → useful result`

The user does not need to choose a task, thread, model, sandbox, flow, or
worker. Those remain inspectable in Details/history when needed, but they are
not competing destinations in the primary navigation. Internal lifecycle
events and worker chatter are not rendered as ordinary assistant prose.

## Durable identity

`SessionHeader.agent` is the source of truth. It contains the stable profile ID,
revision, display name, personality, behaviour, and responsibilities. Missing
`agent` means the built-in Vak agent for additive compatibility with existing
ledgers. The identity is frozen into the session header at admission; editing a
profile changes future admissions only. Existing conversations retain the
identity and prompt contract with which they started.

The server admits an agent by scanning the workspace's durable session ledgers,
then opens the earliest matching root conversation or creates one. A short
cross-process admission lock prevents two clients from creating duplicate
conversations concurrently. The browser stores no agent-to-session routing
authority.

Agent personality is a prompt layer, not a permission layer. The prompt states
that the agent is a continuous user-facing identity and that it grants no
tools, credentials, permissions, budget, or approval bypass. The existing
permission engine, broker, sandbox, frozen route ladder, and approval rules
remain authoritative for every agent and every delegated run.

## Scope and configuration

Agent profiles follow the existing configuration layers:

- Shared profiles are stored in the canonical user workspace and are inherited
  by trusted projects.
- A trusted project may replace profiles by stable ID in its own layer.
- Untrusted project profile data is not advertised or admitted.
- `GET /config/agents?scope=user|workspace` returns the exact layer that a
  matching save replaces; `GET /agents` returns the effective, read-only list.

Profile writes are atomic, preserve unknown fields on retained records, and
advance the server-owned revision. A profile cannot be named `vak`. Removing a
profile does not rewrite or mutate existing session ledgers; an already-open
conversation remains resumable with its frozen identity, while a new admission
of the removed ID is rejected.

## API and UI boundaries

`POST /agents/{id}/open` is the sole user-facing admission path. The composer
uses the active agent's conversation and sends the raw request; it does not add
profile instructions or helper prefixes. `POST /sessions/{id}/run` remains an
internal execution endpoint for an already-admitted conversation.

The sidebar contains Vak, saved agents, agent history, and settings. History is
read-only transcript inspection. “New task”/“new conversation” actions resolve
to the currently selected agent's existing continuous conversation. Draft text
and attachments are isolated by workspace and session; late file or voice
callbacks are discarded after an agent switch.

Background and scheduled runs use the same agent identity layer and effective
profile resolution. Their internal child sessions remain hidden from the main
agent conversation unless an explicit result projection makes them relevant.

## Audit evidence

The implementation was checked across the following boundaries:

| Boundary | Evidence |
| --- | --- |
| Sidebar → admission | Browser fixture rendered Vak/Newsy/Other and changed the header and composer to Newsy after one click. |
| Admission → ledger | Server integration test reopened an empty Newsy conversation with the same session ID, including after router/client recreation. |
| Identity → model | Capture-provider test verified the Newsy system prompt and that another agent never received Newsy's private marker. |
| Follow-up continuity | The reopened Newsy request contained the prior marker and did not contain the former helper-prefix protocol. |
| Scope | Test verified Shared profiles are inherited by effective reads but are not copied into the project layer. |
| Profile revision | Test verified edits increment revisions and existing sessions retain their admitted name/revision. |
| Safety | Agent identity is prompt metadata only; all existing permission and broker boundaries remain in the run path. |

Verification commands:

```text
cargo test -p vak-server --test agent_chats
cargo check --workspace --all-targets
npm run build                 # from crates/vak-client-ui
```

The ignored browser fixture is intentionally credential-free and isolated:

```text
cargo test -p vak-server --test agent_chats browser_fixture -- --ignored --nocapture
agent-browser open http://127.0.0.1:<printed-port>/app
```

## Non-goals

This contract does not make profile text a capability grant, merge arbitrary
agent histories, expose hidden execution details by default, or create a new
permission/scheduler/session database. Existing task, flow, sandbox, gateway,
and operations contracts remain the internal implementation owners.
