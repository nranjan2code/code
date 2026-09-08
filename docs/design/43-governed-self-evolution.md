Status: proposal
(does not describe shipped behavior; paths below are targets rather than
citations and are skipped by check_doc_paths.py)

Governed Self-Evolution
========================

## Goal

Allow vak-capable agents to propose and apply code changes to their own
behavior — subject to human review gates and the narrowing invariant
(docs/design/47-commitment-kernel.md). Self-evolution must never escalate
authority.

## Architecture

```
┌─────────────┐    ┌──────────────┐    ┌──────────────┐
│   Agent     │───▶│   Intent     │───▶│   Proposal   │
│   (model)   │    │   Kernel     │    │   Engine     │
└─────────────┘    └──────────────┘    └──────┬───────┘
                                           │ submit
                                           ▼
                               ┌─────────────────────┐
                               │   Skill Proposal    │
                               │   (docs/design/26)  │
                               └────────┬────────────┘
                                        │ reviewed
                                        ▼
                              ┌──────────────────┐
                              │   Capability     │
                              │   Registry       │
                              │  (docs/design/41)│
                              └──────────────────┘
```

## Proposal Lifecycle

1. **Generate**: The agent proposes a change as a skill proposal
   (`crates/vak-plugin/src/lib.rs`: manifest + capability declaration).
2. **Scope**: The intent kernel (`crates/vak-intent`) narrows the proposal.
   Intent narrows, never widens (rule 32). A proposal cannot add tools,
   extend ladders, or raise caps.
3. **Validate**: The capability registry (`crates/vak-core/src/capability/`)
   resolves the proposal against the host's capability vocabulary. Only
   proposals that fall *within* the existing authority are admissible.
4. **Review**: Skill proposals with network, secret, or external-effect
   impact require human approval (rule 25). Presentation/skill/evidence
   proposals may be auto-admitted only inside an existing envelope.
5. **Promote**: Approved proposals take effect via the level-triggered
   reconcile loop (rule 31). No restart required.
6. **Revoke**: Revocations take effect immediately (rule 31).

## Constraints

- **Authority ceiling**: The proposal engine cannot grant authority the
  submitting agent does not itself possess.
- **Append-only ledger**: All proposals and their outcomes are recorded in
  the session JSONL (rule 2: append-only sessions).
- **No ambient escalation**: A self-evolution proposal that would widen
  authority is rejected at admission by `PermissionEngine`
  (rule 16: every execution path authorizes before dispatch).

## Security

- All proposed changes are signed with the operator's Ed25519 key
  (see `crates/vak-plugin/src/lib.rs` catalog-signature verification).
- A revoked key prevents any further self-evolution proposals from that source.
- The proposal engine is itself a capability — `governed_self_evolution` —
  that must be explicitly admitted; it is never on by default.

## Open Questions

- How does self-evolution interact with the frozen route ladder? (Rule 7:
  retries honor the frozen ladder. Self-evolution proposals must not modify
  a session's frozen route, only propose new ones for future sessions.)
- How do we prevent recursive self-modification? (A proposal cannot modify
  the proposal engine itself without a fresh human approval cycle.)
