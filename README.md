<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/brand/exports/vakyartha-lockup-reverse.svg">
  <img src="docs/brand/exports/vakyartha-lockup-colour.svg" alt="Vakyartha Songbird and wordmark" width="420">
</picture>

# Vakyartha

**From a request to work you can review.**

An open-source AI agent for code, documents, research, and everyday work.<br>
Runs on your machine. Uses your choice of model. Works within the boundaries you set.

[Get started](#get-started) · [What you can do](#put-it-to-work) · [Features](#built-for-work-that-continues) · [Documentation](docs/README.md) · [Website](https://vakyartha.com)

[![Version](https://img.shields.io/badge/version-5.2.13-E66A2C?style=flat-square)](CHANGELOG.md)
[![Rust](https://img.shields.io/badge/Rust-2024-2B2B2B?style=flat-square&logo=rust)](Cargo.toml)
[![License](https://img.shields.io/badge/license-MIT-536B58?style=flat-square)](LICENSE)

</div>

![The Vakyartha Songbird and character family together in a sunlit room](docs/brand/library/wallpapers/ensemble-day-1920x1080.jpg)

Work rarely fits inside one chat response. A question leads to research. Research becomes a document. A proposed fix needs a test, a diff, and a decision.

Vakyartha brings those steps into one workspace. Give an agent a task, let it work with your files and connected tools, and inspect the result. Steer it as it goes, review a draft, or return to a conversation and pick up where you left off.

## Put it to work

Start with something you already need done:

| Bring a task | Work toward a useful result |
|---|---|
| **“Find out why these tests fail.”** | Explore the code, run checks, and review a proposed fix. |
| **“Review this report and tighten the summary.”** | Read Word or PDF files, follow citations back to the source, and inspect proposed document changes before accepting them. |
| **“Compare these options using the attached material.”** | Turn research into a comparison with sources you can check. Connect web tools when the task needs fresh information. |
| **“Help me update this workbook or presentation.”** | Work with Excel and PowerPoint files through drafts and document review. |
| **“Make this a recurring check.”** | Set up a scheduled task for an agent, with results and requests for attention delivered to its configured destination. |

The model and tools you connect determine what an agent can do. Start with one workspace and one task; add specialist agents, integrations, and routines as you need them.

## Why Vakyartha

### Work you can inspect

Code diffs, document drafts, previews, and source citations give you something concrete to review. Behind the conversation, an append-only record preserves model inputs, tool results, and decisions so you can trace how the agent reached an answer. Cancelling a run preserves its partial output.

### Boundaries you choose

Choose read-only exploration, work inside a selected workspace, or explicitly grant full access. Tool permissions are checked before execution, and restricted tools run in isolated workers. Provider keys and bot tokens live in a secret store. Connected chats have their own admission and approval controls.

[Read the security model →](docs/design/24-agent-security.md)

### Your models, your tools

Connect Anthropic, OpenAI, Google Gemini, OpenRouter, or a compatible endpoint—or use a local model through Ollama. Vakyartha discovers the models available to your connection. Your files and conversation history stay under your control; requests go to the model provider and tools you choose.

Add **skills** for domain knowledge, **MCP servers and plugins** for integrations, **hooks** for lifecycle events, and **flows** for repeatable work. Vakyartha is built in Rust around a shared agent core, with specialized capabilities added through extensions.

[Explore extensions →](docs/design/09-extensibility.md)

### Agents you can return to

Create specialists with their own instructions, conversations, workspaces, and schedules. Reach them from the desktop, browser, terminal, or a connected chat. Each agent owns its ongoing work, so you have a place to return to when a task spans more than one sitting.

## Built for work that continues

**Carry context forward.** Agents can search past conversations and keep workspace notes, so useful context can survive a session. You can inspect and edit those notes. Reusable skills proposed from experience go through human review before becoming available for future work. [Memory and recall](docs/design/23-memory.md) · [Learning and skill review](docs/design/26-learning.md)

**Give longer work a finish line.** Use `vak plan` to break an open-ended task into executable steps, or goal mode to track an objective against acceptance criteria. Completion is checked against evidence, with failed and unverified outcomes kept visible. [Planning](docs/design/11-planner.md) · [Goals and verification](docs/design/42-managed-work-contracts.md)

**Review the actual deliverable.** Read, cite, and revise Word, Excel, PowerPoint, and PDF files. Inspect proposed changes in the workspace, ask for revisions, and accept a reviewed draft. Charts, tables, and previews help make results easier to examine. [Office documents](docs/design/72-openxml-documents.md) · [PDFs](docs/design/77-pdf-documents.md)

**Keep recurring work moving.** Assign schedules to agents and connect Telegram, Discord, or Slack for delivery. Review runs, approvals, and requests for attention from the management console. Configured voice support adds transcription of channel voice notes and spoken replies. [Agents and scheduled work](docs/design/64-agent-owned-platform.md) · [Voice support and current limits](docs/design/49-live-voice.md)

**Stay in control of time, cost, and changes.** Set spend limits, steer or cancel active work, and inspect provider attempts when something fails. Workspace checkpoints let you restore captured files. [Reliability](docs/design/15-reliability.md) · [Checkpoints](docs/design/14-checkpoints.md)

## Work where you are

| Surface | Use it for |
|---|---|
| **Desktop** | A workspace with conversation, files, previews, diff review, approvals, and a terminal. |
| **Browser** | The shared workspace in your browser, with a separate console for settings and operations. |
| **Terminal & CLI** | Interactive conversation with `vak term`, or individual requests and scripts with `vak exec`. |
| **Telegram, Discord & Slack** | Optional chat connections for reaching an agent and receiving its work. |

![The CLI, desktop, browser, and channels meet at one governed core](docs/assets/vak-surfaces.webp)

## Get started

Vakyartha runs on **macOS and Linux**. Bring a provider key or a local Ollama model; a hosted model is not included. The command is **`vak`**.

### 1. Install from source

You need Git, a stable Rust toolchain, Node.js/npm, and Python 3.

```bash
git clone https://github.com/nranjan2code/code.git
cd code
scripts/build.sh
```

The script builds and installs Vakyartha. Follow its printed `PATH` instruction if needed. For a headless build, use `scripts/build.sh --no-desktop`.

Prefer a prebuilt app? Check [releases](https://github.com/nranjan2code/code/releases) for your platform and version; published binaries may lag `main`. See the [installation guide](docs/release-and-install.md) for details, including the first launch on macOS.

### 2. Connect a model and choose a workspace

```bash
vak setup
```

Setup walks you through the workspace, provider, model, and permissions. On a headless machine, use `vak setup --terminal`. Background services activate only when you choose to enable them.

### 3. Give it a task

Open the desktop app, or start a terminal conversation from your chosen workspace:

```bash
vak term
```

Try: **“Explain this project and suggest a useful first improvement.”**

For a single request:

```bash
vak exec "Explain this project and suggest a useful first improvement"
```

<details>
<summary><strong>Prefer the browser?</strong></summary>

Start the server:

```bash
vak serve --port 8901
```

Then, in another terminal, open the workspace:

```bash
vak open app
```

The server listens on loopback by default. `vak open admin` opens the management console. For remote access, follow the [hosting guide](docs/hosting.md).

</details>

If setup needs attention, run `vak setup status` or `vak doctor`. For command options, use `vak --help`.

## Under the hood

![A person and the Vakyartha Songbird following an idea through planning, action, and review](docs/assets/vakyartha-hero.webp)

Every surface uses the same core: a streaming agent loop, persistent sessions, permission checks, and brokered tool execution. The interface gives you the conversation and review; the underlying record keeps the evidence. Transient provider failures have bounded retries, and work can be steered or cancelled while it runs.

Explore the [agent loop](docs/design/03-agent-loop.md), [session record](docs/design/02-sessions.md), and [agent ownership model](docs/design/64-agent-owned-platform.md), or start with the [documentation guide](docs/README.md).

## Go further

- **Configure your workspace:** [Settings, providers, and secrets](docs/design/05-config.md) · [Permissions](docs/design/08-permissions.md)
- **Run an always-on agent:** [Hosting and services](docs/hosting.md) · [Channels](docs/design/34-channel-onboarding.md) · [Administration](docs/design/33-admin-console.md)
- **Build on Vakyartha:** [Development guide](docs/development.md) · [Extensions](docs/design/09-extensibility.md) · [Flows](docs/design/10-flows.md) · [Engineering contract](AGENTS.md)
- **Follow the project:** [Release notes](CHANGELOG.md) · [Brand and artwork](docs/brand/README.md)

Design documents declare their own **Status**; some describe proposals or historical decisions.

## License and maintenance

The software and original documentation are [MIT licensed](LICENSE). The Vakyartha name, Songbird mark, character artwork, and wallpapers have [separate brand terms](docs/brand/README.md). Third-party components retain their own license notices.

Maintained by [@nranjan2code](https://github.com/nranjan2code). External pull requests are not accepted.
