# Permission propagation audit — 2026-09-03

Status: historical audit of the reviewed permission surfaces.

## Contract

A permission answer has two halves. `vak_permission::PermissionEngine::evaluate`
decides `Allow | Ask | Deny`; the hosting surface then supplies an `Approver` to
resolve an `Ask`, and a control that lets an operator change the policy the
engine reads. The engine half was correct and stayed untouched. This audit is
about the second half: on most surfaces there was no control that *wrote* a
governing setting, and on several there was a control that *displayed* a value
the server never sent.

The invariant the fixes restore: **every setting the composed policy reads is
settable from a surface, and every value a surface shows is the one dispatch
uses.** A setting readable on three screens and writable nowhere is not a
policy — it is a fact about a file.

## Evidence that opened the audit

The installed instance's own audit log, `security-events.jsonl`:

```text
4×  capability_unreachable  capability=mcp server `tavily`
                            reason='mcp' needs approval: mcp call tavily,
                            and this surface has no approver to answer it
4×  capability_unreachable  capability=`webfetch`   … no approver to answer it
4×  capability_unreachable  capability=`browse`     … no approver to answer it
```

`~/vak-home/.vak/config.toml` configures the `tavily` MCP server and has no
`[gateway]` section, so `approvals` fell to its default of `"deny"`.
`GatewayApprover::answerable()` therefore returned false, every `Ask` on the
Telegram surface was a foregone denial, and `vak_core::reach` correctly stripped
all three capabilities from the turn. Every layer behaved as designed. The
defect was that the remedy `reach` prints — set `[gateway] approvals =
"forward"` — named an action no surface could perform.

## Findings and fixes

Ordered by severity. 1–3 were unreachable from every interface, 4–8 made an
interface report or apply something other than the truth, 9–13 were the shapes
that would have produced the next instance of 1–8.

