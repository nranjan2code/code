# Review 2 — the data architecture after a week of new work

Status: **review, 2026-10-01; applied the same day.** The maintainer
accepted every finding and all eight decisions in §4, and §5 says where
each fix landed (plan revision 3, docs 73 and 74, the blast-radius
inventory, AGENTS.md and the dependent proposals). It reads plan
revision 2, docs 73 and 74 and the blast-radius inventory against what
changed since they were locked on 2026-09-25: releases 5.0.0 to 5.3.2 (248
commits), the new proposals 76, 79, 80, 81 and 82, doc 78 (in progress), the
collaboration plan, the reliable-work-execution plan and the Canvas plan.
Source read on `main` at `438cfcd5`; nothing was run live. Finding numbers
continue from `data-architecture-review.md` (R1–R36, D1–D24).

## Verdict

The direction still holds: git-style storage, SharePoint-style information
architecture, CI-style runs, one reconciler, one catalog, the scenario
matrix. M0 shipped as planned. But the ground has moved in five ways:

1. **The version cut is gone.** The plan cuts "a new 5.0.0 baseline" from
   4.0.2. 5.0.0 shipped on 2026-09-26 and main is at 5.3.2. (R37, R38)
2. **The debt is growing faster than the plan assumed.** Raw home-path
   calls rose about 10% in six days, new per-Agent stores pass a registry
   that declares all of `agents/` as one ledger, and the server writes every
   Agent's sandbox, Office-room and coworking records into the built-in
   Agent's home. (R40–R43)
3. **Five proposals need a layer the plan does not have.** Intake (76), mail
   and calendar (80), pieces (81), the fleet (79) and the collaboration plan
   each need triggers beyond cron, durable cursors, external effects with
   receipts, fenced leases and connections. Each would build its own. (R45,
   R46)
4. **Doc 79 put requirements on the substrate** that M2 must not preclude:
   a key-release authority, versioned Desired state for a coherent recovery
   manifest, fencing epochs, and integrity checks without keys. They are
   cheap before M2 and expensive after. (R48–R53)
5. **Multi-person work breaks two assumptions.** The trace key has no actor,
   and one key per conversation cannot erase one person from a shared
   conversation. (R54–R57)

Almost every roadmap item now waits on M6 or M8, so the order of the
critical path matters more than it did. §3 proposes a new one.

## 1. Findings

Severity as before: **C** would ship something wrong or blocks the plan;
**H** a significant gap or wrong assumption; **M** correctness or
completeness; **L** accuracy or hygiene.

### Stale or wrong

| ID | Sev | Finding | Evidence | Fix |
|---|---|---|---|---|
| R37 | C | **The cut's version number is spent.** L3 says "cut a new 5.0.0 baseline; the workspace is at 4.0.2". 5.0.0 was released with M0's removals on 2026-09-26, and main is at 5.3.2. Every "5.0.0 baseline" reference is now wrong. | CHANGELOG 5.0.0; workspace `Cargo.toml:36`; plan L3, M3b, §5; blast radius M3b and "External boundaries"; doc 73 §13; doc 82 §1, §9.2; doc 76 §8; AGENTS.md "Pending" | Re-lock L3 as "the next major version at M3b, expected 6.0.0". Plan §1 holds the number; every other doc says "the data baseline" and points there, so this cannot go stale again. |
| R38 | H | **There is no release plan for M3b.** Main releases daily. M3b is XL and non-additive; invariant 29 forbids a non-additive change inside a major, so no part of M3b can ship in a 5.x release, and a long-lived branch will rot against a 24 108-line `vak-server/src/lib.rs` at this commit rate. | `git log --since=2026-09-25`: 248 commits, 15 releases | Decide the train (decision 2, §4): when M3b's first slice merges, main becomes the 6.0 line and cuts no 5.x release; fixes the EC2 host needs go on a `release/5` branch until 6.0.0. M3b lands in slices on main (R59). L5 allows a dev purge between slices. |
| R39 | M | **Doc 76 §9 shipped but still says "lands now".** The untrusted layer strip, `env_clear` and install-only `feeds_dir` are in 5.1.x. | `5426018f`; `crates/vak-server/src/feeds.rs:93,179`, test `feeds_dir_never_resolves_from_a_workspace` | Doc 76's status line names §9 as shipped, with the commit. |
| R40 | M | **Appendix A misplaces writers and misses new ones.** `commitments.jsonl` and `intent-evidence.jsonl` are per Agent, not at the data root. Sandbox records, candidates, execution streams, Office rooms and coworking grants are in the server Core's home, not the session Agent's (R43). New since: `auth/` (owner passkeys, doc 78), `agents/<id>/office-workspaces/<session>/<room>.json` (rewritten on every edit), plugin `catalog-staging/`. | `vak-commit/src/ledger.rs:166`; `vak-core/src/misread.rs:145`; `vak-server/src/office_workspace.rs:131-160`; `vak-server/src/auth_identity.rs:78-121`; `vak-server/src/lib.rs:10497` | Correct the rows. `auth/` is tenant-scoped Desired holding public keys and recovery-code digests, always backed up (doc 79 §8 requires owner records). Office rooms are a **Document** (each save a version, current a ref), which is what their compare-and-swap head already is. |

