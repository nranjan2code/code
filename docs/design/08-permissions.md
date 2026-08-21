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

First matching rule wins. No match → mode default:
- read-only: read/glob/grep/ls/search allowed, everything else denied
- workspace-write: reads allowed; write/edit allowed inside cwd (canonicalized,
  `..`-aware); bash and outside-workspace writes ask
- full-access: everything allowed

## Invariants

- A denied call never executes; its reason becomes an `is_error` tool result.
- Approval requests are answered exactly once (oneshot channel in the TUI).
- The engine is pure: no I/O beyond path canonicalization.

## OS sandbox layer

Enforcement is defense-in-depth: rules decide *whether* to run; the sandbox
constrains *what the process can touch* even when allowed.

- `vak-tools/src/sandbox.rs`: `Sandbox` trait (`wrap(command) -> command`).
  `Seatbelt` backend wraps bash commands in `sandbox-exec -p '<profile>' -- sh -c '…'`.
- Profiles deny-by-default: reads + process exec/fork always granted;
  `WorkspaceWrite` adds file-write under the **canonicalized** cwd plus
  `/private/tmp`, `/private/var/tmp`, `/dev/null`, `/dev/urandom`.
  `ReadOnly` grants no write paths at all.
- Derived automatically from the effective permission mode
  (`full-access` ⇒ off); visible via `vakcoder config dump`
  (`sandbox = seatbelt | off`). macOS only today — Landlock/seccomp backend
  planned for Linux behind the same trait.
- Known semantics: on macOS `/tmp` resolves to `/private/tmp`, so tmp writes
  are permitted in workspace-write mode by design (output spill files rely on
  it). Everything else outside the cwd is blocked at kernel level.

## Later

- session-scoped "always allow" learned rules persisted to project config
- OS sandbox backends (Seatbelt/Landlock) as a second enforcement layer under
  the same Decision vocabulary

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