| # | Previous risk | Current rule |
| --- | --- | --- |
| 1 | `gateway.approvals` / `approver` decided whether a chat gate reached a human, and had no writer: no route, no CLI verb, no UI. The console displayed it read-only. | `PUT /gateway/approvals` persists to a chosen layer and installs live. `forward` with no `<surface>:<chat>` target is refused with a reason, not silently degraded; returning to `deny` clears the target so a later `forward` cannot reuse it. `GET` returns approved chats as ready-to-pick addresses. Console panel reads and sets it. |
| 2 | `Core::learn_allow_rule` was built, tested, and had zero callers — the `[p]` keybind its comment referenced was never built. Rule lists had no writer either. | `Core::learn_from_call` derives the narrowest covering rule and is called by `POST /sessions/{id}/approvals/{req}` with `remember: true`; console and desktop both expose it. `PUT /config/permissions` edits the three lists, every spec parsed before anything is written. |
| 3 | `onboarding::permission_step` returned `Satisfied` unconditionally; the wizard renders actions only for an unsatisfied step, so the three-posture chooser was unreachable code. | The step reads the config layers as text and reports incomplete until one names a posture. It is deliberately **not** a term of `core_ready`: an unmade decision is worth asking about, not a reason to call a working install broken. |
| 4 | `/admin/api/config` omitted `approval_mode`, `sandbox` and `subagents` while the console's `ConfigInfo` declared all three, so TypeScript caught nothing. The approval picker never highlighted, the sandbox line loaded forever, and the sub-agents toggle wrote the inverse of what it showed. | All three are returned, and the rule lists read the live override rather than the config loaded at startup. The console labels a value as inherited rather than showing nothing selected. |
| 5 | The desktop had a permission-mode picker and nothing else — no approval mode, no sandbox, no rules. | `GET /config` returns the same three fields; the desktop Permissions page gained an Approvals group, a containment row, and the rule lists. |
| 6 | `resolve_channel_permission` capped the chat pin by the workspace only, ignoring the bot tier, and skipped resolution entirely when a chat inherited the gateway's workspace — so the console read **wider** than dispatch, or `null`. | It takes the bot pin and folds the same three tiers `core_for_entry` folds, falling back to the gateway's default workspace. `bot_permission_mode` is reported alongside, and the capping audit line names which ceiling capped. |
| 7 | A global-scope write force-applied a runtime override even when the project layer pinned the same key, so the live process disagreed with the files until a restart silently reverted it. | The write still persists and no longer force-applies; the response carries `shadowed_by_project` so a client can say "saved, but this project overrides it". |
| 8 | `HttpApprover` had no timeout and no `answerable()` override. The scheduler and best-of-N ran through it unattended, so a gate emitted an SSE event to nobody and blocked until the process restarted. | A bounded wait that fails closed, plus an `answerable` flag. `begin_turn` takes `attended`; unattended callers pass false, and `spawn_isolated_run` stamps the child core *before* the prompt freezes. |
| 9 | `Core::approver_answerable` duplicated `Approver::answerable()`, reconciled by a comment asking hosts to keep them in step. Finding 8 was that comment being violated. | `Core::with_approver` derives the flag from the approver itself. Hosts that must stamp early still can, and `run_turn_inner` reconciles every run against the installed approver, takes the approver's word, and records `answerability_mismatch`. |
| 10 | `CorePool` and the admin projection passed `trust = true` unconditionally, so a workspace whose trust prompt was declined in a terminal still had its hooks, MCP servers and `permission_mode` applied when a chat routed into it. | Both read `vak_core::trust`. Approving or re-pointing a channel's workspace in the console records that decision. Marker paths are canonicalized so one directory has one record however it is spelled; legacy markers are still honoured. |
| 11 | `apply_permission_override` reads a workspace ceiling once, at pool-entry construction, so a narrowed mode reached warm channels only when their idle window expired. | A mode change calls `CorePool::invalidate_pooled`, discarding every channel instance so the next inbound message resolves a fresh ceiling. The gateway's own core is never dropped; the count is audited. |
| 12 | `vak plan` had neither `--permission-mode` nor `--write-path` despite dispatching model-generated shell through the same engine. `vak config` had only `dump`. | `vak plan` takes both, applied before the session freezes its prompt. `vak config permissions` prints the mode, approval mode, rules, trust and composed capability standings; `set-mode` and `set-approval` persist to a chosen layer. |
| 13 | `08-permissions.md` described an interactive TUI approver, a `[p]` keybind and an `[a]` session-only grant. None existed. | The doc lists the four approvers that ship, states plainly that there is no interactive terminal approver, and describes the real trigger for a learned rule. |

## Follow-up findings, caught by the fixes running live

Two defects surfaced within hours of the upgrade, both found by the new code
reporting on itself rather than by a test.

| # | Risk | Rule |
| --- | --- | --- |
| 14 | Finding 7's shadow check compared a global write against `project_path(cwd)` without noticing they can be **the same file**. The gateway's workspace *is* `default_workspace()`, so `global_path() == project_path()`, and `load_with_trust` already skips the project pass for that case. Every global write on the gateway's own workspace therefore reported "(persisted, shadowed)" — telling an operator their change would not take effect when it would. Observed on the live install: `permission_mode=FullAccess (persisted, shadowed)` in `security-events.jsonl`, with the value in force moments later. | The check returns empty when the two paths resolve to one file. One file cannot shadow itself. |
| 15 | `capability_reach_tests` did not pin the test data home, so `global_path()` was the developer's real `~/vak-home/.vak/config.toml`, merged *under* each test's tempdir project layer. When an operator set `full-access` on their own install, every `Gated`/`Blocked` expectation became `Open` and four tests failed on that machine only. | The module pins the data home like the others. Separately, `GlobalLayerGuard` now holds a process-wide lock: restoring the file on drop is not enough when tests run in parallel, because a reader can observe a writer's window. Writers and readers of the global layer both take it. |

