# 08 — Permissions (vak-permission)
Status: implemented in 2.0.0

## Model

Decisions are decoupled from enforcement and from UI:

```
evaluate(tool, args, mode, cwd) -> Allow | Ask{reason} | Deny{reason}
```

The agent loop gates every tool call before dispatch. `Ask` is resolved by an
`Approver` (trait). Four implementations ship, and they are the complete set:

| Approver | Surface | `answerable()` |
|---|---|---|
| `AutoApprove` | `vak exec --yes`, `vak plan --yes`, evals | true |
| `AutoDeny` | CLI without `--yes`, the heartbeat, a chat gateway not in forward mode | false |
| `HttpApprover` | desktop app, admin console, and — unattended — scheduled tasks and best-of-N | matches whether a client is watching |
| `GatewayApprover` | a chat gateway in forward mode | true only in forward mode |

There is no interactive terminal approver. `vak exec` and `vak plan` are
one-shot: `--yes` installs `AutoApprove`, its absence installs `AutoDeny`, and
a denied call's reason is fed back to the model as an error tool result so it
can adapt instead of crashing.

`Approver::answerable()` is the second half of the trait and matters as much
as `approve()`: it says whether a gate raised here reaches anyone. An
unattended surface answers `false`, and `vak_core::reach` reads it *before*
the prompt is composed so a capability that can only ever be refused is
removed from the turn rather than discovered one denied call at a time.

A `Core` also carries `approver_answerable`, because the system prompt is
frozen at session creation and the approver does not exist until dispatch.
`Core::with_approver` derives it from the real approver wherever the host has
one; `with_approver_answerable` is the escape hatch for a host that knows
which approver it will build but cannot build it yet (the gateway). Either
way `run_turn_inner` reconciles the stamp against the installed approver, takes
the approver's word, and records an `answerability_mismatch` security event —
the stamp is a prediction, never an authority.

Permission mode and approval behavior are separate controls. Permission mode
sets the maximum execution boundary (`read-only`, `workspace-write`, or
`full-access`); the approver decides how an `Ask` is resolved. An automatic
approver must never replace the sandbox: it can approve work inside the
selected boundary, while broker and OS enforcement still constrain the
resulting process.

An invoking surface may also supply a run-scoped direct-write allowlist. For
CLI this is repeatable `--write-path <path>` on both `vak exec` and
`vak plan`; both also take `--permission-mode` for the run. `vak config
permissions` prints the mode, the approval mode, the rule lists and the
composed capability standings; `vak config set-mode` and
`vak config set-approval` persist to a chosen layer. It is not a model
instruction or a global setting: `write` and `edit` calls outside the declared
paths are denied before ordinary rules or permission mode are considered. This
is deliberately narrow: shell commands keep their normal sandbox and approval
semantics, so a caller needing filesystem confinement for arbitrary shell code
must use a restricted sandbox/worktree rather than infer shell effects from
text.

## Rules

Config layers merge three lists (`deny`, `ask`, `allow`; deny wins by order).
They are editable from `PUT /config/permissions` (the admin console's rule
editor and any local trusted surface), and every spec is parsed through
`vak_permission::Rule::parse` before anything is written — one bad rule
rejects the whole request rather than half-applying a set nobody chose:

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
- Approval requests are answered exactly once: the pending map holds a
  oneshot sender, and answering removes the entry before responding.
- Every gate is bounded. `GatewayApprover` waits `gateway.approval_timeout_secs`
  (default 300); `HttpApprover` waits 15 minutes. Both fail closed on expiry,
  and both drop their pending entry first so a late reply resolves nothing.
- The engine is pure: no I/O beyond path canonicalization.
- **Intent never gates the engine** (`docs/design/47-commitment-kernel.md`).
  A resolved engagement supplies a *ceiling* on approval permissiveness, and
  the effective mode is the stricter of that and the configured
  `ApprovalMode`. So an irreversible turn reaches a human even under
  `auto-approve`, and nothing the intent kernel concludes can skip a gate the
  operator asked for or turn a `Deny` into an `Ask`. A resolution bug can make
  vak more cautious; it cannot authorize anything.
