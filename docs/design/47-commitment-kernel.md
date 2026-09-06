# 47 — The commitment kernel

Status: **all phases shipped; lifecycle recovery and progress hardening shipped in 3.0.8; outcome integration current in 3.0.10**.

| Phase | Delivers |
|---|---|
| I0 | kernel (`vak-intent`): seven axes, cascade, authority, narrowing lattice |
| I1 | commitment ledger (`vak-commit`): lifecycle, satisfaction lattice, projection, portfolio scheduler |
| I2 | admission, intent ledger entry, `[intent]`/`[commitment]` config, `vak intent explain` |
| I3 | route demand, per-commitment budget, prompt projection |
| I4 | capability slicing (progressive disclosure) |
| I5 | envelopes: `vak grant` / `vak revoke`, live grant reaching the turn's authority, revocation honoured on read |
| I6 | episodes bracketing durable turns; commitment upkeep on its own tick — schedule wakes, zero-token predicate wakes, escalation policies, explicit expiry |
| I7 | server endpoints, admin portfolio, desktop composer strip, the `commitments` capability, per-channel autonomy ceiling, delivery cadence/urgency |
| I8 | misread evidence: escalation-as-measurement, per-cell accuracy with routing-ledger epistemics |

Every part is switchable off with `[intent] enabled = false` and
`[commitment] enabled = false`, which together reproduce the runtime's
pre-kernel behaviour exactly.

## Problem

vak decides a great deal before a turn runs — provider and model, permission
mode, capability packet, budget, prompt layers — and made every one of those
decisions **without any model of what the user was trying to do**. Three
findings from the code as it stood:

1. **The only intent classifier was a keyword hack.** `is_managed_work_request`
   lowercased the prompt, looked for one of thirteen English verbs, and called
   it multi-step if it contained two markers from `[" and ", " then ",
   "first", …]`. That boolean was the entire difference between
   `WorkMode::Direct` and `WorkMode::Managed`, and it fired on "explain what
   this and that mean".
2. **Demand-driven routing was wired but fed zeros.** `score_demand` /
   `order_ladder_v2` is a good ordering function. Its only caller passed
   `estimated_input_tokens: 0, structured_output: false,
   reasoning_required: false, evidence_required: false`. Every session scored
   identical demand, so objective selection was constant and the router inert.
3. **The capability packet was all-or-nothing.** Every tool, skill, and MCP
   server was advertised on every turn, whether the user said "hi" or "migrate
   the billing schema".

Underneath those three is one structural gap. vak's unit of identity was the
session and its completion signal was "the model stopped talking". Both
assumptions break as soon as work outlives a conversation: there is nowhere to
put a month-long obligation, and no way to say whether it was ever met. The
stop-gate, the premature-completion guard, and the doom-loop detector are all
patches on an unfalsifiable signal.

## Design

Three things, deliberately not conflated:

| | Source | Wrong-ness costs |
|---|---|---|
| **Reading** | inferred from the request | a worse turn |
| **Authority** | granted by a human | a safety incident |
| **Engagement** | derived from both | bounded by the narrowing invariant |

A resolution bug can change what vak *thinks the work is*. It must never change
*what it is allowed to do about it*.

### Reading: seven axes, not a taxonomy

vak runs everything from "hi" to a multi-month programme, so a subject-matter
taxonomy is both endless and useless — "research" says nothing about what the
runtime should do. The axes are behavioural and orthogonal, and each drives one
subsystem.

| Axis | Values | Drives |
|---|---|---|
| `act` | `converse` `answer` `locate` `analyze` `author` `modify` `operate` `verify` `orchestrate` `govern` | capability slice, output shape, stop profile |
| `horizon` | `immediate` `turn` `session` `durable` | managed admission, commitment promotion, context profile |
| `stakes` | `inert` `reversible` `costly` `irreversible` | approval ceiling, checkpoint-before, delivery urgency |
| `evidence` | `none` `cited` `verified` `audited` | **minimum satisfaction strength to close** |
| `clarity` | `clear` `underspecified` `ambiguous` | ask vs. state-an-assumption |
| `modality` | `text` `image` `audio` `video` `screen` `data` `stream` | ladder filtering, delivery format |
| `attendance` | `interactive` `supervised` `unattended` | HIL mode, delivery cadence, gate answerability |

No axis collapses into another. A long task can be closely watched and a
ten-second automation unattended, so `horizon` is not `attendance`. Work can be
irreversible yet need only an assertion, or inert yet need citations, so
`stakes` is not `evidence`.

Subject matter survives only as an open-vocabulary `domains` tag, used for
skill affinity and telemetry and **never** for control flow — so adding a
domain can never change what the runtime is allowed to do.