Finding 14 is the more instructive of the two. The shadow check was correct
about layering and wrong about identity — it asked "does a project layer set
this key?" when the question it needed was "is there a *separate* project layer
at all?". The fix that made a silent revert visible briefly made a successful
write look like a silent revert.

## Composition hierarchy

Each layer can only narrow. Marked stages are where the composed answer used to
diverge from what an operator could see or set.

```text
workspace ceiling            .vak/config.toml, gated on the trust marker
        ↓ capped_by
bot pin                      ← finding 6: the console did not fold this in
        ↓ capped_by
chat pin                     allowlist.json
        ↓
rule set (deny > ask > allow)   ← finding 2: had no writer
        ↓ no match
mode default arm             engine, by AskSource
        ↓ Ask
approval mode                auto_approve(), by source
        ↓ still Ask
Approver::answerable()       ← findings 1 and 8: unsettable, and could hang
        ↓
dispatch — or blocked before the prompt is built
```

## Install verification performed

The installed 2.0.0 (`d25e5b2`) instance was inspected before repair. Both new
endpoints returned 404 and `/admin/api/config` was missing all three fields, as
expected: pushing to `main` does not move `/Applications/Vak.app`.

The fix was then reproduced against a **copy of that install's real config and
gateway state**, on a spare port, before anything was replaced:

1. With `approvals = "deny"` — the installed default — a real Telegram turn
   through `/gateway/inbound` logged the same three `capability_unreachable`
   events quoted above, and the model replied that it had no way to reach the
   web.
2. `PUT /gateway/approvals` with the install's own approved chat flipped the
   policy to `forward`. The same turn logged **zero** new `capability_unreachable`
   events.

The build was then stamped with `VAK_GIT_SHA`, installed with
`self install --force`, verified against the manifest (`install verifies clean`),
and `self services-sync` restarted the gateway and Telegram bridge —
`self status` flagged both as stale processes beforehand and clean afterwards.
Post-upgrade, both endpoints return 200, the three fields are present, and the
console's approval panel offers the install's real chat.

The gateway's policy was left at `deny`. Turning on forwarding decides that a
chat message can authorize a gated action; that is an operator's decision, and
the audit only made it reachable.

## Where the controls now live

| Surface | Permission mode | Approval mode | Answer a gate | Gateway forwarding | Author a rule |
| --- | --- | --- | --- | --- | --- |
| Admin console | yes, per scope | yes | yes, with "don't ask again" | yes | yes |
| Desktop app | yes | yes | yes, with "always allow this" | no | shows; opens the file |
| Setup wizard | yes, posture chooser | no | n/a | no | no |
| `vak exec` | `--permission-mode` | `--yes` or auto-deny | no | no | no |
| `vak plan` | `--permission-mode` | `--yes` or auto-deny | no | no | no |
| `vak config` | `set-mode --scope` | `set-approval --scope` | n/a | read only | reads, with reach |
| Chat surfaces | no | no | forward mode only | no | no |
| `.vak/config.toml` | yes | yes | n/a | yes | yes |

Nothing in the last two columns was reachable outside the config file before
this change.

## What was deliberately not changed

`evaluate`'s severity aggregation, the universal quantification of `Allow` over
a command's segments, `resolve_through_existing`'s symlink handling, and
`PermissionMode::capped_by` all held up under review and were left alone. So
did `reach`, which is the reason this failure was legible at all: it composes
every layer before the prompt is built and states what the turn cannot use,
instead of letting a model discover it one denied call at a time.

Three test-hygiene defects surfaced while fixing the above and were repaired:
the suite wrote workspace-trust markers into the developer's real data home
(one orphan per tempdir, each granting trust to a path that no longer exists);
it read the developer's real `~/vak-home` config, so results depended on whose
machine the suite ran on; and the shared global layer was restored on drop but
not locked, so a parallel reader could observe a writer's window. The data home
is pinned per test binary and the global layer is serialized.
