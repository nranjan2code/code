<div align="center">

# vak

### A coding agent you can inspect, constrain, and extend.

**A local-first Rust harness for running serious coding agents without giving up the receipts.**

[![Version](https://img.shields.io/badge/version-0.8.0-E66A2C?style=flat-square)](CHANGELOG.md)
[![Rust](https://img.shields.io/badge/Rust-2024-2B2B2B?style=flat-square&logo=rust)](Cargo.toml)
[![License](https://img.shields.io/badge/license-MIT-536B58?style=flat-square)](Cargo.toml)
[![Safety](https://img.shields.io/badge/safety-fail--closed-384A6B?style=flat-square)](docs/design/24-agent-security.md)

[Quick start](#quick-start) · [Why vak](#why-vak) · [Features](#what-you-get) · [Architecture](#one-core-many-surfaces) · [Documentation](#documentation)

</div>

![An editorial illustration of vak moving a coding task through an auditable ledger, permission gate, sandboxed execution, and verified patch](docs/assets/vak-hero.webp)

vak is an open-source coding-agent runtime for people who want powerful automation **and** a system they can reason about. It combines a native desktop app, a headless CLI, flows, an HTTP/SSE server, and chat gateways on top of one auditable core.

Its thesis is simple: **Codex-grade safety, pi-grade transparency, Claude Code-grade extensibility, and opencode-grade simplicity.**

The result is not another thin model wrapper. Sessions are append-only ledgers, permissions are evaluated before every effect, restricted tools run across a broker boundary, partial work survives cancellation, and every provider dispatch produces a receipt.

> **Project status:** v0.8.0. The core roadmap is implemented and live-tested. The project is actively developed; see the [roadmap](docs/design/00-roadmap.md) and [changelog](CHANGELOG.md).

## Why vak

Most coding agents make you choose between capability and legibility. vak is built around the idea that the agent can be ambitious while the runtime remains explicit.

| Principle | What it means in practice |
|---|---|
| **Model-visible means logged** | Anything sent to a model can be reconstructed from the session JSONL. |
| **Permission before dispatch** | Agent turns, tools, subagents, flows, plans, evals, server runs, and desktop runs all pass through the same policy engine. |
| **Append-only by default** | Branching and compaction create entries; they do not rewrite history. |
| **Failure is part of the contract** | Typed errors, bounded retries, watchdogs, circuit breakers, frozen route ladders, and preserved partial output. |
| **Extensions stay extensions** | Skills, hooks, MCP servers, custom commands, and flows add capability without bloating the kernel. |
| **Your models, your machine** | Use Anthropic, OpenAI, OpenRouter, OpenCode Zen, Gemini, or Ollama; model catalogues are discovered from the provider. |

## Quick start

### 1. Build from source

You need a [stable Rust toolchain](https://www.rust-lang.org/tools/install) and Git.

```bash
git clone https://github.com/vak/vak.git
cd vak
cargo build --release -p vak
cargo install --path crates/vak
```

`cargo install` places the `vak` binary in Cargo's bin directory. You can also run every command from the repository with `cargo run --bin vak -- <command>`.

### 2. Add a provider key

Put credentials in a gitignored project `.env`, in the user secret store at
`data_home()/.env` (`~/Library/Application Support/vak/.env` on macOS,
`~/.local/share/vak/.env` on Linux), or in your environment. Real
environment variables take precedence.

```bash
# Choose one—or use Ollama locally without a hosted-provider key.
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
OPENROUTER_API_KEY=sk-or-...
OPENCODE_API_KEY=...
GEMINI_API_KEY=...
```

Secrets are never forwarded as ambient Bash or MCP subprocess state. Project `.env` files and privileged project configuration are loaded only after the workspace is trusted.

### 3. Start coding

```bash
vak exec "fix the failing test"
vak plan "add rate limiting to the API"
vak config dump             # inspect the effective configuration
```

The default provider is Anthropic. Select another provider and one of the models discovered for your key through configuration, environment variables, or command flags:

```bash
vak exec "explain this workspace" \
  --provider openai-responses \
  --model YOUR_DISCOVERED_MODEL
```

## What you get

### A real agent loop

- Streaming model output with both deltas and snapshots
- Parallel tool calls scheduled in conflict-free resource waves
- Steering while a run is active, cancellable work, and preserved partial output
- Goal mode with acceptance criteria, brokered verification, and regression obligations
- Static flow DAGs plus a bounded, fail-closed dynamic planner

### Safety that is architectural

- Three permission modes: `read-only`, `workspace-write`, and explicit `full-access`
- Composable `allow`, `ask`, and `deny` rules with deny taking precedence
- Canonical workspace confinement and symlink-escape protection in restricted modes
- Disposable built-in tool workers and separately sandboxed MCP workers
- Seatbelt on macOS, Landlock on Linux, and an opt-in no-network Docker Bash backend
- Permission changes cancel in-flight work and reject stale approvals

### Sessions with receipts

- Append-only JSONL trees with branch lineage and compact-as-entry history
- A frozen execution contract recorded at admission
- Provider dispatch receipts with purpose, model, failure domain, settlement, and usage
- State-based workspace checkpoints and restore
- Cross-session search, durable memory, and human-reviewed skill proposals

### An interface for every context

- Tauri 2 desktop app with isolated worktrees, streaming chat, diff review, editor, PTY terminal, previews, side chats, and best-of-N comparison
- **Web admin console** at `/admin` on the secured server — live activity feed, session transcripts with search, approval gates, config editing, prompt/steering/best-of-N from any browser (cookie login; see `docs/design/33-admin-console.md`)
- Headless `exec` and `plan` commands for scripts and CI
- HTTP + SSE server for custom clients
- Always-on gateway with Telegram and outbound webhooks, including fail-closed approval forwarding

### Multi-provider without a static catalogue

| Provider flag | API family |
|---|---|
| `anthropic` | Anthropic Messages |
| `openai-responses` | OpenAI Responses |
| `openai` | OpenAI-compatible Chat Completions |
| `openrouter` | OpenRouter |
| `opencode-zen` | OpenCode Zen |
| `google` | Gemini |
| `ollama` | Local OpenAI-compatible Ollama endpoint |

vak asks the provider for the models available to your key and caches the result briefly. It does not bake yesterday's model list into the binary.

## Choose your surface

### Headless and goal mode

```bash
vak exec "refactor the parser" --worktree

vak exec "ship the parser fix" \
  --goal "the parser handles empty input without regressions" \
  --criteria "verify:cargo test -p vak-parser,errors remain typed"
```

Goal mode does not accept the agent's declaration of success on faith: deterministic criteria run through the tool broker, qualitative criteria go through a skeptical judge, and failures are returned to the loop as evidence.

### Desktop app

The desktop client requires Node.js/npm in addition to Rust.

```bash
cd crates/vak-desktop/ui
npm ci
npm run build
cd ../../..
cargo run -p vak-desktop
```

### Install

`scripts/build.sh` builds the workspace and then hands placement to the
managed installer, which owns the install root and its manifest:

```bash
scripts/build.sh
```

`--no-install` builds without placing anything, `--no-desktop` skips the
frontend, and `--prefix DIR` installs somewhere other than the platform
default. Every `self` subcommand accepts the same `--prefix`, so a custom
install stays inspectable and removable:

```bash
vak self status         # build vs manifest vs service units
vak self verify         # every component against its recorded digest
vak self services-sync  # regenerate + reload the launchd/systemd units
vak self reinstall      # clear the prefix and place a fresh build
vak self update --url <feed>  # opt-in pull-and-replace from a release feed
vak self uninstall      # remove it; --purge also deletes the data home
vak doctor              # read-only health report across config, services, routes
```

`vak doctor` diagnoses; `vak doctor --repair` acts on the checks that have a
known mechanical fix (today: reinstalling on self version parity drift) and
re-checks, leaving anything else — provider auth, config warnings — for you.
Checks without a mechanical fix are never guessed at.

`scripts/vak.sh <verb>` is a thin dispatcher over all of the above, if
you'd rather remember one entry point than which of `scripts/*.sh` or
`vak self <verb>` owns a given step:

```bash
scripts/vak.sh build          # -> scripts/build.sh
scripts/vak.sh release        # -> scripts/release.sh
scripts/vak.sh install        # -> vak self install
scripts/vak.sh reinstall      # -> vak self reinstall
scripts/vak.sh verify         # -> vak self verify
scripts/vak.sh status         # -> vak self status
scripts/vak.sh update         # -> vak self update
scripts/vak.sh uninstall      # -> vak self uninstall
scripts/vak.sh services-sync  # -> vak self services-sync
scripts/vak.sh doctor         # -> vak doctor
```

It resolves `vak` from PATH first, falling back to the freshly built
`target/release` or `target/debug` binary — it never reimplements a verb,
only routes to the thing that already owns it.

### Release

```bash
scripts/bump-version.sh 0.8.1   # THE version, plus a lockfile refresh
scripts/check-version.sh        # proves no second version stamp exists
scripts/release.sh --base-url https://downloads.example.com
```

There is exactly one authoritative version — `[workspace.package] version`.
Crates inherit it, `tauri.conf.json` omits the key so Tauri derives it, and
the private frontend packages stay pinned at `0.0.0`. `check-version.sh`
fails if a second stamp reappears anywhere.

`release.sh` builds, checksums each artifact, and writes
`dist/<version>/release.json` — the feed `vak self update` reads. Every
artifact carries a SHA-256 that `self update` verifies before installing;
an artifact without one is refused.

### Server and gateway

```bash
vak serve --port 8901
vak serve --gateway --trust
```

The server exposes the same session, run, approval, transcript, diff, and steering contracts used by the desktop app. For a durable macOS LaunchAgent or Linux systemd user service, follow the [hosting guide](docs/hosting.md).

## One core, many surfaces

![A flat editorial diagram showing a shared auditable vak core connected to terminal, desktop, server, and chat interfaces](docs/assets/vak-surfaces.webp)

```text
vak CLI              Tauri desktop        HTTP + SSE / gateway
         \                    |                    /
          └──────────────── vak-core ─────────────┘
                               |
            ┌──────────────────┼──────────────────┐
            |                  |                  |
        vak-agent          vak-flow         vak-permission
            |                                     |
     ┌──────┼──────┐                              |
     |      |      |                              |
 vak-tools hooks  vak-mcp  ───── broker + sandbox boundary
     |
 vak-session ── append-only ledger, contracts, receipts, recall
     |
  vak-llm ─── providers, streaming, routing, usage accounting
```

The interfaces do not implement their own privileged shortcuts. They compose the same core, permission engine, brokered registry, and session ledger. Read the [architecture roadmap](docs/design/00-roadmap.md) for the full crate map and phase history.

## Permission model

| Mode | Filesystem | Commands | Intended use |
|---|---|---|---|
| `read-only` | Workspace reads only | Restricted | Audits, exploration, review |
| `workspace-write` | Reads and writes inside the canonical workspace | Sandboxed and policy-gated | Everyday coding; the default |
| `full-access` | Unrestricted host access | Unsandboxed, still rule-gated | Explicitly trusted, supervised work |

`full-access` is never selected automatically after a denial, failure, retry, prompt request, or model recommendation. Missing containment fails closed. Unattended gateway turns deny escalations unless an explicitly configured approver surface answers in time.

See the [threat model](docs/design/24-agent-security.md), [permission design](docs/design/08-permissions.md), and [Docker sandbox design](docs/design/25-docker-sandbox.md) before changing security-sensitive behavior.

## Everyday commands

```bash
# Sessions and checkpoints
vak sessions
vak checkpoints list
vak checkpoints restore SESSION_ID SEQUENCE

# Static flows
vak flow list
vak flow check FLOW_NAME
vak flow run FLOW_NAME

# Memory and reviewed learning
vak memory
vak skills-review list

# Deterministic and live evaluation
vak eval
vak eval --live --provider PROVIDER --model MODEL
```

Run `vak --help` or `vak <command> --help` for the complete flags.

## Configuration

Configuration is layered predictably:

```text
defaults < ~/.config/vak/config.toml < .vak/config.toml < environment < CLI
```

Unknown keys warn instead of preventing startup. Privileged project keys—permissions, hooks, MCP servers, gateway, sandbox, and provider endpoint overrides—require workspace trust. Start with:

```bash
vak config dump
```

Then use the [configuration reference](docs/design/05-config.md) for provider, retry, context, UI, permission, hooks, MCP, FinOps, gateway, memory, and sandbox settings.

## Reliability and cost control

- Transient failures retry within a route ladder frozen when the run is admitted
- `Retry-After`, watchdog deadlines, circuit breaking, and cancel-aware endurance are built into the loop
- A shared dispatch ceiling prevents retry multiplication across nested mechanisms
- FinOps admission can enforce per-run, daily, and monthly spend caps
- Every attempt is receipted, including route fallback, cancellation, and typed failure domain
- Deterministic evals cover the loop, context integrity, broker boundary, routing, spend gates, and goal completion

The expected behavior for overloads, network loss, malformed streams, cancellation, open circuits, and other failures is documented in the [failure-handling matrix](docs/design/15-reliability.md).

## Extending vak

Keep the core small; add specialized behavior at the edges:

- **Skills** load instructions progressively when relevant
- **Hooks** observe and gate lifecycle events
- **MCP** exposes external tool servers through a lazy meta-tool
- **Commands** add project, plugin, or user Markdown templates
- **Subagents** run lineage-linked child sessions with attach, steer, and stop controls
- **Flows** define validated, resumable DAGs with typed failure policy

Start with the [extensibility design](docs/design/09-extensibility.md) and [flow design](docs/design/10-flows.md).

## Development

The workspace uses stable Rust, edition 2024. Before submitting a change, run the same checks required by the repository contract:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The deterministic eval suite runs offline in roughly 100 ms:

```bash
cargo run --bin vak -- eval
```

Live checks need a configured provider key:

```bash
cargo run --bin vak -- eval --live --provider PROVIDER --model MODEL
```

Read [AGENTS.md](AGENTS.md) before changing the agent loop, tool boundary, sessions, permissions, providers, gateway, or sandbox. Its invariants are part of the product contract.

## Documentation

| Start here | Covers |
|---|---|
| [Roadmap](docs/design/00-roadmap.md) | Phase history, shipped capabilities, and remaining work |
| [Agent loop](docs/design/03-agent-loop.md) | Turns, tools, steering, scheduling, and outcomes |
| [Sessions](docs/design/02-sessions.md) | Append-only trees, projection, compaction, and contracts |
| [Security](docs/design/24-agent-security.md) | Threat model, trust boundaries, and priority order |
| [Reliability](docs/design/15-reliability.md) | Retries, watchdogs, circuit breaking, and recovery |
| [Desktop](docs/design/20-tauri-desktop.md) | Native client architecture and workflows |
| [Gateway](docs/design/22-gateway.md) | Chat routing, approvals, transports, and unattended safety |
| [Memory](docs/design/23-memory.md) | Cross-session recall and model-visible search |
| [Learning loop](docs/design/26-learning.md) | Durable notes and human-reviewed skill proposals |
| [Hosting](docs/hosting.md) | Durable local or VPS deployment |

---

<div align="center">

**Powerful enough to do the work. Explicit enough to trust the process.**

</div>
