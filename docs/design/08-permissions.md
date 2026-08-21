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

## Later

- session-scoped "always allow" learned rules persisted to project config
- OS sandbox backends (Seatbelt/Landlock) as a second enforcement layer under
  the same Decision vocabulary
