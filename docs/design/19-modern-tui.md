# 19 — TUI interaction contract

The terminal client is a small Runtime adapter. Its local state is limited to
input and rendering; Runtime remains authoritative for sessions, runs,
configuration, permissions, and effects.

## Interaction

The TUI creates one session, accepts a prompt at a time, prints the Runtime
event stream, and exposes `/sessions`, `/tasks`, `/memory`, `/config`, and
`/quit`. Every request uses the typed `vak-client` API and the authenticated
gateway connection.

## Rendering and failure

Run output is printed from server events. A failed request is shown as an
error; the client never fabricates an empty result or falls back to local
files. Disconnects leave Runtime state intact and can be inspected after
reconnecting.

## Acceptance

```bash
cargo test -p vak-tui -p vak-client -p vak-server
cargo build --release -p vak-tui --bin vakcoder-tui
```
