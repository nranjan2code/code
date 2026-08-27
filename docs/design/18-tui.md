# 18 — Terminal client

`vak-tui` is a thin line-oriented Runtime client. It owns terminal input and
rendering only; it never opens Runtime files, creates a local agent, or runs a
second server.

## Connection

Startup resolves an explicit `VAKCODER_URL`/`VAKCODER_TOKEN` pair first. For
the managed local Runtime, `vak-client` resolves the live receipt and performs
the authenticated `/version` and `/health` handshake before the TUI registers
the canonical project. The TUI itself never reads a Runtime file or guesses a
port. Connection failures are printed and terminate the client with a non-zero
status, so the next invocation performs a fresh resolution.

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
