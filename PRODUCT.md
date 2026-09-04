# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Primary user: a solo, security-conscious technical user who wants powerful agent automation but refuses to give up inspectability and control. They run vak against their own machine, files, and tools, want to see what the agent actually did, and want to constrain what it's allowed to do before it does it.

## Product Purpose

vak is a local-first Rust harness for running general-purpose agents without giving up the receipts. It lets a user delegate real work — engineering, research, writing, data, and operations alike — to an agent while retaining an inspectable, constrainable system: every session is reconstructable, every effect is permission-gated, and failures are handled explicitly rather than silently.

## Positioning

vak's mechanism is "one core, many surfaces": a single auditable, policy-gated core (append-only session ledgers, permission-before-dispatch on every effect, receipts for every provider dispatch) drives a CLI, a Tauri desktop app, a browser client served by the server itself, an HTTP/SSE server, and chat gateways. Other agent tools bolt safety or transparency onto individual surfaces; vak enforces the same policy engine and produces the same reconstructable record everywhere the agent runs, so behavior and auditability don't vary by which surface you're using.

## Operating Context

- Runs locally against the user's own machine, workspace, and connected tools, or on a headless Linux box reached through a browser (tunnel, tailnet, or a TLS-terminated hostname); provider credentials come from a gitignored project `.env`, a user secret store, or the environment.
- Sessions are append-only JSONL ledgers; branching and compaction append rather than rewrite history.
- Agent turns, tools, subagents, flows, plans, evals, server runs, and desktop runs all pass through the same permission/policy engine before any effect.
- Every turn is resolved into a typed, recorded "intent": a reading on seven behavioural axes, and an engagement derived from it that can only *narrow* what the run may do. Work that outlives a session becomes a durable commitment whose completion the runtime verifies against the world rather than accepting from the model.
- Supports multiple model providers (Anthropic, OpenAI, OpenRouter, OpenCode Zen, Gemini, Ollama for local use).
- Extensibility (skills, hooks, MCP servers, custom commands, flows) is additive and does not bloat the core.
- Can run as a durable background service (macOS LaunchAgent / Linux systemd user service) via the server, in addition to interactive CLI/desktop use.

## Capabilities and Constraints

- CLI (`vak exec`, `vak plan`, `vak config dump`, `vak intent explain`, `vak commit`, etc.) is the headless surface.
- Tauri 2 desktop app (`crates/vak-desktop`) and the browser client at `/app` — one source tree (`crates/vak-client-ui`) behind a host port, built twice: isolated worktrees, streaming chat, diff review, editor, PTY terminal, previews, side chats, best-of-N comparison, commitment portfolio. Capabilities that differ by host (native dialogs, a real shell) are absent rather than broken where the host cannot provide them.
- Admin UI (`crates/vak-admin-ui`): in scope for design work alongside the desktop app.
- HTTP/SSE server (`crates/vak-server`) exposes the same session, run, approval, transcript, diff, and steering contracts used by the desktop app, and serves the workspace client at `/app`. It binds loopback by default; reaching it by a real hostname is an explicit opt-in (`[server] trusted_hosts`) that the server refuses to start without.
- Requires a stable Rust toolchain and Git to build; desktop client additionally requires Node.js/npm.
- Terminology: "receipts" (auditable record of provider dispatch), "ledger" (append-only session record), "surfaces" (CLI/desktop/server/gateways sharing one core), "reading" (what a request is, on seven axes), "engagement" (what the runtime will therefore do — narrowing only), "commitment" (a durable obligation with a verifiable done-condition).

## Brand Commitments

- Product name: vak.
- Existing tagline: "An agent you can inspect, constrain, and extend." (Broadened from "A coding agent…" — vak is no longer positioned as coding-only; do not reintroduce the narrower wording.)
- Existing README hero/surfaces illustrations use a flat editorial illustration style (`docs/assets/vak-hero.webp`, `docs/assets/vak-surfaces.webp`) with badge colors E66A2C (version), 2B2B2B (Rust), 536B58 (license), 384A6B (safety) — treat as existing brand evidence, not yet confirmed as binding design tokens.

## Evidence on Hand

- README.md documents quick start, architecture ("one core, many surfaces"), and a features/documentation index.
- `docs/design/` contains architecture design docs (e.g. `20-tauri-desktop.md`, `24-agent-security.md`, `00-roadmap.md`).
- No testimonials, customer names, benchmarks, or pricing exist; future work must not fabricate them.
- Current version: 2.0.1 (per README badge and CHANGELOG), actively developed.

## Product Principles

1. Auditability is not a feature bolt-on — every surface must produce the same reconstructable, inspectable record.
2. Nothing acts without permission — every effect (tool, subagent, flow, server run) passes the same policy gate before dispatch, and this must stay visible/legible in the UI, not just enforced invisibly.
3. Failure is part of the contract — typed errors, partial-work preservation, and explicit failure states are first-class, not edge cases to hide.
4. One core, many surfaces — design consistency and behavioral consistency across CLI, desktop, server, and gateways matters more than any single surface's polish in isolation.
5. Local-first and user-controlled — the developer's own machine, keys, and codebase; the product should never feel like it's phoning home or acting behind the user's back.