### The debt rules are not holding

| ID | Sev | Finding | Evidence | Fix |
|---|---|---|---|---|
| R41 | H | **Drift since the inventory.** Using the same patterns as blast-radius §0: `sessions_home()` 200 → 213 calls (20 → 23 files), `shared_data_home()` 69 → 84 (10 → 13 files), `hash_cwd(` 10 → 13. A broader home-identifier regex finds 626 uses in 69 files (not directly comparable with the 552). M3a's job grows every week it waits. | rerun of §0, production code only, `#[cfg(test)]` items removed by brace matching | Land a **ratchet** now: a test with the counts checked in, failing on any increase and accepting decreases. It is M0-sized; M3a deletes it when the counts reach zero. |
| R42 | H | **The registry declares `agents/` as one Ledger.** Everything under an Agent home is "declared" by one line with the wrong kind: memory and presentation packs (rewritten), Office rooms (rewritten, added 2026-09-27), coworking grants, sandbox records, commitments, checkpoints. AGENTS.md's "declare every new durable file" cannot be enforced for Agent-scoped state, and the enforcement test cannot see a new one. | `crates/vak-core/src/state.rs:139` | Replace the entry with per-subpath entries under an `agents/{agent}/` pattern, each with its real kind, and make the test fail on an unknown subpath. M3b's class registry needs exactly this list. |
| R43 | H | **D25: server-side stores ignore the session's Agent.** `sandbox_records_path`, `sandbox_candidates_root`, `session_sandbox_events_path`, the Office rooms and the coworking grants resolve `state.core.sessions_home()`, the server's default Core, not the session's scoped Core. One file holds every Agent's records. That breaks invariant 37's private state, and an Agent-scope erasure (doc 74 §2.3) would miss another Agent's records or take them with the built-in Agent's. | `vak-server/src/lib.rs:11052-11082`, `:7962`, `:8142`, `:8203`; `office_workspace.rs:136,285`; doc 82 §0 already depends on it | Doc 73 §2 records it as D25. The target layout fixes it by construction (records keyed by TraceKey). M3a marks these call sites, M3b fixes them, and the Library prototype keeps filtering by session header until then (doc 82 §3.8). |
| R44 | L | **A new unpinned dependency.** `webauthn-rs = "0.5.5"` is a caret range in a crate manifest; `rusqlite` and `async-nats` are still unpinned. | `crates/vak-server/Cargo.toml:46`; code rules | Pin all three exactly in the workspace manifest. M2 already moves `rusqlite`. |

### The missing layer: work that runs without a person

