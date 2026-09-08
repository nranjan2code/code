# Sandbox system audit — 2026-09-08

## Follow-up after greenfield slice

The original findings below are the baseline reproduction. The new
`vak-sandbox` crate now owns environment and promotion contracts; brokered
workers carry execution IDs and replay structured sandbox events to the parent,
and every Bash process gets its own process group so timeout cleanup reaches
descendants. The remaining findings are deliberately retained as acceptance
criteria for the next implementation slices rather than erased from the audit
record.

Status: baseline audit retained as historical evidence; the greenfield runtime
and Workbench follow-up are implemented and acceptance-tested. Findings that
remain relevant are tracked as residual limitations below rather than being
treated as unverified claims.

Follow-up: the research-backed universal workflow and explicit preservation of vak's existing harness are proposed in `docs/design/54-task-environments-and-promotion.md`. It extends this audit from execution visibility into task preparation, multimodal outcomes, review, and controlled acceptance.

**Historical verdict:** the containment primitives were real, but the advertised
observable execution system was not connected end to end. The follow-up now
connects broker events, execution identity, cancellation, artifact ownership,
persistence, and Workbench hydration. The remaining acceptance limitation is
model-generated output quality: runtime receipts must still be checked against
the actual filesystem and browser result before an outcome is presented as
verified.

Scope: current working-tree Rust execution path, permission engine, native/Docker backends, SSE delivery, client store, Workbench and artifact APIs. Existing Dockerfile/Compose edits were inspected as current state and left untouched. This is a source audit with disposable local reproductions, not a certification of every tool, backend, or deployed UI. No actual user credentials were read. No Docker daemon or browser session was exercised.

## How it works today

1. Core constructs brokered built-ins (`crates/vak-core/src/lib.rs:1158`). Permission rules resolve before dispatch; workspace-write Bash asks by default, read-only denies Bash by default, and explicit rules can affect those decisions (`crates/vak-permission/src/engine.rs:126`). Approval and containment are separate.
2. Core chooses a containment backend. FullAccess returns no sandbox; native macOS uses Seatbelt, Linux uses Landlock, and Docker is explicitly selectable (`crates/vak-core/src/lib.rs:5606`). Docker sessions use a retained task container (`:5657`, `:5679`). Unsupported restricted execution gets a deny backend.
3. The broker sends a versioned request to a disposable worker. Native backends wrap the worker process. Docker wraps only the Bash command; file tools remain on the host and depend on permission/path checks (`crates/vak-tools/src/broker.rs:232`).
4. Bash runs `sh -c` in the workspace, clears the environment and restores an operational allowlist. It creates `.vak/scratch`, reads stdout/stderr, and optionally emits events. After exit it scans the top level of scratch for changed files (`crates/vak-tools/src/bash.rs:44`).
5. The agent creates an event sink and a forwarder, but the worker recreates ToolContext without that sink. Its response contains only `version`, `content`, and `is_error`. The broker waits for process completion before parsing that response (`crates/vak-tools/src/broker.rs:25`, `:127`, `:198`).
6. If Sandbox events reach the agent, they can travel through SSE. The server retains a 1,024-event in-memory replay ring (`crates/vak-server/src/events.rs:338`). Workbench consumes only Sandbox events into a global client signal (`crates/vak-client-ui/src/store.ts:130`, `:850`). It does not derive execution records from ordinary Bash tool results.

## Prioritized findings

Severity: P1 = major correctness/security issue; P2 = significant usability or diagnostic gap. Related consequences are grouped; there are nine P1 and three P2 findings below. No P0 emergency or confirmed remote exploit is claimed.

### P1 — Live execution events never cross the production worker boundary

**Evidence:** `broker.rs:25–29`, `:127`, `:198–199`; `context.rs:35–42`; agent sink creation at `vak-agent/src/lib.rs:3486`.

The worker receives no event channel and builds `ToolContext::new(cwd)`, whose `sandbox_sink` is None. WorkerResponse has no event variants. The caller's sink is unused. A real compiled broker/worker reproduction returned successful Bash output and **zero sandbox events**.