`Author` / `Modify` / `Operate` is the safety-relevant split and means the same
thing for prose, a spreadsheet, and a production deploy.

Interaction rule: **ambiguity only interrupts when the stakes are high.**
`ambiguous + inert` proceeds on a stated assumption; `ambiguous + irreversible`
asks.

#### Axis algebra

Two shapes, and conflating them was a real bug during implementation:

* **Categorical** (`act`, `clarity`): rival explanations. Confidence comes from
  the winner's lead over the runner-up.
* **Ordered** (`stakes`, `horizon`, `evidence`): levels on a scale. A signal
  saying `reversible` does not argue against `irreversible`, it agrees with it
  more weakly. Scoring these by argmax made corroborating evidence *reduce*
  confidence — a dirty working tree voting `reversible` dragged "deploy the
  billing service to production" below the acceptance floor and lost the
  irreversible reading entirely. Ordered axes now take the highest level with
  real support, which is also the cautious direction on every one of them.

Environment-derived signals (a dirty tree) apply only to effectful acts. A
repository mid-edit does not make answering a question risky.

#### Confidence is per-axis

A single scalar conflates independent questions. Capability slicing depends
only on `act`; commitment promotion only on `horizon`. Gating both on the
weakest axis meant an unsignalled horizon — most requests, since few say how
long they will take — suppressed slicing the act reading was certain about.

For the slice specifically, the question is not "which single act won" but
"does the slice cover this request". Acts within half the winner's weight join
the slice, and confidence is how much of the total act evidence that set
accounts for. "Fix the failing test" is a `modify` *and* a `verify`; resolving
the tie by argmax would remove half of what it needs.

### Resolution cascade

Cheapest first, stopping once confidence clears the bar.

| Tier | Cost | Reproducible |
|---|---|---|
| 0 — **declared**: CLI flags, flow intent, pinned channel policy, inherited subagent engagement | free | yes |
| 1 — **signals**: lexical, structural, deictic, workspace, session, surface, attachment | free | yes |
| 2 — **local model**: strict JSON via Ollama | ~free | no |
| 3 — **cloud model**: only below the tier-1 threshold | metered | no |

Tiers 0–1 are pure functions of recorded inputs, so a decision that narrowed a
turn can be reconstructed months later. Tiers 2–3 record the model id and
prompt digest and are marked `reproducible: false` — the same honesty the
routing ledger applies to `Settlement::Unknown`. Never claim replay fidelity
you do not have.

The kernel decides **whether** a paid tier is warranted; `vak-core` performs
it, because a dispatch is a dispatch: `WorkPurpose::Classify`, a work receipt,
spend-gate admission under `max_classify_usd`, the cheapest leg on the frozen
ladder rather than the primary, a short watchdog, and **fail-open** to the
general engagement. A classifier outage must never block work.

A classifier may raise stakes freely; it may not lower them below what the act
implies. A model cannot talk the runtime out of caution it reached
deterministically.

### Authority: the autonomy spectrum

Autonomy is **delegated by a human**; attendance is **observed by the runtime**.
Conflating them is why agents nag when you wanted autonomy and barrel ahead
when nobody is watching.

| Autonomy | Meaning |
|---|---|
| `manual` | propose only |
| `assisted` | act on reversible things; ask for costly or irreversible |
| `delegated` | act inside a declared envelope; escalate outside it |
| `autonomous` | act freely within the permission mode; report afterwards |

An **envelope** is pre-authorization *within existing authority* — never a
grant of new authority. Its `permission_ceiling` can only lower the effective
mode, through the same `PermissionMode::capped_by` a gateway channel override
uses. Revocation follows invariant 11: it cancels in-flight work.

`intent.autonomy` and `intent.escalate = "cloud"` are **privileged config**,
stripped for an untrusted project. A cloned repository must not grant itself
the right to act without asking, nor spend the user's credentials classifying.

Irreversible work reaches a human whatever was delegated. A grant to act
without asking is not a grant to act without anyone ever knowing.

### Human-in-the-loop: four modes

Chosen by `attendance × stakes × autonomy`.

| Mode | When | Behaviour |
|---|---|---|
| **Interrupt** | interactive, high stakes | block and ask now |
| **Envelope** | delegated, long-running | proceed inside the grant, escalate outside |
| **Review** | reversible and checkpointed | do it, show the diff, offer reversal |
| **Defer** | unattended, needs a human | suspend into the inbox |

**Defer** is the new one. An unattended surface used to fail closed
unconditionally — correct for a one-shot turn, wrong for month-long work, which
should wait rather than fail. Deferring needs somewhere to park the question,
so it is only offered when the horizon opens a commitment; a one-shot
unattended turn still fails closed exactly as before.

