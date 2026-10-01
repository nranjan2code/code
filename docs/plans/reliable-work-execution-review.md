# Reliable work execution — design review

Status: review complete; proposed changes are not implemented.
Date: 2026-09-30.
Source reviewed: `0bfcaa39e341cef46360513e9d6f037c0f5f2493`.

## Decision

Accept the proposed shared work budget with revisions. A budget alone would
contain this incident but could prevent delivery of the report the user liked.
The general fix needs shared resource accounting, coordinated provider admission,
context selected for the work, and evidence-based continuation. These belong in
the existing runtime contracts, not in a domain-specific workflow.

The target is reliable completion with efficient resource use and retained
progress. Complete answers remain subject to available evidence, capabilities,
authority and provider availability. The runtime must never claim completion
just because it stopped, or create an unlimited spending obligation to avoid
reporting a blocker.

## Incident evidence and limits

Read-only inspection of the AWS session identified this request by its quoted
directive. No provider text, credentials, raw source material or unrelated
conversation content is reproduced here.

| Recorded fact | Finding |
|---|---|
| User assessment | The resulting report was good; preserve this quality |
| Provider completions with usage | 29 |
| Failed provider attempts without usage | 109 |
| Failure mechanism | All 109 reported tokens-per-minute rate limits |
| Uncached input tokens | 331,737 |
| Cached input tokens | 3,192,986 |
| Output tokens | 11,489 |
| Recorded total tokens | 3,536,212 |
| Input per successful completion, including cache | 66,937 to 168,665 |
| Sum of successful attempt latency | 269,987 ms |
| Sum of failed attempt latency | 529,928 ms |

Cached input is a separate tier in Vak's normalized `Usage`; the total above
includes it once. Tokens are not dollars. Failed attempts do not establish zero
usage or zero charge, and their missing usage must remain explicit. The large
input and TPM errors support investigating provider capacity and context demand;
they do not prove which historic content was irrelevant or whether cached tokens
counted against this account's provider limit. That requires request-shape and
provider-limit evidence. Attempt latency does not account for tool execution,
approval waits, backoff, queue time, or every interval in the wall-clock timeline.

The deployed source revision was not matched to the local source revision during
this review. Incident observations and current-tree findings are separate evidence.

## Findings in the current tree

1. **A dispatch ceiling is scoped to a model step.**
   `crates/vak-agent/src/lib.rs` constructs a new `StepLedger` for each execution
   completion. Retry and endurance counters also restart for later completions.
   A local ceiling does not govern the complete request or its child work.

2. **There is already a broader spend gate.**
   `Core::spend_gate_for` reuses `CoreSpendGate` across a session and the day tracker
   coordinates gates in a Core. The earlier claim that the runtime has no broader
   cost control was too strong. The gaps are optional dollar caps, session scope
   rather than explicit request ownership, uneven accounting of failed attempts,
   and propagation across the complete execution tree.

3. **Reservations need exact ownership and settlement.**
   `CoreSpendGate::authorize` reserves an estimate for the day cap but checks the
   run cap against settled spend. Settlement subtracts actual cost from aggregate
   reservations rather than releasing a specific reservation. Failed dispatches
   have no corresponding settlement call in this path. Parallel work, estimate
   differences, cancellation and unknown usage therefore need explicit lifecycles.

4. **Recovery does not coordinate account capacity.**
   Per-step retries and ladder fallback can repeatedly contend with the same TPM
   allowance. Alternate wire adapters are not independent capacity when they use
   the same provider account. Rate limiting needs pacing and shared admission;
   informed transience must retain invariant 7's distinction from blind failures.
   Typed error normalization also needs review: the Responses streamed-error path
   currently groups `insufficient_quota` with rate limits.

5. **Context planning largely fills available capacity.**
   `vak-context/src/planner.rs` ranks closed turns and fills a capacity-derived
   budget. The open turn is replayed verbatim. Capacity determines what fits, not
   everything needed to solve the current request. Improve selection and evidence
   representation without arbitrary cuts or abandoning whole-turn invariants.

6. **Progress and quality contracts already exist.**
   Design 52 explicitly requires evidence of progress and recovery charged to
   existing budgets and deadlines. Designs 42, 47 and 50 supply work ownership,
   requirements and scoped evidence. Wire and extend these contracts rather than
   create a mandatory planner or a second definition of success.

7. **The client obscures the operational cause.**
   `client_events::project` reduces retries to a generic state and drops route
   fallback events. A person needs a plain explanation of waiting, retained work,
   and what will happen next; detailed attempt diagnostics belong in disclosure.

## Revisions to the proposed fix

- Share accounting across a root request and all its descendants, nested under
  existing session, commitment and operator caps. An allocation is not authority
  to raise a cap. Keep a step receipt for attribution without resetting the root
  accounting.
- Coordinate capacity by confirmed account identity and applicable provider limit
  dimensions, using structured provider observations. Never presume that another
  adapter or model grants a new account quota, and never expose credential values.
- Use learned estimates as scheduling inputs. Estimates carry uncertainty and
  cannot authorize unlimited budget expansion. Explicit policy supplies finite
  outer bounds; defaults and calibration must be reviewed before implementation.
- Select context for required facts, references, constraints and evidence, within
  measured capacity. Retain exact material in the ledger/evidence store and make
  excluded material recallable. Do not silently rewrite an already dispatched
  request in the middle of an exact retry.
- Use typed evidence and requirement transitions to judge progress. Creative
  responses and conversations can progress without tools; a count of sources,
  words or calls cannot be a universal success measure.
- Protect capacity for synthesis, verification and delivery before starting more
  exploration. Honest partial delivery is not equivalent to fulfilled work.
- Save resumable state at existing lifecycle boundaries. Resume does not imply
  scheduling background work; that requires existing ownership and authority.
  New recurring work remains a `TaskDef`.

## Architectural constraints

Preserve append-only sessions, `derive_messages()` reconstructability, permission
revocation, the per-turn admitted route ladder, same-model identity rules, broker
boundaries and abort preservation. Use existing session/commitment/evidence
storage. Any unavoidable new durable file needs a registry declaration and
canonical path ownership.

This proposal starts no pending data-architecture milestone, introduces no second
schedule model, no provider SDK, no domain-name branch, and no baked-in model list.
It does not authorize production changes or deployment.

Implementation and acceptance gates are in
[reliable-work-execution-plan.md](reliable-work-execution-plan.md).