**User impact:** no Workbench execution row, stdout, stderr, telemetry, package events, or artifacts, although the tool completes in chat. The chat's “Inspect in Workbench” button cannot reconstruct what is missing.

**Fix:** versioned streaming worker frames with start/output/artifact/finish and final result; parent validates and relays them. Use bounded framing and broker-assigned identity. Keep tool effects behind the existing broker. Drain the event forwarder on completion: the current unconditional `forwarder.abort()` at `vak-agent/src/lib.rs:3505` can also discard queued events once streaming is restored.

### P1 — Viewing a different execution changes where new output is written

**Evidence:** `store.ts:850–940`; `WorkbenchPanel.tsx:311`.

Events lack execution IDs. Every output/status update goes to `activeExecutionId`, which is also changed by clicking a historical row. The reproduction started A, finished A, started B, selected A, and delivered B's stdout: it was appended to A. Global records also lack session/workspace/tool-call ownership. Chat inspection does not pass a tool-call identity (`ChatPane.tsx:282`).

**Fix:** every event carries broker-assigned workspace/session/run/tool-call/execution identity and sequence. Store by identity; keep UI selection separate. Scope the panel to the selected task and link each chat tool card to its exact execution. This is required even if Bash calls within one run are serialized.

### P1 — Brokered command timeout does not terminate the intended process group

**Evidence:** `bash.rs:65–67`, `:136–146`, `:283`; `broker.rs:100–102`.

The broker establishes the worker's group. Bash deliberately does not create a new group in a worker, then timeout attempts `kill(-child_pid)`, although the shell belongs to the worker's group. The kill result is ignored and the code awaits the shell. **A real broker command with a 1,000 ms timeout and `sleep 3` returned after 3,027 ms**, reporting “timed out after 1000ms.”

**Fix:** broker-owned execution cancellation with correct group ownership, TERM/KILL escalation and verified termination. For Docker, implement cancellation inside the container; killing a host `docker exec` client is not an adequate process-lifecycle contract. Container cancellation remains an untested risk in this audit.

### P1 — Cancellation and timeout discard partial output

**Evidence:** `bash.rs:136–170`; `broker.rs:143–144`.

Bash returns only an error on timeout/cancel without collecting stdout/stderr readers; broker cancellation returns “tool broker cancelled” and discards buffered response bytes. The fixture printed a marker before sleeping; the timeout result lost it. With the broken event bridge there is no alternate user-visible record.

**Fix:** persist output as it arrives, terminate the execution, drain readers with a deadline, and append a terminal state referencing retained partial logs/artifacts. Distinguish failed, timed out, cancelled, and lost-worker states.

### P1 — Scratch quarantine is a convention, not an enforced execution boundary

**Evidence:** `bash.rs:59`, `:69–71`; `sandbox.rs:128–131`, `:175–186`; `sandbox_docker.rs:54–57`.

Bash uses the workspace as cwd. Creating scratch does not redirect HOME/TMPDIR/package caches/build paths or constrain writes to scratch. Native workspace-write grants the full workspace and broad temporary directories; Docker mounts the full workspace read-write. The fixture's relative output was created in workspace root. Docker's writable container root layer is another storage domain, distinct from scratch.

**Fix:** first define the product contract. Source edits legitimately belong in a coding workspace, while generated runtime state belongs in per-task scratch. Separate those writable roots and provide explicit artifact promotion. Set runtime/cache/temp paths and enforce mounts/OS rules. A default cwd alone is not enforcement. Rewrite invariant 35's absolute claims to match the chosen enforceable contract.

### P1 — Workspace boundaries include the very control files and secrets they should protect

**Evidence:** whole-workspace Seatbelt allowances in `sandbox.rs:98`, `:128–131`; whole-workspace Docker mount in `sandbox_docker.rs:54–57`; default file permissions in `vak-permission/src/engine.rs:193–210`.

