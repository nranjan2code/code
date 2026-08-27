# 24 — Security boundaries

Security is enforced by `vak-runtime` and its broker, not by a UI convention.
All surfaces are untrusted clients of the authenticated Runtime API.

## Trust boundaries

```text
user/client → authenticated server → Runtime policy → broker → worker/effect
                                      │
                           state + audit + ledger
```

The provider, model, channel, project files, memory, and skills are untrusted
inputs. Runtime validates them before admission or dispatch and records
model-visible decisions.

## Required controls

- canonical project roots reject traversal and symlink escapes in restricted
  modes;
- every effect checks permission, sandbox, capability epoch, resource claims,
  and cancellation immediately before dispatch;
- built-in workers receive bounded operational environments;
- provider/channel credentials are recipient-scoped and never enter sessions,
  logs, prompts, or client state;
- FullAccess requires an explicit human decision and is never inferred from a
  model request or a previous approval;
- approval silence, timeout, missing delivery, stale epoch, and malformed
  verdicts deny access;
- capability changes cancel and join old work before reopening admission;
- the gateway lock prevents duplicate Runtime authorities.

## Auditability

Session JSONL reconstructs every model-visible message. Runtime audit JSONL
records mutations, authorization decisions, credential-independent metadata,
and terminal outcomes. SQLite projections are disposable and cannot replace
the ledger or audit record.

## Verification

The security suite exercises hostile paths, symlink escapes, worker environment
redaction, stale approvals, capability revocation, missing sandbox, broker
boundary enforcement, gateway singleton behavior, and typed failures. Any
failed containment or authorization check fails closed.
