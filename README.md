<div align="center">

# vakcoder

### A coding agent you can inspect, constrain, and extend.

**A local-first Rust harness for running serious coding agents without giving up the receipts.**

[![Version](https://img.shields.io/badge/version-0.9.2-E66A2C?style=flat-square)](CHANGELOG.md)
[![Rust](https://img.shields.io/badge/Rust-2024-2B2B2B?style=flat-square&logo=rust)](Cargo.toml)
[![License](https://img.shields.io/badge/license-MIT-536B58?style=flat-square)](Cargo.toml)
[![Safety](https://img.shields.io/badge/safety-fail--closed-384A6B?style=flat-square)](docs/design/24-agent-security.md)

[Quick start](#quick-start) · [Why vakcoder](#why-vakcoder) · [Features](#what-you-get) · [Architecture](#one-core-many-surfaces) · [Documentation](#documentation)

</div>

![An editorial illustration of vakcoder moving a coding task through an auditable ledger, permission gate, sandboxed execution, and verified patch](docs/assets/vakcoder-hero.webp)

vakcoder is an open-source coding-agent runtime for people who want powerful automation **and** a system they can reason about. It combines a full-screen terminal experience, a native desktop app, a headless CLI, flows, an HTTP/SSE server, and channel-neutral delivery packets on top of one auditable Runtime.

Its thesis is simple: **Codex-grade safety, pi-grade transparency, Claude Code-grade extensibility, and opencode-grade simplicity.**

The result is not another thin model wrapper. Sessions are append-only ledgers, permissions are evaluated before every effect, restricted tools run across a broker boundary, partial work survives cancellation, and every provider dispatch produces a receipt.

> **Project status:** v0.9.2. The greenfield Runtime contract is the only supported architecture. The project is actively developed; see the [architecture](docs/design/36-greenfield-runtime.md) and [changelog](CHANGELOG.md).

## Why vakcoder

Most coding agents make you choose between capability and legibility. vakcoder is built around the idea that the agent can be ambitious while the runtime remains explicit.

| Principle | What it means in practice |
|---|---|
| **Model-visible means logged** | Anything sent to a model can be reconstructed from the session JSONL. |
| **Permission before dispatch** | Every effectful agent turn, tool, flow run, server run, and desktop run passes through the same Runtime policy engine. |
| **Append-only by default** | Branching and compaction create entries; they do not rewrite history. |
| **Failure is part of the contract** | Typed errors, explicit cancellation, terminal outcomes, and preserved partial output. |
| **Extensions stay extensions** | Skills, provider traits, tool workers, and flows add capability without creating another state owner. |
| **Your models, your machine** | Use Anthropic, OpenAI, OpenRouter, OpenCode Zen, Gemini, or Ollama; model catalogues are discovered from the provider. |

## Quick start

For a local build and managed installation, use the scenario-driven installer:

```bash
./scripts/build-install.sh                       # headless base + gateway
./scripts/build-install.sh --with-tui            # add the terminal client
./scripts/build-install.sh --with-tray           # add the menu-bar tray client
./scripts/build-install.sh --with-tui --with-desktop # add both UI clients on macOS
./scripts/build-install.sh --help
```

Add `--no-service` for packaging/container tests or `--gates` to run the full
release gate before installation.

### Release versioning

The workspace version in `Cargo.toml` is the single release version. Bump it
with the checked-in helper, review the generated changelog entry, commit the
result, and run the consistency gate before creating the matching tag:

```bash
./scripts/bump-version.sh patch   # or minor, major, or an explicit X.Y.Z
./scripts/check-release-version.sh
./scripts/release.sh
git tag vX.Y.Z
```

Release and install scripts require a clean commit, and the gate rejects reuse
of a version tag on a different commit.

### 1. Build the base from source

You need a [stable Rust toolchain](https://www.rust-lang.org/tools/install) and Git.

```bash
git clone https://github.com/vakcoder/vakcoder.git
cd vakcoder
cargo build --release -p vakcoder --no-default-features
target/release/vakcoder self install
```

This installs the Runtime service, gateway, embedded browser admin, and a
`~/.local/bin/vakcoder` launcher. Add the TUI, desktop, or tray surface
explicitly when that surface is needed. All surfaces connect to the same
authenticated Runtime and data home.

Install the TUI client:

```bash
cargo build --release -p vak-tui --bin vakcoder-tui
install -m 755 target/release/vakcoder-tui "$HOME/.local/bin/vakcoder-tui"
vakcoder-tui
```

### 2. Add a provider key

Put credentials in a gitignored project `.env`, in `<data_home>/.env` (`~/Library/Application Support/vakcoder/.env` on macOS, `~/.local/share/vakcoder/.env` on Linux), or in your environment. Real environment variables take precedence.

```bash
# Choose one—or use Ollama locally without a hosted-provider key.
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
OPENROUTER_API_KEY=sk-or-...
OPENCODE_API_KEY=...
GEMINI_API_KEY=...
```

Secrets are never forwarded as ambient Bash subprocess state. Project `.env`
files and privileged project configuration are loaded only after the workspace
is trusted.

### 3. Start coding

```bash
vakcoder exec "fix the failing test"
vakcoder flow list
vakcoder admin                   # opens the browser admin console
vakcoder-tui                     # optional terminal client
vakcoder config dump             # inspect the effective configuration
```

The default provider is Anthropic. Select another provider and one of the models discovered for your key in the TUI, through configuration, environment variables, or command flags:

```bash
vakcoder exec "explain this workspace" \
  --provider openai-responses \
  --model YOUR_DISCOVERED_MODEL
```

## What you get

### A real agent loop

- Streaming model output with both deltas and snapshots
- Parallel tool calls scheduled in conflict-free resource waves
- Steering while a run is active, cancellable work, and preserved partial output
- Runtime-registered flow definitions with typed validation and Runtime-backed runs

### Safety that is architectural

- Three permission modes: `read-only`, `workspace-write`, and explicit `full-access`
- Composable `allow`, `ask`, and `deny` rules with deny taking precedence
- Canonical workspace confinement and symlink-escape protection in restricted modes
- Disposable built-in tool workers with bounded operational environments
- Seatbelt on macOS and Landlock on Linux, with fail-closed behavior when the
  selected containment backend is unavailable
- Permission changes cancel in-flight work and reject stale approvals

### Sessions with receipts

- Append-only JSONL trees with branch lineage and compact-as-entry history
- A frozen execution contract recorded at admission
- Provider dispatch receipts with purpose, model, failure domain, settlement, and usage
- State-based workspace checkpoints and restore
- Cross-session search, durable memory, and human-reviewed skill proposals

### An interface for every context

- Retained-render TUI with Markdown, approvals, themes, Vim/Emacs editing, accessibility modes, and Runtime-backed task controls
- Tauri 2 desktop app with isolated workspaces, streaming chat, editor, side chats, tasks, memory, checkpoints, backup, and diagnostics
- **Web admin console** at `/admin` on the secured server — Runtime-owned project, session, run, task, memory, inbox, diagnostics, cancellation, and live-event views (cookie login; see `docs/design/33-admin-console.md`)
- Headless `exec` and `flow` commands for scripts and CI
- HTTP + SSE server for custom clients
- Authenticated gateway with durable task and approval handling

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

vakcoder asks the provider for the models available to your key and caches the result briefly. It does not bake yesterday's model list into the binary.

## Choose your surface

### Terminal UI

```bash
vakcoder-tui
```

Use `/help` inside the TUI to discover commands, `/doctor` to inspect the runtime, `/resume` and `/rewind` for session recovery, and `/keymap` to view or rebind controls.

### Headless execution

```bash
vakcoder exec "refactor the parser" --worktree

vakcoder exec "ship the parser fix" \
  --permission-mode workspace-write
```


### Desktop app

The desktop client requires Node.js/npm in addition to Rust.

```bash
cd crates/vak-desktop/ui
npm ci
npm run build
cd ../../..
cargo run -p vak-desktop
```

On macOS, the project installer can create and install a release bundle:

```bash
cargo install tauri-cli --version 2.11.4 --locked
./build-install.sh
```

Use `./build-install.sh --no-clean` for an incremental build or `--no-install` to produce the bundle without installing it.

### Server and gateway

```bash
vakcoder serve --gateway --port 8901
```

The server exposes the same project, session, run, approval, transcript,
configuration, task, memory, checkpoint, backup, flow, and diagnostics
contracts used by every client. For a durable macOS LaunchAgent or Linux
systemd user service, follow the [hosting guide](docs/hosting.md).

## One Runtime, many surfaces

![A flat editorial diagram showing a shared auditable vakcoder core connected to terminal, desktop, server, and delivery adapters](docs/assets/vakcoder-surfaces.webp)

```text
vakcoder CLI / TUI        Tauri desktop        HTTP + SSE / gateway
         \                    |                    /
          └─────────────── vak-runtime ────────────┘
                               |
       ┌───────────────────────┼───────────────────────┐
       |                       |                       |
 vak-server/client      vak-agent              vak-services
       |                       |                       |
 vak-domain             vak-tools              vak-storage
 vak-config              broker/sandbox          + vak-session
       |                       |                       |
       └─────────────── append-only data + effects ────┘
```

The interfaces do not implement their own privileged shortcuts. They submit
typed commands to `vak-runtime`, which owns configuration, sessions, runs,
approvals, tasks, memory, checkpoints, backups, and effects. Read the
[Runtime architecture](docs/design/36-greenfield-runtime.md) for the crate map
and data-flow contract.

## Permission model

| Mode | Filesystem | Commands | Intended use |
|---|---|---|---|
| `read-only` | Workspace reads only | Restricted | Audits, exploration, review |
| `workspace-write` | Reads and writes inside the canonical workspace | Sandboxed and policy-gated | Everyday coding; the default |
| `full-access` | Unrestricted host access | Unsandboxed, still rule-gated | Explicitly trusted, supervised work |

`full-access` is never selected automatically after a denial, failure, prompt request, or model recommendation. Missing containment fails closed. Unattended gateway turns deny escalations unless an explicitly configured approver surface answers in time.

See the [threat model](docs/design/24-agent-security.md), [permission design](docs/design/08-permissions.md), and [sandbox design](docs/design/25-sandbox.md) before changing security-sensitive behavior.

## Everyday commands

```bash
# Sessions and checkpoints
vakcoder sessions
vakcoder checkpoints list
vakcoder checkpoints restore SESSION_ID SEQUENCE

# Static flows
vakcoder flow list
vakcoder flow check FLOW_NAME
vakcoder flow run FLOW_NAME

# Memory and reviewed learning
vakcoder memory
vakcoder skills-review list

# Deterministic and live evaluation
vakcoder eval
vakcoder eval --live --provider PROVIDER --model MODEL
```

Run `vakcoder --help` or `vakcoder <command> --help` for the complete flags.

## Configuration

Configuration is layered predictably:

```text
<data_home>/config.toml < <project>/.vakcoder/project.toml < environment < CLI
```

Unknown keys warn instead of preventing startup. Provider endpoint overrides,
permissions, sandbox settings, and connection profiles require workspace trust.
Start with:

```bash
vakcoder config dump
```

Then use the [configuration reference](docs/design/05-config.md) for provider,
model, permission, sandbox, limits, and Runtime connection settings.

## Reliability and cost control

- Provider and tool failures remain typed values and are persisted with the run
- Cancellation is propagated through provider and broker calls with bounded teardown
- A single terminal run outcome is persisted even when completion and cancellation race
- Runtime records operational mutations and run outcomes in append-only audit
- Delivery jobs retain their source payload while an external adapter is unavailable
- Deterministic Runtime evals provide a typed, auditable evaluation boundary;
  the workspace test suite covers loop, context, broker, routing, spend, and
  completion behavior

The expected behavior for overloads, network loss, malformed streams, cancellation, and other failures is documented in the [reliability contract](docs/design/15-reliability.md).

## Extending vakcoder

Keep the core small; add specialized behavior at the edges:

- **Skills** load instructions progressively when relevant
- **Tool workers** expose filesystem, shell, web, claims, and sandbox capabilities through the broker
- **Commands** add project, plugin, or user Markdown templates
- **Flows** define project-local TOML metadata that is validated before a
  Runtime-backed run

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
cargo run --bin vakcoder -- eval
```

Live checks need a configured provider key:

```bash
cargo run --bin vakcoder -- eval --live --provider PROVIDER --model MODEL
```

Read [AGENTS.md](AGENTS.md) before changing the agent loop, tool boundary, sessions, permissions, providers, gateway, or sandbox. Its invariants are part of the product contract.

## Documentation

| Start here | Covers |
|---|---|
| [Runtime map](docs/design/00-roadmap.md) | Current authority graph, ownership, and verification |
| [Agent loop](docs/design/03-agent-loop.md) | Turns, tools, steering, scheduling, and outcomes |
| [Sessions](docs/design/02-sessions.md) | Append-only trees, projection, compaction, and contracts |
| [Security](docs/design/24-agent-security.md) | Threat model, trust boundaries, and priority order |
| [Reliability](docs/design/15-reliability.md) | Failures, cancellation, durable state, and recovery |
| [TUI](docs/design/21-world-class-tui.md) | Interaction model, themes, keymaps, and accessibility |
| [Desktop](docs/design/20-tauri-desktop.md) | Native client architecture and workflows |
| [Gateway](docs/design/22-gateway.md) | Authenticated transport, approvals, delivery, and unattended safety |
| [Memory](docs/design/23-memory.md) | Cross-session recall and model-visible search |
| [Learning loop](docs/design/26-learning.md) | Durable notes and human-reviewed skill proposals |
| [Hosting](docs/hosting.md) | Durable local or VPS deployment |

---

<div align="center">

**Powerful enough to do the work. Explicit enough to trust the process.**

</div>