No mandatory exclusion for workspace `.env` or `.vak/config.toml` exists in these boundaries. A native Seatbelt fixture successfully read a **fake** workspace `.env` and changed a fake `.vak/config.toml`. Scrubbing environment variables does not prevent filesystem reads. This is especially consequential when the workspace is the canonical user home containing shared configuration and credentials.

This establishes readable/writable paths, not a demonstrated complete privilege-escalation chain. Operator-specific deny rules may narrow access, but are not a universal floor and shell effects must still be contained.

**Fix:** protect control/credential storage structurally and in every filesystem/OS backend. Make scratch a narrowly writable exception beneath any protected `.vak` directory. Prefer a container workspace projection that omits control files; privileged mutations must use authenticated control-plane operations. Do not rely on shell-string matching or prompts.

### P1 — Execution history is not durable or recoverable as Workbench state

**Evidence:** `store.ts:112–131`, `:634`, `:850`; `vak-server/src/events.rs:338–405`; `sandbox_events.rs:15`.

Workbench is an in-memory global signal. Sandbox events contain neither full state snapshots nor durable execution identity. Short SSE reconnects can replay the server ring, which is a useful existing feature; server restart, ring overflow, or transcript-only hydration cannot reconstruct this execution model. UI-generated random IDs are also unsuitable for replay deduplication. Clearing history during a run deletes its destination for subsequent chunks.

**Fix:** append durable execution records and bounded log references, expose a session-scoped snapshot/history endpoint, and reconcile after gaps/reload. Sequence/deduplicate deltas against snapshots. “Clear” should hide/filter records rather than destroy the active projection. Existing ledgers remain append-only; old records should report unavailable execution detail rather than invent it.

### P1 — Artifact discovery and opening miss common user outcomes

**Evidence:** `bash.rs:176`, `:358–405`; `WorkbenchPanel.tsx:202`, `:499–506`, `:520`; `api.ts:785`; `vak-server/src/lib.rs:6689`.

Discovery only runs at exit, only scans immediate scratch children, and compares mtimes. Nested outputs, container-only files, and files outside scratch are absent. The fixture emitted `.vak/scratch/result.txt` but not `.vak/scratch/nested/result.txt`. Timeout/cancel skips discovery. Direct file-tool outputs do not participate in this Bash scanner.

Clicking an artifact under the Execution tab fetches content but does **not** switch to the Artifacts tab, where the viewer lives. It can appear to do nothing. A path-only API reads current workspace bytes, not the historical artifact version; container-only files are inaccessible. Binary responses have no content/data URL, and this viewer reports inability to read them rather than providing a download action.

**Fix:** explicit artifact registration with execution identity, durable content or digest/version, safe recursive export, MIME/size metadata, and preview/download/open actions. Import from backend storage before teardown. Register incrementally and on interruption. Clicking an artifact must navigate to the visible viewer. Use scoped artifact IDs, not mutable paths as identity.

### P1 — UTF-8 preview truncation can panic

**Evidence:** `sandbox_events.rs:85–86`.

`&code[..2000]` slices at an arbitrary byte boundary. A command with 1,999 ASCII bytes followed by `€` panicked in the disposable test. The currently broken worker event path masks this for brokered Bash; restoring events exposes it.

**Fix:** truncate at a UTF-8 boundary and add multibyte regression cases. Event formatting must not crash execution.

### P2 — Workbench presents inferred or unrelated state as execution truth

**Evidence:** `bash.rs:109`, `:333–355`, `:439`; `WorkbenchPanel.tsx:194–200`, `:241`; `Settings.tsx:1048`.

CPU is always emitted as zero. Memory probes one host PID and returns zero on failure; Docker telemetry would describe the CLI, not the container workload. “Installed packages” is inferred from command words plus exit success, rather than an environment inventory. Stop cancels the currently selected session, not the displayed execution, and errors are swallowed. Every run is labelled “Sandbox,” even though FullAccess is unsandboxed. Settings assigns containment the `good` class unconditionally.