| ID | Sev | Finding | Evidence | Fix |
|---|---|---|---|---|
| R45 | C | **Five proposals need primitives the plan does not define,** so each will build its own (table below). M4 defines only `TaskDef`, a `Cause` enum of seven kinds and a slot claim. Doc 81 §20.5 leaves "triggers or `TaskDef`" open, doc 80 routes through `TaskDef`, doc 76 defines its own Source. | docs 76 §1–2, 80 "Operating modes", D4, D10, 81 §6.2, §10, §15.1, 79 §8 | **M4 becomes "runs, triggers and effects"** (decision 3): `TaskDef` grows into one **Trigger** model (schedule is one kind; event, webhook, on-open, watch and source poll are others), which settles doc 81 §20.5 and restates invariant 38; `Cause` gains the trigger kinds; a **Cursor** is a Ref kind with CAS; an **EffectRecord** chain (`prepared → dispatched → accepted | confirmed | failed | unknown → reconciled`, idempotency key, receipt) makes the outbox its first kind and serves doc 80's commit gate, doc 81's effects and doc 79's "reconcile from receipts, never replay"; leases carry fencing epochs (R48). **Connection** (an account) and its grants enter doc 73 §5 now as privileged Desired plus Secret, and are built with their first consumer. |
| R46 | H | **The reliable-work plan creates a second root identity.** E1 adds durable reservations under a "root work resource account" keyed by the admitted request, and says not to introduce the Run model. When M4 lands there would be two meanings of "the root of this work", which is R13 again. | `reliable-work-execution-plan.md` E0 decision 1, E1 | Before E1 writes a durable record, M1's `ids.rs` lands with `RunId`, and E1 keys its account by it, so M4 adopts the account instead of replacing it. |
| R47 | H | **Where an Agent's workspace lives after M3b is undecided, and the default label would expire it.** M3b makes Agent workspaces "environments of kind agent"; doc 74 §3.1 expires environments 7 days after settle. But an Agent workspace holds deliverables, not runtime: invariant 35 calls a command's files "the work itself", and docs 81 and 82 call a deliverable Saved when it is "in the folder". Doc 82 §10 question 3 asks for this decision. | blast radius M3b (`paths.rs:91-97`); doc 74 §3.1; doc 82 §1 decision 4 | Decide before M3b (decision 6): an Agent workspace is a **Workspace** bound to (Space, Agent) with the Space's lifecycle, never an Environment; deliverables reach the Space folder through Review. Reword doc 73 §5's L4 rule and §13. |

The shared needs behind R45:

| Need | 76 intake | 80 mail and calendar | 81 pieces | 79 fleet | collaboration |
|---|---|---|---|---|---|
| Trigger other than cron | source poll | watch, event-relative | event, webhook, on-open, manual | | handover |
| Durable cursor | per source | mailbox and calendar | piece KV | channel cursors | |
| External effect, receipt, unknown outcome | alert delivery | send, RSVP | effects | reconcile, never replay | "retry must not create a second assignment" |
| Lease with fencing | | one per routine, multi-host | one host at a time | old VM must not resume | |
| Connection and grant | | account linking | connections | | |

### What doc 79 needs from the substrate

| ID | Sev | Finding | Evidence | Fix |
|---|---|---|---|---|
| R48 | H | **Fencing is needed before any remote.** The plan keeps a flock locally and adds leases only with a remote (R18, M9). A restore onto a replacement machine creates two writers with no remote configured, and doc 80 and 81 need one routine lease and one host per piece. | doc 79 §8; doc 73 §11.1 ("a new fenced writer epoch"); doc 80 "Operating modes"; doc 81 §15.1 | M2's refs carry a monotonic generation and a writer epoch. Slot claims, routine leases, channel pollers and session writers record the epoch they hold; a restore bumps it. M9 extends the same epochs across remotes. |
| R49 | H | **A coherent recovery manifest needs versioned Desired state.** The manifest binds record heads, ref generation, object inventory, key grants, the Desired-state revision and the erasure watermark. Desired state (config, bots, allowlist, tasks) is atomic-replace files with no revision, in the plan as today. | doc 79 §8; doc 73 §5 Desired row | Desired becomes versioned like a Document: each save an immutable version, current a ref. Config history and its audit come with it. |
| R50 | H | **Key custody must be a trait.** Doc 73 §7.3 puts the KEK in the credential store. Doc 79 §5 needs release from a customer-controlled authority bound to an approved software measurement, and revoking it must stop reads (scenario 7). | doc 79 §5, §8 table | M2's `keys` defines a `KeyAuthority` (wrap, unwrap, rotate, revoke, health). The credential store is the local implementation; KMS and attested release come later. Admission fails closed when the authority is unhealthy. |
| R51 | M | **The hash chain must cover stored frames, not plaintext.** Today `prev_hash` is the digest of the plaintext line. Under per-entry encryption, a plaintext chain cannot be verified without keys, cannot be verified at all after a crypto-shred, and cannot give doc 79 its "signed, content-free validation result". | `crates/vak-session/src/log.rs:306-319` | Hash the frame as stored (ciphertext when encryption is on). KEK rotation re-wraps and never re-encrypts, so hashes stay stable. M2 exit tests verify a chain without keys and after a shred. |
| R52 | M | **Compression is in the wrong place.** Doc 73 §6 compresses sealed segments, but ciphertext does not compress, so with encryption on (the default) segment compression does nothing. | doc 73 §6; plan M2 | Compress, then encrypt, per frame and per object. Segment compression applies only when tenant encryption is off. The 20 KB-per-turn budget is measured with encryption on. |
| R53 | M | **Doc 79's strong durability mode needs every write in one commit protocol**, and three classes write around `Store`: Workspace (tools write files directly), Secret (the credential store) and piece databases (doc 81 §8). | doc 79 §8.1 | Out of scope to build, but M2's `Store` exposes a commit generation and a pre-acknowledgement hook so the mode can be added. Doc 73 names the classes that cannot join it, which caps what the mode may promise. |

