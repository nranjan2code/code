# 08 — Permissions (vak-permission)

## Model

Decisions are decoupled from enforcement and from UI:

```
evaluate(tool, args, mode, cwd) -> Allow | Ask{reason} | Deny{reason}
```

The agent loop gates every tool call before dispatch. `Ask` is resolved by an
`Approver` (trait): TUI prompts y/n interactively; `exec` uses `--yes`
(AutoApprove) or defaults to AutoDeny with the reason fed back to the model as
an error tool result — the model can adapt instead of crashing.

## Rules

Config layers merge three lists (`deny`, `ask`, `allow`; deny wins by order):

```toml
permission_mode = "workspace-write"
allow = ["Bash(git *)", "Bash(cargo *)"]
ask   = ["Edit(~/**)"]
deny  = ["Bash(rm -rf *)", "Bash(sudo *)"]
```

Syntax per rule: optional prefix (`+` allow, `?` ask, `-` deny; bare =
allow), tool name (case-insensitive), optional glob on argument candidates:
- bash: full command + each subcommand split on `&& ; |`, leading `VAR=value`
  assignments stripped
- file tools: the path argument

Matching rules aggregate by severity (`Deny > Ask > Allow`). No match → mode
default:
- read-only: workspace-scoped read/glob/grep/ls/search allowed, everything
  else denied
- workspace-write: workspace-scoped reads and writes allowed (canonicalized,
  symlink- and `..`-aware); bash and outside-workspace writes ask; reads
  outside the workspace are denied
- full-access: everything allowed

## Invariants

- A denied call never executes; its reason becomes an `is_error` tool result.
- Approval requests are answered exactly once (oneshot channel in the TUI).
- The engine is pure: no I/O beyond path canonicalization.

## OS sandbox layer

Enforcement is defense-in-depth: rules decide *whether* to run; the sandbox
constrains *what the process can touch* even when allowed.

- `vak-tools/src/broker.rs`: all production built-in tools use a bounded,
  versioned stdin/stdout protocol to a disposable `__tool_worker` process.
  Permission and approval stay in the broker. The worker receives one tool
  name plus arguments, a scrubbed environment, workspace cwd, cancellation by
  process group, and no provider/session/control-plane handles.
- `vak-tools/src/sandbox.rs`: `Sandbox` trait (`wrap(command) -> command`).
  `Seatbelt` wraps the entire worker command, not only Bash, in
  `sandbox-exec -p '<profile>' -- sh -c '…'`.
- Profiles deny-by-default: process exec/fork remains available, while file
  contents are readable only below explicit OS, canonical-workspace,
  executable, toolchain, and temp roots. Global metadata traversal is allowed
  so permitted descendants can resolve, but arbitrary home/volume contents are
  not.
  `WorkspaceWrite` adds file-write under the **canonicalized** cwd plus
  `/private/tmp`, `/private/var/tmp`, `/dev/null`, `/dev/urandom`.
  `ReadOnly` grants no write paths at all.
- The optional Docker backend is command-scoped: the broker rewrites the
  validated Bash command inside its worker request, and that shell command runs
  with no network, a read-only root, bounded tmpfs, CPU/memory/PID limits,
  dropped capabilities, and `no-new-privileges`. File tools still use local
  workers; a pinned Linux worker image is required before the full protocol can
  move into the container. See `25-docker-sandbox.md`.
- Derived automatically from the effective permission mode
  (`full-access` ⇒ off); visible via `vak config dump`
  (`sandbox = seatbelt | landlock | off`).
- Linux backend (`vak-tools/src/landlock.rs`): Landlock LSM (kernel 5.13+)
  via the safe `landlock` crate — reads+execute are limited to explicit OS,
  workspace, toolchain, executable, and temp roots; writes are limited to the
  canonicalized cwd and temp in workspace-write mode. `wrap()` re-executes the
  vak binary with a hidden `__sandbox` subcommand that applies the
  ruleset to itself before running the command, so children inherit it.
  The probe is never applied to the long-lived broker. Unsupported enforcement
  makes the disposable worker exit 126; it never falls back to host execution.
- Network scoping: every sandboxed mode denies all TCP bind/connect
  (Landlock ABI v4 on Linux; Seatbelt's deny-default profile already does
  this implicitly on macOS) and the Landlock runner refuses to run when the
  kernel can't enforce the denial (fail-closed enforcement check).
  FullAccess runs unsandboxed and keeps network access.
- Bash subprocesses inherit only a small operational environment allowlist
  (`PATH`, locale, terminal, temp, user/home, XDG, and Rust toolchain paths).
  Provider keys, gateway tokens, and arbitrary host secrets are not inherited,
  including in full-access mode. Future credential use must be explicit and
  target-scoped rather than ambient.
- Configured MCP servers are separately spawned, sandboxed process-group
  workers. Their environment is empty except for `PATH` and variables
  explicitly declared in that server's trusted configuration. Restricted MCP
  servers have no network because they inherit the same sandbox policy.
- `task` and `session_search` are broker-owned structured capabilities rather
  than worker code: child agents receive brokered tool registries, while
  session search receives only its fixed session-index scope.
- Static and dynamically planned Bash flow nodes evaluate the same permission
  engine and approver before dispatching through the brokered registry. A flow
  cannot treat a model-generated command as implicitly approved.
- Changing permission mode through the server revokes every in-flight main and
  side run and rejects pending approvals before the new mode is reported. A
  running agent never continues with a stale, more-permissive snapshot.
- Learned allow rules: pressing `[p]` on an approval persists a SCOPED rule
  derived from the call — `bash(<first-word> *)`, `<write|edit>(<path>)`,
  `mcp(<server>/*)`, `task(<label>)` — into
  `.vak/permissions.local.toml` (trusted workspaces only). Every spec
  is round-trip validated (must parse AND match the triggering call) before
  it is written. Loaded at Core startup for trusted workspaces and merged
  into every engine build (`exec`/`plan` included); because evaluation is
  severity-aggregated, a learned Allow can never shadow an explicit Deny.
  `[a]` remains session-only for calls that cannot be scoped safely
  (e.g. opaque bash with command substitution).
- Known semantics: on macOS `/tmp` resolves to `/private/tmp`, so tmp writes
  are permitted in workspace-write mode by design (output spill files rely on
  it). Everything else outside the cwd is blocked at kernel level.

## Later

- A pinned full-worker container image and VM/remote execution remain the
  stronger hostile-code backends. The local worker plus Seatbelt/Landlock path
  is the default low-friction boundary; today's Docker backend contains Bash.

See `24-agent-security.md` for the adversarial threat model, comparative
research, residual risks, and the containment roadmap.

## Diff note — severity aggregation + opaque commands (this change)

`evaluate` aggregates matching rules by severity (Deny > Ask > Allow) instead
of first-match-wins, so deny precedence is a property of the engine rather
than of rule insertion order. Bash commands containing newlines, backticks,
or `$()`/`<()`/`>()` produce NO arg candidates: pattern-based allow rules
cannot see inside them, so they fall through to the mode default / approver,
which sees the full command. Blanket (patternless) rules still apply. MCP
calls expose `server/tool` candidates (`Mcp(docs/*)`), tasks expose their
label, and Ask reasons for mcp/task include the resolved target instead of
approving blind.