**Fix:** show actual backend/effective policy on each execution; represent unknown metrics as unavailable with reasons; use backend workload counters and attested package manifests. Stop must target the displayed execution and report acknowledgement/termination failure. FullAccess must visibly say unsandboxed.

### P2 — Output transport/rendering lacks a bounded continuous-log contract

**Evidence:** `bash.rs:299–329`; `sandbox_events.rs:69`; `WorkbenchPanel.tsx:470–483`; `store.ts:873–880`.

The reader stops consuming at 1 MiB rather than continuing to drain while limiting retained data. That can alter a noisy command's behavior through broken pipes or blocked writers. The event sink is unbounded. The client repeatedly copies and parses accumulated strings; stdout and stderr are displayed in separate blocks, losing arrival order. These become materially worse once the live bridge works.

**Fix:** continuously drain bounded frames to durable log chunks; keep a bounded UI tail with “load earlier.” Explicitly represent truncation. Preserve observed stream sequence, apply backpressure safely, and separate model-result truncation from the user's inspectable execution log.

### P2 — Narrow-panel, theme, and accessible-state handling need follow-through

**Evidence:** `styles.css:3457`, `:3783`, `:3805`; `WorkbenchPanel.tsx:254–270`, `:46–65`, `:576`.

Fixed 220/240px sidebars leave little room in a narrow dock; no Workbench-specific responsive override was found. Tab selection is communicated through CSS without selected-state semantics. ANSI colors and hover whites are hardcoded; the HTML viewer is forced white. Automatic scrolling follows every update, including when the reader wants earlier output.

**Fix:** responsive master/detail views, selected-state semantics, theme-aware terminal tokens and opt-in follow-tail. Keep preview isolation. The ANSI renderer escapes text before inserting markup, and the HTML iframe uses `sandbox="allow-scripts"`, both worth preserving. Multi-file HTML previews need a defined asset-serving policy; srcdoc alone does not provide a packaged application runtime.

## UI quality check (source-only, provisional)

| Dimension | Score / 4 | Evidence |
|---|---:|---|
| Accessibility | 2 | Native buttons, titles and image alt text exist; selected tab/run states lack semantics. No rendered WCAG assessment. |
| Performance | 1 | Full accumulated-log parsing/copying; unbounded records and sink. |
| Responsive layout | 1 | Fixed inner sidebars without a narrow-panel alternative. |
| Theming | 2 | Shared tokens plus fixed ANSI/hover/preview colors. |
| Implementation integrity | 1 | UI promises cannot be satisfied by the production worker contract. |
| Total | 7 / 20 | Poor; provisional code assessment, not a visual or accessibility certification. |

The bundled Impeccable detector emitted no findings for the targeted TSX file. That does not validate the runtime path; the findings above were checked directly. No generic dashboard redesign is needed to solve the primary issue.

## Greenfield design I would choose as a user

The default experience should answer: **what is running, where, with what access, what did it produce, and can I stop it?** Those answers should survive closing the UI.

```text
Task + admitted policy
  -> broker creates execution ID and durable start record
  -> backend starts a scoped runtime and reports actual capabilities
  -> worker streams bounded, sequenced events through the broker
  -> append-only execution/log/artifact storage
  -> session snapshot + resumable events
  -> inline execution card and the same record in Workbench
```

