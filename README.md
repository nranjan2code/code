<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/brand/exports/vakyartha-lockup-reverse.svg">
  <img src="docs/brand/exports/vakyartha-lockup-colour.svg" alt="Vakyartha Songbird and wordmark" width="420">
</picture>

# Vakyartha

**An agent you can inspect, constrain, and extend.**

A local-first agent for work that crosses code, documents, research, and everyday operations. You choose the workspace, model, tools, and permission level; Vakyartha keeps a record of what it did and gives you a place to review the result.

[Website](https://vakyartha.com) · [Get started](#get-started) · [Use Vakyartha](#use-vakyartha) · [Settings](#settings) · [Administration](#administration) · [Docs](#documentation)

[![Version](https://img.shields.io/badge/version-5.2.0-E66A2C?style=flat-square)](CHANGELOG.md)
[![Rust](https://img.shields.io/badge/Rust-2024-2B2B2B?style=flat-square&logo=rust)](Cargo.toml)
[![License](https://img.shields.io/badge/license-MIT-536B58?style=flat-square)](LICENSE)

</div>

![The Vakyartha Songbird and character family together in a sunlit room](docs/brand/library/wallpapers/ensemble-day-1920x1080.jpg)

The public product is **Vakyartha**. The command, Rust crates, and data paths use the shorter name **`vak`**. The desktop app, browser workspace, terminal, CLI, and chat channels all use the same agent core. A request can become a quick answer, a reviewed file, a bounded task, or work that continues across sessions.

Vakyartha is built around three promises:

- **You can see what happened.** Sessions are append-only records. Model requests, tool results, provider attempts, and decisions have receipts.
- **You decide what it may do.** Workspace permissions, approval rules, isolated tool workers, and review steps govern actions before they run.
- **You can shape it.** Choose a provider and model, add skills, hooks, MCP tools, plugins, flows, agents, and scheduled tasks without growing the core for every use case.

## Get started

### 1. Build and install

Vakyartha supports **macOS and Linux**. Windows is not supported yet. For the current source tree, the reliable path is to build from this repository. Published [releases](https://github.com/nranjan2code/code/releases) may lag `main`; check the tag before using a prebuilt binary.

You need Git, a stable Rust toolchain, Node.js/npm, and Python 3. From a terminal:

```bash
git clone https://github.com/nranjan2code/code.git
cd code
scripts/build.sh
```

The script builds the client and server bundles, builds Rust, installs through Vakyartha's managed installer, and verifies the installed files. A first install starts no background service. If `vak` is not on your `PATH` afterward, follow the installer's printed instruction or use `target/release/vak` from this checkout. Use `scripts/build.sh --no-desktop` to skip the desktop app, or `scripts/build.sh --no-install` to leave the build in `target/release/`. See the [install and release guide](docs/release-and-install.md) for platform details.

### 2. Set up a workspace

```bash
vak setup
# Or, on a headless machine:
vak setup --terminal
```

Setup walks through a workspace, its trust decision, a provider and discovered model, a permission level, and optional integrations. Bring a provider key or use a local Ollama route; Vakyartha does not include a hosted model. Setup activates always-on services only when you choose that step. `vak setup status` shows what is ready and what still needs attention. Provider keys go to a secret store, not to project TOML or this repository.

### 3. Do a first task

Run these from the workspace you selected:

```bash
vak term
vak exec "Explain this project and suggest a safe first improvement"
vak plan "Investigate the failing tests and propose a fix"
```

`vak term` opens the interactive terminal workspace. `vak exec` runs one request and prints its result. `vak plan` plans and executes an open-ended task. Use `vak exec -C /path/to/project "your request"` to target another workspace from any directory. Run `vak --help` or `vak <command> --help` for exact options.

Want the browser workspace? Start the server in one terminal, then open its signed-in client from another:

```bash
vak serve --port 8901
vak open app
```

The workspace is at `/app`; the management console is at `/admin` (`vak open admin`). The server listens on loopback by default. See [hosting](docs/hosting.md) before exposing it beyond your machine.

## Use Vakyartha

| What you want to do | Where to start |
|---|---|
| Talk through a task and review its work | Desktop app, `vak term`, or `vak open app` |
| Run a one-off request from a script or terminal | `vak exec "your request"` |
| Plan a multi-step task | `vak plan "your goal"` |
| Keep an outcome open until it is verified | Goal mode (`vak exec --goal ...`), then `vak commit list` |
| Inspect past work or recover a workspace | `vak sessions`, `vak checkpoints list` |
| Work with Word, Excel, or PowerPoint files | The workspace review flow or `vak office --help` |
| Read or change a PDF | Send or drop it in a conversation, or `vak office --help` |
| Schedule a routine | `vak tasks --help`; review attention in `vak inbox list` |
| Create a specialist agent | `vak agents templates`, then `vak agents init --help` |

A typical file change starts as a draft. You can inspect the diff or document review, ask for a revision, then accept the result. Cancellation keeps partial work rather than pretending the run never happened. For repeatable workflows, use [flows](docs/design/10-flows.md); for specialized capabilities, use [skills and plugins](docs/design/09-extensibility.md).

### Choose a surface

- **Desktop:** the native workspace with chat, files, diff review, previews, approvals, and a terminal.
- **Browser:** the same workspace client served at `/app`, plus `/admin` for operation and configuration.
- **Terminal and CLI:** `vak term` for conversation; `vak exec` and `vak plan` for headless runs and scripts.
- **Channels:** optional Telegram, Discord, and Slack bridges for an always-on agent. An unknown chat requires admission, and unattended approvals fail closed.

![The CLI, desktop, browser, and channels meet at one governed core](docs/assets/vak-surfaces.webp)

## Settings

Setup handles the first choices. Afterward, the desktop Settings screen and the browser management console provide the everyday controls. The CLI exposes the effective configuration and the permission posture:

```bash
vak config dump
vak config permissions
vak config set-mode workspace-write --scope project
```

| Area | What you control | Where to learn more |
|---|---|---|
| **Model route** | Provider and discovered model, plus scoped overrides for a run or task | [LLM design](docs/design/01-llm.md) |
| **Permissions** | Read-only, workspace-write, or explicit full-access; allow, ask, and deny rules | [Permission model](docs/design/08-permissions.md) |
| **Tools and extensions** | Built-in tools, MCP servers, skills, plugins, and hooks | [Extensibility](docs/design/09-extensibility.md) |
| **Appearance** | Theme, presentation, and technical-detail display | [Visual system](DESIGN.md) |
| **Budgets and reliability** | Spend limits, retries, deadlines, and fallback routing | [Failure handling](docs/design/15-reliability.md) |
| **Always-on work** | Agents, schedules, channels, delivery, and approvals | [Agent model](docs/design/64-agent-owned-platform.md) |

Settings are layered: built-in defaults → Shared settings → this project's `.vak/config.toml` → scoped choices such as a task or CLI flag. The Shared file is `~/vak-home/.vak/config.toml` by default; `VAK_HOME` can relocate the data home. Project settings that grant capability require a trusted workspace. `vak config dump` shows the effective result, and the [configuration reference](docs/design/05-config.md) explains the layers and precedence.

**Secrets are separate from settings.** Provider keys and bot tokens live in a project or Shared secret scope backed by the OS secret service, or an encrypted fallback where no service is available. They are not stored as plaintext `.env` files, returned by the admin API, or committed to Git.

### Permissions at a glance

| Mode | What the agent can reach | Best for |
|---|---|---|
| `read-only` | Reads inside the selected workspace; effectful operations remain restricted | Exploration and review |
| `workspace-write` | Reads and writes inside the canonical workspace through the broker and sandbox | Everyday work |
| `full-access` | Unsandboxed host access, still subject to explicit rules | Supervised work that needs it |

Full access is a human choice. A denial or failed tool call never switches to it automatically. A permission change revokes work that was running under the old mode. The [security model](docs/design/24-agent-security.md) covers the boundaries in detail.

## Administration

Vakyartha keeps normal work and operations in separate views. Start the server and run `vak open admin` to inspect sessions, active runs, approval requests, provider status, channels, scheduled work, delivery, services, and incidents. The console requires authentication; the embedded workspace at `/app` shares that sign-in.

| Task | Command |
|---|---|
| Check setup and diagnose a problem | `vak setup status` · `vak doctor` |
| See install and service drift | `vak self status` |
| Verify installed files | `vak self verify` |
| Reconcile enabled background services | `vak self services-sync` |
| Review scheduled work and attention | `vak tasks list` · `vak inbox list` |
| Inspect durable commitments | `vak commit list` |
| Back up or restore data | `vak backup --help` |

`vak doctor` is read-only. `vak doctor --repair` acts only on checks with a known mechanical fix, then checks again. Installation and setup are separate: placing binaries does not start a gateway or chat bridge. If you run Vakyartha on a server, begin with the [hosting guide](docs/hosting.md); a real hostname requires explicit trusted-host configuration and an appropriate secure front door.

## How it works

![A person and the Vakyartha Songbird following an idea through planning, action, and review](docs/assets/vakyartha-hero.webp)

The agent loop streams responses and tool output, accepts steering and cancellation, and records an append-only session. Every model-visible input must be reconstructable from that record. Tool calls cross a broker boundary into disposable workers; permissions are checked before dispatch. The client shows a reviewable result while the underlying ledger retains the evidence.

Providers are selected from what your credentials can actually reach; model lists are discovered from providers rather than baked into the source. A turn plans its route from current evidence and can retry transient failures within bounded deadlines. See the [agent loop](docs/design/03-agent-loop.md), [sessions](docs/design/02-sessions.md), and [reliability contract](docs/design/15-reliability.md).

## Develop and extend

The public repository is useful for reading, building, and adapting the software. The smallest extension that solves a problem is usually the right one:

- **Skills** add instructions and resources when relevant.
- **Hooks** observe or gate lifecycle events.
- **MCP servers and plugins** connect capabilities without making them core features.
- **Flows** encode repeatable steps with typed outcomes.
- **Specialist agents** own their conversations, workspaces, schedules, and channel targets.

The repository uses Rust 2024. Before committing a change, follow [AGENTS.md](AGENTS.md) and run its required checks:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-version.sh
python3 scripts/check_doc_paths.py
```

## Brand and assets

The [Vakyartha Songbird](docs/brand/README.md) is the public mark. Its [SVG master](docs/brand/mark/vakyartha-songbird.svg) and [ready-to-use exports](docs/brand/exports/) include light, dark, one-ink, wordmark, and app-icon variants. The [asset gallery](docs/brand/library/index.html) collects the canonical artwork, characters, and wallpapers. Use the full **Vakyartha** name in public writing; `vak` remains the CLI and internal identifier.

## Documentation

| Start here | For |
|---|---|
| [Documentation guide](docs/README.md) | Find current guides, design records, and dated research |
| [Install and release](docs/release-and-install.md) | Platform support, build, install, update, verification, and uninstall |
| [Configuration](docs/design/05-config.md) | Settings layers, trust, secrets, and capability inheritance |
| [Admin console](docs/design/33-admin-console.md) | Browser management surface and operations |
| [Hosting](docs/hosting.md) | Local services, SSH access, gateway, and remote exposure |
| [Security](docs/design/24-agent-security.md) | Threat model and permission boundaries |
| [Agent-owned platform](docs/design/64-agent-owned-platform.md) | Agents, channels, workspaces, and durable state |
| [Architecture roadmap](docs/design/00-roadmap.md) | Crate map, shipped stages, and project history |
| [Brand guide](docs/brand/README.md) | Logo, artwork, exports, colour, and usage |

The `docs/design/` files each declare a **Status**. Read that line before treating a design as shipped behavior.

## License and contributions

The original software and documentation are available under the [MIT License](LICENSE). The Vakyartha name, Songbird mark, character artwork, and wallpapers are excluded from that software license; see the [brand guide](docs/brand/README.md). Third-party components retain their own license notices.

[@nranjan2code](https://github.com/nranjan2code) alone maintains changes to this repository. External pull requests are not accepted.