- An **envelope** is pre-authorization *within* existing authority, never a
  grant of new authority: its `permission_ceiling` composes through the same
  `PermissionMode::capped_by` a gateway channel override uses, so it can only
  lower the effective mode. Revocation follows invariant 11.
- A gate nobody can answer still fails closed on a one-shot turn. Durable
  work instead **defers**: the commitment suspends on a `Human` wake
  condition and the question lands in the inbox. Nothing happens without the
  answer, but the work survives — a hard denial destroys month-long work that
  merely needed to wait.

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
  with no network, a disposable writable root in workspace-write mode or a
  read-only root in read-only mode, bounded tmpfs, CPU/memory/PID limits,
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
  side run, rejects pending approvals, and discards every pooled per-channel
  `Core` before the new mode is reported. A running agent never continues with
  a stale, more-permissive snapshot, and a warm channel never keeps a ceiling
  computed before the change.
- `[gateway] approvals` / `approver` decide whether an `Ask` on a chat surface
  reaches a human at all. Settable at runtime through
  `PUT /gateway/approvals` (persisted to the chosen config layer and applied
  live); `deny` clears the target rather than leaving a stale one for a later
  `forward` to reuse. `forward` with no `<surface>:<chat>` target is refused
  by the endpoint and degraded to `deny` by the loader — it is never
  representable in `GatewayState`.
- A per-channel mode is folded as chat pin → capped by bot pin → capped by the
  workspace. `gateway::resolve_channel_permission` folds the same three tiers
  for the admin projection, so what the console reports and what dispatch pins
  are the same computation.
- The server reads workspace trust from `vak_core::trust`, not by assumption.
  Approving or re-pointing a channel's workspace in the console records that
  decision in the same marker store the CLI reads.
- Learned allow rules: answering an approval with **"don't ask again"**
  (`POST /sessions/{id}/approvals/{req}` with `remember: true`; "Always allow
  this" in the desktop, "Approve, don't ask again" in the console) persists a
  SCOPED rule derived from the call — `+bash(<first-word> *)`,
  `+<write|edit>(<path>)`, `+mcp(<server>/*)`, `+task(<label>)` — into
  `.vak/permissions.local.toml` (trusted workspaces only). Every spec is
  round-trip validated (must parse AND match the triggering call) before it is
  written. Loaded at Core startup for trusted workspaces and merged into every
  engine build (`exec`/`plan` included); because evaluation is
  severity-aggregated, a learned Allow can never shadow an explicit Deny.

  `vak_core::scoped_allow_rule` returns `None` — approve again next time —
  for anything that cannot be narrowed safely: bash whose effects cannot be
  enumerated (command substitution, a redirection to a real path, an
  unbalanced quote, more than one segment), and network tools, whose one URL
  generalizes to nothing. A blanket `+webfetch` is a config decision made
  deliberately, not one that falls out of a single yes.
- Known semantics: on macOS `/tmp` resolves to `/private/tmp`, so tmp writes
  are permitted in workspace-write mode by design (output spill files rely on
  it). Everything else outside the cwd is blocked at kernel level.

## Later

- A pinned full-worker container image and VM/remote execution remain the
  stronger hostile-code backends. The local worker plus Seatbelt/Landlock path
  is the default low-friction boundary; today's Docker backend contains Bash.

See `24-agent-security.md` for the adversarial threat model, comparative
research, residual risks, and the containment roadmap.

See `../audits/permission-propagation-audit-2026-09-03.md` for how the
settings above became reachable from a surface. The engine described here was
correct throughout; what that audit repaired is the other half of the contract
— the controls that write a policy and the projections that report it.

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