1. **One execution contract.** A backend supports start, stream, cancel, wait, inspect, artifact export, and cleanup. The existing `Sandbox::wrap` can remain an implementation detail during transition but cannot express this lifecycle alone. Backend-specific behavior lives behind adapters, not extra top-level model tools.
2. **Separate permission from environment.** Permission defines allowed effects. Backend defines enforcement. Runtime describes packages, storage and resource limits. The UI displays their effective combination and its source. An approval never disables containment; FullAccess remains an explicit human choice.
3. **Usable, isolated task storage.** Keep source workspace edits available when authorized; put virtual environments, caches, generated intermediates and outputs under a task-owned scratch root or isolated container volume. Protect credentials and control files. Offer an explicit promotion path for deliverables. State what persists across commands, turns, UI reloads and daemon restarts.
4. **Packages without FullAccess pressure.** Offer curated prepared environments and a brokered dependency acquisition workflow with scoped network policy and receipts. A default Alpine image with network disabled does not by itself provide a general-purpose Python/Node environment. Installation approval should authorize that bounded operation rather than require abandoning all containment.
5. **One visible execution record.** Chat shows a compact live card: command summary, actual cwd/backend, status, elapsed time, latest output, and artifact chips. Selecting it opens its exact Workbench record. Workbench contains Output, Files, and Environment views of the same execution. UI selection never controls event routing.
6. **Honest empty/error states.** Distinguish no commands, preparing runtime, waiting for approval, sandbox unavailable, disconnected, replaying, and history unavailable. Do not show “No Sandbox Executions Yet” after known Bash calls solely because telemetry is absent. A failed event connection should be diagnostic evidence, not an apparently empty task.
7. **Discoverable outcomes.** Surface images/documents/data/apps in chat as artifacts appear. Provide preview, download, save/promote, and provenance. HTML assets use an isolated preview origin and explicit content/network policy; a shell sandbox and an iframe sandbox are different boundaries.

## Implementation order and acceptance gates

**First: truthful execution and safety.** Protect control files, repair group cancellation and partial-output retention; implement framed worker events and stable identities together. Add UTF-8-safe preview handling. Do not bypass the broker to make the UI light up.

**Second: durable observation.** Add execution storage/snapshots, scoped reducers and artifact export. Reconnect must reconcile server truth rather than guess from the selected row. Drain forwarding tasks and bound output.

**Third: user-facing integration.** Inline execution cards, exact inspection links, working artifact navigation, scoped Stop, actual backend labels and actionable runtime errors. Use `$impeccable harden` and `$impeccable clarify` for those states, `$impeccable adapt` for narrow panels, and `$impeccable polish` only after the execution contract passes.

Release acceptance should exercise the actual worker, agent, server, store, and UI:

- Slow command displays a row and output before exit; no provider credentials enter its environment.
- Viewing execution A while B runs cannot change where B's output or finish lands; parallel sessions remain separate.
- UI reload, SSE gap beyond ring capacity, and daemon restart recover the same records or explicitly mark lost executions.
- Cancel and timeout finish within a bounded deadline, retain prior output/artifacts, and verify descendant termination on native and Docker backends.
- Workspace-local secrets/control files remain protected; approved source edits still work; runtime caches remain quarantined.
- Nested and interrupted artifacts can be previewed/downloaded; clicking from Execution opens the viewer; historical versions remain stable.
- Unicode and large-output commands do not panic, deadlock, exhaust the UI, or silently corrupt logs.
- FullAccess, unavailable sandbox, pending approval, missing package and unavailable telemetry are visibly different states.

## Verification performed

- `cargo test -p vak-tools --lib sandbox -- --nocapture`: 14 passed.
- `cargo test -p vak-tools --test sandbox`: 11 passed, including native Seatbelt containment tests.
- Four disposable Rust reproductions passed **by asserting the defects**: workspace-root output/nested-artifact omission, partial-output loss, Unicode panic, and fixture-only workspace control-file exposure.
- A compiled probe using the actual `brokered_default_tools` and `worker_main` confirmed successful Bash with zero events, plus the 3,027 ms timeout overrun.
- The exact Sandbox reducer body extracted from `store.ts` reproduced selection-driven output misattribution in Node.
- Reproduction Rust sources retained in `docs/research/sandbox-audit-2026-09-08/`; temporary integration-test registration and compiled probe files removed after use. No production code changed.

Remaining validation: Linux runtime behavior, live Docker lifecycle and platform support for its storage flags, MCP containment, preview network policy, and rendered desktop/mobile behavior. These are explicitly unverified, not assumed to pass.