### People: actors, principals and shared conversations

| ID | Sev | Finding | Evidence | Fix |
|---|---|---|---|---|
| R54 | C | **The trace key has no actor.** `Cause::User { request_id }` names no person. Doc 82 notes a version's author is not recorded; the collaboration plan requires every contribution to record its real actor and the authority an Agent acts under; doc 74's person-scope erasure and data roles need principal ids. Doc 78 has one owner, doc 69 invitation principals, the gateway channel senders: three identities with no common id. | doc 73 §4; doc 82 §0; collaboration plan §3; doc 74 §3.4, §3.6 | M1 adds `prn_` principal ids (owner, invitee, channel sender, Agent, system) and `actor` plus `on_behalf_of` on the trace key (decision 5). How a person proves who they are stays with doc 78 and collaboration stage C4; the record shape lands now, so the data dictionary and the new layout carry it. |
| R55 | H | **One key per conversation cannot erase one person from it.** Doc 69 guests write messages and comments into the owner's conversation, and doc 74 allows person-scope erasure while rule 5 forbids per-entry deletion. Erasing guest B either destroys owner A's conversation or leaves B. | doc 69; doc 74 §1 rule 5, §3.4 | Each non-owner contributor's frames are encrypted under a (conversation, principal) key. Shredding it leaves the frames in place (the chain still verifies, R51), and `derive_messages()` shows a typed "removed at a participant's request" placeholder. Rule 5 states this one exception; invariant 1 already needs the same qualification for a whole erased conversation. |
| R56 | M | **Keyed object ids confirm files between people in one tenant.** The HMAC id stops cross-tenant confirmation, but any scope can test whether a file exists elsewhere in its tenant. That is harmless for one owner and a leak once Members and guests exist. | doc 73 §3, §7.3; doc 74 §3.6 | Doc 73 §7.3 states the limit. When a tenant gains a second person, scope the id key per space (dedupe narrows to a space). Decide with M7's roles. |
| R57 | M | **Erasure is ambiguous for artifacts made across conversations.** An artifact's versions come from several conversations and travel by attachment (doc 82 §6). Doc 74 §4 step 3 erases "artifacts produced and not promoted elsewhere", which does not say what happens to a Saved artifact with one version from the erased conversation. | doc 74 §4; doc 82 §2, §6, §8 | A version made in conversation C holds only C's key grant until it is Saved, starred or shared, when the artifact scope takes a grant of its own. Erasing C erases its unsaved drafts and every label quoting C (field-encrypted under C's key); Saved versions survive as the person's document, with the lineage edge to C tombstoned. The receipt lists both. |

### Sequencing and scope

| ID | Sev | Finding | Evidence | Fix |
|---|---|---|---|---|
| R58 | H | **The strict chain gates the roadmap.** Doc 76 waits on M6, doc 80's stored content on M2 and M7, doc 81 on M6 and M8, doc 82 L3–L5 on M4, M6 and M8, doc 79's durability on M2, M7 and M9, collaboration C4 on principals. The plan runs M1 → M2 → M3a → M3b → M4 → M6 → M7 → M8 strictly, though M2 has no vak dependency and M8 does not need M7. | plan §4; dependency notes in each doc | Resequence (§3). |
| R59 | M | **M3b is too large for one change at this rate.** Baseline, purge, layout, classes, slim ledgers, runtime out of the tree, space identity, registry, docs and site in one XL change. | plan M3b | Slices on the 6.0 line (R38), each with its own tests: (1) baseline, purge and the runtime root; (2) sessions on segments and slim ledgers; (3) side ledgers to record chains, and Documents; (4) runtime out of the project tree and sandbox grants; (5) space identity, R12's list; (6) docs, site and services. 6.0.0 is released after slice 6. |
| R60 | M | **Pieces' storage has no class.** A piece declares KV, a SQLite database, snapshots and time series: mutable state owned by a piece, with its own retention. Workspace and Document do not fit. | doc 81 §5, §8 | Doc 73 §5 adds an **Application** class: a mutable store per owner, keyed per piece, always backed up, erased with its piece or Space or with an erased `derived_from` source. A piece's declared retention is a label on the piece node. |
| R61 | M | **Labels attach only to tenant, space, Agent and conversation.** Drafts fade by label (doc 82), pieces declare retention (81), sources bound items (76), connections and routines need it (80). | doc 74 §3.1 | Labels attach to any labelable catalog node (artifact, piece, source, connection). Resolution is unchanged: the longest `retain_for`, the shortest `delete_after`, holds over everything. |
| R62 | M | **Doc 74's client screens predate doc 75's plain words.** "Space", "Runs", "settled / scrubbed", storage byte counts and ids appear on everyday screens. | doc 74 §6.3; doc 75 §7–8 | Add the data vocabulary to doc 75's glossary before the M3b and M7 screens, and give each doc 74 screen an everyday view and a technical one. "Space" needs a person-facing word (decision 7). |
| R63 | L | **The telemetry test can miss short content.** "No string over 64 bytes outside an allowlist" lets a filename, URL or short prompt through; doc 79 §6 counts all of those as customer content. | review R3; plan M5; doc 79 §6, scenario 4 | `telemetry_carries_no_content` checks field names against an allowlist and plants doc 79's canaries (secret, filename, URL, prompt) in every error and tool path. |
| R64 | L | **"Catalog" now means two things in code.** The plugin marketplace shipped as catalogs (`enabled_catalog_entry`, `catalog-staging`, doc 39), next to the model catalogue, card catalogue and "More tools" catalogue. | `vak-server/src/lib.rs:10485-10497`; doc 39 | Name the doc 73 store the **data catalog** everywhere, or the crate `vak-index`. |

## 2. What stays as written

- The model split, the ten classes (plus Application, R60), one
  reconciler, one catalog, catalog before lifecycle, cloud as a remote.
- Decisions L1, L2, L4 and L5. L3 changes only its number (R37).
- Per-entry AEAD, per-object keys with grants, keyed object ids,
  conversation keys spanning rotation, two-step deletion.
- The scenario matrix, which should gain one row per new primitive: a
  webhook trigger, a cursor resync, an effect with an unknown outcome, a
  restore that must fence the old writer, and a guest's erasure from a
  shared conversation.
- Doc 82's phasing: L1 and L2 before the data work, as a prototype whose
  data M3b discards.

## 3. Proposed order

```
M1 ids, trace key, principals ──┬─▶ M3a scope API ──┐
M2 storage substrate ───────────┴───────────────────┴─▶ M3b data baseline (6.0, in slices)
        M5 telemetry (after M1, in parallel)               │
                                                           ▼
                              M4 runs, triggers, effects, fencing
                                                           │
                                                           ▼
                                                M6 catalog ─┬─▶ M6.5 intake (doc 76)
                                                            ├─▶ M8 artifacts, sharing
                                                            └─▶ M7a ─▶ M7b ─┬─▶ M9 remote
                                                                 (M8 too) ──┘
```

- **M1 and M2 in parallel.** M2 has no vak dependency. It must absorb R48–R53
  before it is written.
- **M3a straight after M1** (it needs `TestScope`), in parallel with M2. It is
  the milestone that loses ground each week (R41); the ratchet holds the line
  until it starts.
- **M4 widens** to triggers, cursors, effects and fencing (R45, R48).
- **M8 and M7 in parallel after M6.** Artifacts do not need lifecycle; erasure
  needs the artifacts' lineage, so M7 covers artifacts from its first commit.
- **M7 splits.** M7a: the reconciler, trash to erase for conversations,
  backup rework and quotas (honest deletion). M7b: labels, holds, person,
  space and tenant scopes, roles and the governance screens. L2's bar for a
  first customer release still needs both.
- **M6.5 intake** enters the plan as doc 76 D3 recommends.

## 4. Decisions for the maintainer

| # | Decision | Recommendation |
|---|---|---|
| 1 | The data baseline's version | The next major at M3b, 6.0.0. Plan §1 holds the number; other docs cite the plan. |
| 2 | Releases during M3b | Main becomes the 6.0 line at M3b's first slice; 5.x fixes go on `release/5` until 6.0.0. |
| 3 | Triggers and `TaskDef` (doc 81 §20.5) | `TaskDef` grows into one Trigger model in M4, with cursors, effect records and fencing beside it. |
| 4 | Order of the critical path | §3: M1 ∥ M2, M3a early, M8 ∥ M7, M7 split, M6.5 added. |
| 5 | Principals | `prn_` ids with `actor` and `on_behalf_of` on the trace key, in M1. |
| 6 | An Agent's workspace after M3b | A Workspace bound to (Space, Agent), never an Environment. |
| 7 | The person-facing word for a Space | Settle it in doc 75's glossary before the M3b screens. |
| 8 | Small 5.x changes now | Land the R41 ratchet and the R42 registry split now; record D25 in doc 73 and fix it in M3b. |

All eight were accepted on 2026-10-01 as recommended. They are plan
decisions L3 (re-locked) and L6–L12.

## 5. Where each fix landed

| Finding | Applied in |
|---|---|
| R37 | plan §1 L3 (the only place the number is written); doc 73 status and §13; docs 76 and 82; blast radius; AGENTS.md "Pending" |
| R38, R59 | plan L6 and M3b's six slices; blast radius M3b; AGENTS.md "How to pick it up" |
| R39 | doc 76 status, §9 and D4; AGENTS.md proposal list |
| R40 | doc 73 Appendix A |
| R41, R42 | plan L12 and §4 "Now"; blast radius §0 (patterns and counts at `438cfcd5`) and "Now"; AGENTS.md "don't deepen the debt" |
| R43 | doc 73 §2 (D25), §5 rule; plan M3a and M3b slice 3; AGENTS.md known gaps |
| R44 | plan M2; blast radius M2 |
| R45 | plan L7 and M4; doc 73 §3, §8 (D27); doc 74 §2.5, §6.2, §7; docs 76, 80 and 81 |
| R46 | plan M1; reliable-work plan E0 decision 1; AGENTS.md "don't deepen the debt" |
| R47 | plan L10 and M3b slice 4; doc 73 §3, §5, Appendix A; doc 74 §2.6; doc 82 §10 question 3 |
| R48–R53 | plan §1 crypto list and M2; doc 73 §5, §6, §7.3, §8, §11, §11.1; doc 79 status |
| R54 | plan L9 and M1; doc 73 §3, §4; doc 82 §0; collaboration plan §11 |
| R55 | doc 73 §7.3; doc 74 rule 5, §3.4, §4; plan M7a |
| R56 | doc 73 §7.3; plan M7b |
| R57 | doc 73 §7.3; doc 74 §4 step 3; plan M8 |
| R58 | plan L8 and §4; doc 73 §14 |
| R60 | doc 73 §5, §6; doc 81 §8 |
| R61 | doc 73 §7.4; doc 74 §3.1; plan M7b |
| R62 | plan L11; doc 74 rule 9; doc 75 §7 |
| R63 | plan M5 |
| R64 | doc 73 §9 and plan §1 ("data catalog") |
