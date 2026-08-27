# 22 — Runtime gateway

The gateway is the authenticated HTTP/SSE adapter around one `vak-runtime`
instance. It owns no sessions, runs, configuration, or channel state. The
server process opens the Runtime once, exposes typed resources, and shuts it
down as one unit.

## Request path

```text
client (CLI/TUI/desktop/admin) → bearer or cookie auth → vak-server
                                  → vak-runtime → storage/session/config
```

Every mutating endpoint calls the corresponding Runtime method. The server
does not maintain a second cache or write directly to SQLite, JSONL, config,
blobs, or audit files.

## Exposed resources

The secured router exposes health/version, projects, sessions and transcripts,
runs and cancellation, SSE events, configuration and permission mode, tasks,
approvals, inbox records, memory, skill proposals, checkpoints, backup,
flows, eval, provider model discovery/key lifecycle, workspace file access,
and diagnostics. `/admin` serves the embedded admin client; login creates an
HttpOnly SameSite=Strict cookie backed
by the same gateway token.

Run cancellation is forwarded to Runtime's cancellation token. Runtime keeps
partial session output and records one terminal outcome. Event subscribers
receive the event delta and snapshot supplied by Runtime.

## Singleton and credentials

Gateway mode owns `<data_home>/locks/runtime.lock` and writes
`runtime/gateway.json` only while the listener is alive. The runtime file is
removed during graceful shutdown. The bearer token is loaded from the scoped
Runtime configuration; it is never placed in service-unit environment blocks,
logs, session entries, or worker environments.

## Channel boundary

`vak-delivery` is a pure projection layer. It turns an assistant result into a
typed packet, exact Markdown fallback, and bounded chunks. A channel adapter
may consume that packet, but it cannot mutate Runtime state or bypass the
authenticated API. The gateway itself does not embed a second channel bridge.

## Security

Authentication happens before resource handlers. Runtime validates project
ownership, permission mode, capability epoch, workspace lease, and broker
authorization immediately before every effectful operation. External text
cannot change credentials, sandbox, or approval policy.
