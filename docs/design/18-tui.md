# 18 — Terminal client

`vak-tui` is a thin line-oriented Runtime client. It owns terminal input and
rendering only; it never opens Runtime files, creates a local agent, or runs a
second server.

## Connection

Startup resolves an explicit `VAKCODER_URL`/`VAKCODER_TOKEN` pair first, then
the live authenticated endpoint in `<data_home>/runtime/gateway.json`,
performs the authenticated health check, and registers the canonical project
when needed. Connection failures are printed and terminate the client with a
non-zero status. This makes the installed `vakcoder-tui` work with the
managed local gateway without requiring users to copy a token into their
shell environment.

## Commands

The TUI creates one session, accepts prompts, streams the Runtime event feed,
and exposes `/sessions`, `/tasks`, `/memory`, `/config`, and `/quit`. Events are
printed from the server-provided values; cancellation and permission changes
remain Runtime operations.

## Safety

The TUI has no direct access to SQLite, JSONL, blobs, secrets, tools, or
service managers. Runtime performs project checks, capability-epoch checks,
permission checks, and broker authorization for every effect.

## Verification

```bash
cargo test -p vak-tui -p vak-client -p vak-server
cargo build --release -p vak-tui --bin vakcoder-tui
```