Every deferred question carries an escalation policy. `AssumeConservative` is
refused above `costly`: assuming a default for an irreversible action because
nobody replied is precisely the autonomy this system exists to prevent.

### The commitment

`horizon ≥ session` promotes a resolution into a durable **commitment** in its
own append-only ledger, independent of any session. Below that, intents resolve
and die inside the turn — a simple prompt pays nothing.

```
Proposed → Active ⇄ Suspended ⇄ Blocked → Satisfying → Closed{verdict}
                        ↘ Superseded / Abandoned / Expired ↗
```

`Verdict::Unknown` is mandatory, not a nicety. Without it the only way to tidy
an untracked commitment is to assert an outcome nobody verified.

#### The satisfaction lattice — how we know it is done

```
Asserted  <  Cited  <  Observed  <  Attested
```

Strength comes from **how a criterion was established**, not from anyone's
confidence:

| Criterion kind | Strength |
|---|---|
| `Shell`, `FileExists`, `FileContains`, `ToolSucceeded`, `FlowCompleted` | `Observed` — the runtime ran it |
| `ExternalReceipt` | `Attested` — a third party vouched |
| `Semantic` | `Asserted` — only the model's judgement |

**Closure invariant:** a commitment may not close `fulfilled` below the
strength its `evidence` axis demands, and the ledger refuses the event at
append time. A ledger that can record a lie is not an audit trail.

The model may **propose** criteria; it may never **mark one passed**. Same
separation of powers as permission-before-dispatch. A failed or undetermined
check carries no evidentiary weight regardless of kind — only a pass earns it.

Failure verdicts are deliberately unconstrained: recording `failed`,
`abandoned`, or an honest `unknown` must always be possible, or the ledger
could not tell the truth about work that went wrong.

Criterion and evidence vocabulary is reused from `vak-session`
(`CriterionKind`, `CriterionResult`, `EvidenceRef`, `WorkCriterion`,
`WorkOwner`) rather than redefined.

#### Progress versus motion

Every episode ends with a typed advancement: `Advanced` (a criterion moved),
`Learned` (uncertainty reduced — legitimate progress), `Blocked`, or `Stalled`
(spent budget, moved nothing, learned nothing). Consecutive stalls trip a
breaker at commitment scale. `Learned` exists so genuine exploration is not
punished, which is how naive progress metrics fail.

#### Suspension

Every wake condition maps onto a mechanism vak already has, which is why this
is rewiring rather than new infrastructure:

| Condition | Mechanism |
|---|---|
| `Human` | `vak-core/src/inbox.rs` + gateway approval forwarding |
| `Schedule` | `vak-core/src/tasks.rs` cron engine + scheduler catch-up |
| `Predicate` | `TaskDef.script` watchdog — **zero tokens while the predicate stays false** |
| `Commitment` | dependency edge |
| `External` | `vak-delivery` outbox / webhook |

#### Long-horizon mechanics

* **Re-admission.** A commitment resuming after weeks re-resolves its
  engagement against current reality and records the drift. Never silently
  inherit a stale contract.
* **Lifetime economics.** A lifetime budget and relevance TTL, not just
  per-run caps. Budget exhaustion *holds* the work rather than failing it — a
  human can raise the ceiling, and everything established is still good.
  Expiry produces an explicit `expired` verdict, never a silent deletion.
* **Supersession.** "Forget the migration, just add the index" supersedes with
  a lineage link. Without this you get the zombie task graveyard that kills
  every list-based agent.
* **Portfolio scheduler.** Deterministic and inspectable: a total order over
  stakes, deadline proximity, staleness, progress, review-due, and a user pin
  that dominates every computed factor. Every priority decomposes into named
  components. An opaque scheduler in a system whose thesis is auditability
  would be the one place you could not ask "why did it do that".

### Delivery posture

`DeliveryPosture` decides *when* a packet goes out, never what it says: the
semantic contract, the renderer and the outbox are untouched. Two rules
override the cadence, and both are about not losing something that matters —
an `Interrupt` urgency always sends, because an irreversible step's
confirmation must not sit in a digest until morning; and an approval always
sends, because a held gate is a stopped run and batching it would turn a
question into a hang.

### Per-channel autonomy

`ChannelPolicy.autonomy_ceiling` joins the existing restrictive-only chain. A
chat may cap delegation below what the workspace granted and may never raise
it: a Telegram chat gets to say "propose only, in here", and never "act
freely" on a workspace whose operator did not. `vak-config` ranks the names
without depending on the kernel; a test in `vak-core`, which sees both, pins
the two rankings equal.

### Did we read it right?

Misclassification becomes **measurable**. Typed misread signals: the user
rephrases immediately, corrects, aborts, overrides the intent chip — and
strongest, **escalation**, where the engagement sliced a tool out and the model
then asked for it. That is a measured misread, not a guess, and it is a direct
benefit of doing the slicing at all. These fold into the routing evidence
ledger with the same epistemics (success / failure / **unknown**, Laplace
shrinkage, 30-day TTL), so intent accuracy tunes the resolver the way route
evidence tunes the ladder. *(Phase I8, not yet wired.)*

## Invariants

1. **Intent narrows, never widens.** An engagement's `Limits` may subtract a
   capability, shorten the ladder to a prefix, lower a budget, or *raise* an
   approval floor. Never grant, extend, raise a cap, or lower a floor. `Limits`
   is a meet semilattice whose top element reproduces pre-kernel behaviour;
   `meet` is the only composition operator offered and there is deliberately no
   `join`.
2. **Intent never gates the permission engine.** Permission is evaluated
   exactly as before; intent may only add an approval requirement. Nothing in
   the kernel can authorize anything.
3. **An envelope is pre-authorization within existing authority**, never a
   grant. Revocation cancels in-flight work.
4. **The runtime evaluates satisfaction; the model never does.**
5. **A commitment may not close above its evidence class.**
6. **Model-visible means logged.** The intent note gets its own entry type
   carrying the exact contributed text; the projection reads those bytes rather
   than re-deriving them, so a replay reproduces the prompt even if the
   derivation rules have since changed.
7. **Reproducible, or declared not to be.**
8. **Uncertainty resolves to the general engagement** — byte-for-byte the
   previous behaviour. Being unsure must never silently remove a tool.
9. **Commitments close explicitly**, with a verdict and evidence.
10. **Unsupported modality fails typed, never silently degrades.** Dropping an
    image because the serving model is text-only is the "everything worked as
    designed and the outcome was a lie" failure `reach` exists to prevent.
11. **Subagents inherit, narrowed.**

## What this replaces

A consolidation, not an addition:

| Removed / demoted | Becomes |
|---|---|
| `is_managed_work_request` | **deleted**; managed-ness follows from `horizon` |
| `WorkMode::Auto`'s keyword branch | the reading |
| Session-scoped `WorkContract` as the only durable work state | commitment-scoped (staged) |
| Stop policy's completion role | narrows to "did this episode terminate cleanly" |
| Doom-loop heuristics | the stall breaker's measurement (staged) |
| Per-call approval as the only HIL | one of four modes |

## Layout

```
crates/vak-intent   axes, signals, cascade, authority, the narrowing lattice.
                    No vak dependencies: a pure decision layer, unit-testable
                    without a network, a model, or a config file.
crates/vak-commit   durable commitments, the satisfaction lattice, the
                    append-only ledger and its projection, the portfolio
                    scheduler. Reuses vak-session's criterion vocabulary.
vak-core/intent.rs  the seam: gathers facts, runs the cascade, projects the
                    engagement onto runtime knobs. Every function takes a
                    baseline and returns something no wider.
```

## Configuration

```toml
[intent]
enabled = true              # false reproduces pre-kernel behaviour exactly
accept_confidence = 0.75    # bar for capability slicing
provisional_confidence = 0.45
slice_capabilities = true
posture = true              # stakes may raise the approval floor
escalate = "none"           # none | local | cloud    (privileged at "cloud")
max_classify_usd = 0.01
autonomy = "assisted"       # privileged

[commitment]
enabled = true
lifetime_budget_usd = 25.0
stall_limit = 3
review_every_hours = 24
default_ttl_days = 30
```

## Surfaces

* `vak intent explain "<prompt>"` — the reading, every signal with the weight
  it carried, the engagement diff against doing nothing, and the exact text the
  model would additionally be told. Costs nothing, dispatches nothing.
* `vak intent show` — the resolved policy.
* `vak commit list|show|close|supersede|attest` — the portfolio, ordered by the
  same scheduler the runtime uses. `close --verdict fulfilled` surfaces the
  closure invariant to a person.
* `GET /intent/explain`, `GET /intent/policy`, `GET /commitments`,
  `GET /commitments/{id}`, `POST /commitments/{id}/close` (409 on a refused
  closure — the request was well-formed; the evidence simply does not support
  the claim).

## Verification

`cargo test -p vak-intent -p vak-commit` covers axis algebra, the cascade, the
lifecycle, and the property test that no engagement derived from any reading in
the reachable space (10 acts × 4 horizons × 4 stakes × 4 evidence × 3 clarity ×
3 attendance × 4 autonomy × 2 slice settings) ever widens the baseline.

`crates/vak-session/tests/intent_entries.rs` proves the projection: the note
reaches the model immediately before the turn it governs, only the newest one
applies, a silent engagement costs zero tokens, replay reproduces the recorded
bytes, and notes stay out of the compaction packet.
