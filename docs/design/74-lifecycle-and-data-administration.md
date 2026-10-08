# 74 — Lifecycle management and data administration

Status: **proposal, revision 2 (2026-10-01).** Nothing here is shipped
except M0's trash. This is the companion to
`docs/design/73-data-architecture-and-lifecycle.md` (the data model) and to
`docs/plans/data-architecture-plan.md` (milestones, revision 3), and it
answers review findings R2, R8, R26, R28, R33 and R34 in
`docs/plans/data-architecture-review.md`, and R55, R57 and R60–R63 in
`docs/plans/data-architecture-review-2.md`. It defines how every kind of Vak
data is born, changes state, and ends, which policies govern that, how
people see and act on it, and which screens, APIs and commands that takes.

## 1. Rules

1. **People ask; the reconciler acts.** No screen, API or command deletes
   bytes directly. They create or approve a *request* (trash, erase,
   release, expire). One level-triggered reconciler performs every
   destructive transition, so policy, holds, leases and recording are
   applied in exactly one place.
2. **Every transition is a record** in `audit/lifecycle`: subject, from, to,
   cause (policy, request, hold, quota), actor, time and TraceKey. "Why is
   this gone?" always has an answer that contains no content.
3. **Preview before commit.** Every destructive action shows its impact
   first, computed from catalog lineage: counts per class, derived copies,
   bytes, and processors outside our reach. The commit carries the
   preview's digest; if the impact changed in between, the commit is
   refused and re-previewed.
4. **Holds beat everything**: user delete, retention expiry, quota
   pressure, Agent or space erasure, tenant offboarding.
5. **Records expire per conversation, never per entry.** Partial deletion
   would break `derive_messages()` and the hash chain. Non-content shared
   ledgers (cost, routing, activity) expire by sealed segment. The one
   exception is a non-owner contributor: their frames in a shared
   conversation have their own key (doc 73 §7.3), so erasing that person
   destroys only that key, leaves every frame in place and shows a typed
   placeholder where their words were.
6. **Recoverable before irreversible.** Trash is hidden everywhere and
   restorable. Erasure destroys keys and cannot be undone. The gap between
   them is policy (default 30 days).
7. **Receipts are honest.** An erasure receipt lists what was destroyed and
   what Vak cannot reach: messages already sent to Telegram, Slack or
   Discord, prompts already processed by an LLM provider, and an external
   NATS deployment.
8. **Screens show measured evidence only** (invariant 26) and filter by
   audience before anything else (invariant 37). Operations URLs keep
   `workspace` and `time` context, and every drill-down ends at raw
   evidence.
9. **Everyday words first** (doc 75 §7–8). Client screens (§6.3) use the
   words people recognise and keep ids, digests, paths and byte counts
   behind Show technical details. The data vocabulary (the word for a
   Space, run, trigger, trash, delete permanently) is settled in doc 75's
   glossary before the first screen that needs it (plan L11).

## 2. Lifecycles

Notation: a state list, the transitions (trigger → actor), the data effect,
and whether the step is reversible. Every transition writes a lifecycle
record (rule 2), so that is not repeated per row.

### 2.1 Tenant

`provisioning → active → suspended → offboarding → shredded`

| Transition | Trigger / actor | Data effect | Reversible |
|---|---|---|---|
| active → suspended | billing or admin | admission refused; data intact | yes |
| → offboarding | owner request | export window opens (default 30 d), schedules stop | yes, until shredded |
| offboarding → shredded | window end, no hold | tenant KEK destroyed; every record, object, Document and derived store removed; one retained tombstone | no |

### 2.2 Space

`bound → active → detached → archived → erased`

- **Detached**: every machine binding is removed. Data is kept, and the
  space can be re-bound. This replaces `workspaces/forget`, which forgot
  a path and left data scattered (deleted at plan M7a-e).
- **Archived**: read-only. Its schedules are disabled and its retention
  clocks start.
- **Erased** is an erasure request with scope *space* (§3.4). Files in the
  person's own working tree are never touched: Vak erases what it stored,
  never the directory a person owns.

### 2.3 Agent

`active → paused → archived → revoked → erased`

| State | Admission | Triggers | Endpoints | Data |
|---|---|---|---|---|
| active | yes | run | live | normal |
| paused | refused (fails closed, invariant 37) | skipped (a Run record says so) | live, replies "paused" | intact |
| archived | refused | disabled | detached | read-only; retention clocks start |
| revoked | refused | disabled | credentials cut **immediately**, bot tokens removed | intact under label; for "this Agent was compromised" |
| erased | — | deleted | deleted | erasure scope *agent*: conversations, Documents, runs, artifacts it owns |

`AgentLifecycle` (`crates/vak-core/src/agent_definitions.rs`) has
`Revoked` since plan M7a-g.
`erased` is not a state on the definition; the definition itself becomes a
tombstone.

### 2.4 Conversation and session

`open → idle → sealed → (archived) → trashed → erased`, and separately
`expired` by policy.

| Transition | Trigger / actor | Data effect | Reversible |
|---|---|---|---|
| open → idle | no turn for N minutes | none (leases released) | yes |
| idle → sealed | reconciler | open segment sealed; checkpoints pruned to baseline plus final | yes (appending re-opens a new segment) |
| → archived | person | hidden from the default list; searchable | yes |
| → trashed | person (own conversation) or admin | **hidden everywhere**: lists, every search, recall, prompts, exports, digests. Triggers that deliver into it pause. | yes, within the trash window |
| trashed → erased | window end, or "Delete permanently" | erasure (§4) | no |
| → expired | `last_activity + delete_after` with no hold | erasure (§4) with cause *policy* | no |

This replaces today's `archive.json`/`deleted.json` sidecars
(`crates/vak-server/src/lib.rs:9056-9210`), whose "delete" only hides from
two lists (review R8).

### 2.5 Run, trigger, effect and cursor

- **Run:** `claimed → running → settled{completed | failed | cancelled |
  abandoned | skipped}`. The claim is a CAS on `(trigger, slot | event id)`
  under the claimant's writer epoch. A lease that expires while running
  becomes `abandoned`. The Run record stays with its conversation's
  retention; a Run with no conversation (a skip, a script watchdog) follows
  the space's `runs` label.
- **Trigger** (doc 73 §8): `draft → enabled ⇄ paused → disabled → deleted`.
  - Deleting a trigger tombstones the definition; its Runs are kept.
  - Pausing records every slot that passes as `skipped{paused}`, so the
    history has no silent gap.
  - `on_crash = skip | retry_once` decides what an `abandoned` slot does.
- **Effect:** `prepared → dispatched → accepted | confirmed | failed |
  unknown → reconciled`. An `unknown` effect is reconciled from the
  provider, never replayed, including after a restore or handoff. Effect
  records are audit-adjacent: ids, digests, actor and receipt stay readable
  for the audit label; any content field (a message body, a subject) is
  keyed to its conversation like any other copy (§4).
- **Cursor:** `current → expired → resynced`. A resync is bounded and
  records the gap it could not fill.
- **Connection and grant** (docs 80, 81): `linked → active ⇄ needs
  reconnect → revoked`. Revoking stops every read, write, refresh and
  subscription at once; data already retained follows its own labels, and
  the disconnect screen says what remains.

### 2.6 Execution and environment

- **Execution:** `prepared → running → settled → scrubbed`. Scrubbing
  removes `runtime/executions/<exe>` once the execution settles (plus a
  grace period). Its stdout, stderr and produced files are objects that
  follow the owning conversation.
- **Environment:** reuses `vak_sandbox::EnvironmentState`: `planned →
  preparing → ready → running → stopped → expired → removed`. An Agent's
  workspace is never an environment: it is a Workspace bound to (Space,
  Agent) and lives as long as both (plan L10).
  - Expiry comes after the candidate is promoted or rejected, or after the
    idle TTL, whichever comes first.
  - A worktree environment also removes its branch.
  - Removing an environment never removes an unreviewed draft: the draft's
    files are objects or a frozen candidate, which follow the draft-version
    retention (§2.7, §3.1). A candidate moves to `rejected{expired}` only
    when that retention expires it, which is recorded and notified.

### 2.7 Artifact and version

`draft → candidate → verified → promoted | shared | published | rejected`
per version; the artifact itself is `active → superseded → expired →
erased`.

- A version is immutable. Promotion writes into a space's working tree
  through the existing Review path (invariant 39) and records the target
  digest.
- Sharing is a grant (§6, screen A6) and never copies data.
- Unpublishing revokes grants and links; external copies are named in the
  receipt.
- A draft version that is not accepted, starred or shared moves to the trash
  when the draft-version label expires it (default 60 days, §3.1),
  restorable for the trash window and then erased. Accepting, starring or
  sharing it stops the clock. The Library lists such drafts under "Older
  drafts" after 14 days; that is a view, not a state (`82-library.md` §3).

### 2.8 Documents (memory, entities, skills, prompt layers, presentation packs)

`active → amended (new version) → forgotten (tombstone)`.

- Every save keeps the previous content as a version, under the Document's
  history retention (default 90 days for memory and entities).
- **Forget** tombstones the current value and schedules history erasure
  once the history window ends.
- A Document whose `derived_from` names an erased conversation is erased
  with it (§4). A person may choose to *keep* a memory note by re-affirming
  it, which writes a new version with provenance `person`.

### 2.9 Checkpoints

`captured → retained → pruned`.

- Policy keeps the session baseline, the last N (default 20) while the
  session is open, and baseline plus final after sealing, for `delete_after`
  (default 30 days).
- Checkpoints are **space-owned**: erasing a conversation removes its
  manifests' *turn linkage* and labels, never the space files a person
  kept.

### 2.10 Delivery jobs and inbox

- **Delivery job:** an effect record of kind delivery (§2.5): `queued →
  attempting → delivered | dead-lettered → sealed`. When a job settles its record moves into the deliveries ledger
  and the job file is removed. Today delivered jobs are never removed.
- **Inbox entry:** `unread → acked → expired` (default 90 days). Entry
  bodies are content keyed to their conversation (§4).

### 2.11 Endpoints and allowlist

`pending → allowed | denied → revoked`. Pending and denied entries hold
personal data (sender ids, chat ids) and expire by policy (default 90 days).
A denied entry that expires is replaced by a hashed sticky-deny fingerprint,
so "a denied entry never re-prompts" (invariant 15) survives without keeping
the identifier.

### 2.12 Incidents, actions and audit

Incidents follow `open → resolved → reopened` (doc 28) and expire by policy.
Lifecycle, erasure, hold, grant and promotion records are **audit class**:
they are retained for the audit label's duration (default 7 years) and
**contain no content**, only ids, digests and reasons. That is what lets
them outlive the erasures they record.

### 2.13 Objects and keys

- **Object:** `live (≥1 key grant or ref) → orphaned → grace → collected`.
- **Conversation key:** `active → destroyed`. Destruction *is* erasure.
- **Tenant KEK:** `active → rotating → active`. Rotation re-wraps the
  keys; it never re-encrypts content.

### 2.14 Telemetry, caches, runtime

These are ephemeral or telemetry classes with no per-item lifecycle, only
size and age limits and a boot sweep of `runtime/`. They never hold content
(plan M5), so erasure never needs them.

### 2.15 Connected provider accounts

Provider connections are Agent-owned Desired state; credentials and account
principal/display data live only in the credential vault. Durable records use
an opaque connection id and retain account lineage on every fetched object,
candidate, excerpt, citation, automation cursor, and derived/indexed copy.
Authorization also carries the owning Agent and audience, and is rechecked at
each broker read or effect.

The lifecycle is `connected → paused | revoked → erase requested → erased`.
Disconnect/revoke immediately removes the secret, fences watchers and
schedules, and cancels undispatched work; it does not claim to erase retained
content. Account deletion is an erasure request scoped to that connection id.
The M6 catalog must find copies across Agents and conversations; M7 must
selectively crypto-shred account-derived data, remove derived plaintext and
backup restore paths, preserve unrelated conversation content, and issue a
receipt. The key/grant design for selective erasure is an M2/M6 exit gate.
Provider systems, recipients, and other processors outside Vak's control are
listed in the receipt as external copies that Vak cannot erase.

## 3. Policies

### 3.1 Retention labels

```
RetentionLabel {
    id, name, description,
    rules: [{ class, retain_for?: Duration, delete_after?: Duration,
              on_expiry: erase | archive | review }],
}
```

- **Attached to** any labelable catalog node: a tenant, space, Agent,
  conversation, artifact, piece, source or connection. It inherits
  downward, and a node may *break* inheritance and attach its own label, as
  with SharePoint labels. A piece's declared snapshot retention (doc 81 §5)
  and a source's item bound (doc 76) are labels on those nodes, so they
  resolve with everything else and never outlive a hold or a shorter
  `delete_after` above them.
- **Resolution per class:**
  - the minimum keep is the **maximum** `retain_for` along the chain;
  - the maximum keep is the **minimum** `delete_after`, but never below the
    minimum keep;
  - holds suspend expiry entirely.
  A person can make data live longer by labelling. Nobody can make a hold
  shorter.
- **`on_expiry = review`** queues the item for a data steward instead of
  acting, which is useful for regulated spaces.
- **Defaults** (tenant label `default`):

| Class | retain_for | delete_after |
|---|---|---|
| conversation records + content objects | — | none (keep) |
| trash window | — | 30 d |
| runs (no conversation) | — | 180 d |
| checkpoints (sealed session) | — | 30 d |
| environments | — | 7 d after settle |
| draft versions (not accepted, starred or shared) | — | 60 d, then the trash |
| Document history (memory, entities) | — | 90 d |
| inbox entries | — | 90 d |
| pending/denied allowlist entries | — | 90 d |
| cost / routing / activity segments | 90 d | 400 d |
| incidents | — | 400 d |
| audit class | 7 y | 7 y |
| telemetry | — | 14 d or 200 MB |

As built (plan M7a-d, 2026-10-08): run records and allowlist entries have
no rule yet. Runs of no conversation share segments with the rest and are
kept until M7b's audit rules; allowlist entries wait for M7b's rules about
people. Rows in a record chain leave a sealed segment at a time, so a row
can outlive its rule by up to the 30 days a quiet chain's segment stays
open.

### 3.2 Legal hold

```
Hold { id, scope: Tenant | Space | Agent | Conversation | Audience(person) | Query(catalog filter),
       reason, created_by, created_at, released_by?, released_at? }
```

A hold blocks every destructive transition on anything in scope, including
items that enter the scope later (a Query hold is re-evaluated each tick).
Creating and releasing a hold are audit records, and releasing needs a
reason. A person whose trash is held sees "retained by your organisation's
policy" when the tenant allows that disclosure, and otherwise the item simply
stays in trash.

### 3.3 Quotas

- **Soft quota:** a notification and a Data-posture warning.
- **Hard quota on Ephemeral or Derived data:** evict caches and derived
  stores, which are rebuildable.
- **Hard quota on Record, Object or Document data:** new work is *refused
  at admission* with a clear error. It fails closed and never silently
  evicts records.
- Quotas attach at the tenant, space or Agent.

### 3.4 Erasure scopes and approval

| Scope | Who may request | Approval |
|---|---|---|
| conversation | its owner (a person, for their own), or a steward | none for the owner after the trash window; steward otherwise |
| audience / person (data-subject request), including their contributions to other people's shared conversations | steward | two-person when a second admin exists |
| Agent | owner or steward | two-person |
| space | owner or steward | two-person |
| tenant | owner | two-person, plus the offboarding window |

A request moves `previewed → approved → executing → completed | blocked{hold}
| failed{reason}`. Execution is resumable: the reconciler checkpoints its
progress in the request record.

### 3.5 Export and portability

Export can be scoped by conversation, person, Agent, space or tenant. The
bundle contains plaintext records (JSONL), objects, Documents, the manifest
and signatures, and is itself a Run with a receipt. It is what
offboarding, portability requests and `vak data export` all produce.

### 3.6 Roles

Today the admin console has a single operator credential. A customer space
needs the following, which M7b introduces:

| Role | Can |
|---|---|
| Owner | everything; tenant-level actions |
| Steward | retention labels, holds, erasure requests and approvals, audit export |
| Operator | runs, schedules, reconciler actions, integrity, storage views |
| Member | their own conversations: trash, restore, export, delete permanently where policy allows |
| Auditor | read-only audit, lifecycle and erasure receipts; no content |

Full RBAC for the rest of the product is a separate design. Until it lands,
a single-operator install holds Owner, and the two-person rules degrade to a
typed confirmation plus a delay (default 24 h, cancellable).

## 4. Erasure mechanics

Erasure is only complete if every copy of the content is keyed or linked.
Two rules make that true:

- **Content is keyed to its conversation wherever it is written.** A
  content field in a shared ledger (inbox `body`, delivery text, outbox job
  payload, commitment statement, turn-card text) is field-encrypted under
  the conversation key. Non-content fields (ids, timestamps, states) stay
  readable for audit and scheduling. (Plan §M7a "M7a design", 2026-10-08: such content is stored as a tenant object granted to the conversation's scope instead, and the row keeps a reference.)
- **Derived writes record `derived_from`.** Memory notes, entities, skill
  proposals, FTS rows and embeddings name their source conversation and turn
  (plan M1).

An erasure of scope S then runs these steps:

1. **Resolve** S to its conversations and source records through the catalog
   (for a person: every conversation whose audience includes them; for a
   provider account: every source and descendant carrying that account's
   lineage, including copies in multiple Agents and conversations).
2. **Check holds.** Any hold in scope blocks the whole request, recorded as
   `blocked{hold}` with the hold ids.
3. **Walk lineage outward** from the source records and affected
   conversations: Documents with `derived_from`, artifacts produced and not
   promoted elsewhere, catalog rows, embeddings, key grants, run and effect
   records, delivery jobs and inbox entries. For artifacts, an unsaved,
   unstarred and unshared version holds only its conversation grant and is
   erased with it. A Saved version holds its own artifact grant and survives
   as the person's document; tombstone its lineage edge to the conversation
   and erase any label quoting that conversation (doc 73 §7.3).
4. **Destroy the relevant scope keys.** For conversation erasure, destroy its
   conversation key, or the contributor key for one person's contributions
   to another's conversation. For provider-account erasure, destroy the
   account-scoped grants/key so provider-derived content becomes unreadable
   without destroying unrelated conversation content. The exact wrapping
   and grant composition is an M2/M6 exit requirement.
5. **Remove derived plaintext:**
   - catalog rows, with `secure_delete` and a WAL checkpoint;
   - embeddings;
   - Document versions derived only from the erased scope (a version with
     another live source is re-derived or kept, and the receipt says which);
   - checkpoint labels and turn links.
6. **GC objects** whose last key grant was destroyed.
7. **Write the receipt:**
   - counts per class, and a digest of the erased identifiers;
   - the processors outside reach, with timestamps: channel deliveries from
     `deliveries`, provider dispatches from `WorkReceipt`, the bus if
     configured;
   - backups that still hold ciphertext, and when they expire.
   The receipt is an audit-class record, signed, and downloadable.
8. **Restore guard.** A backup restored later re-applies every erasure
   tombstone from the audit record before any restored data becomes
   readable (plan M7a).

Headless honesty (doc 73 §7.3): where the KEK sits in the encrypted-file
fallback, whose key lives beside it, at-rest encryption is nominal.
Crypto-shred still works because it destroys the conversation key.

## 5. The reconciler

```
loop every tick (default 10 min; hints wake it early; one pass at boot):
  observe   catalog + refs + policy + holds + quotas + leases   (read-only)
  plan      desired transitions, ordered: runtime scrub → env expiry →
            checkpoint prune → trash/expiry erasure → document history →
            segment seal/tier → object GC → quota eviction
  guard     drop any action whose subject is held, leased by a live turn,
            or whose preview digest changed; record why
  stage     quarantine: move to lifecycle/quarantine/<action>/ (same fs)
  commit    make the change permanent after the stage grace (0 for ephemeral,
            1 tick for everything else)
  record    one lifecycle record per transition; update metrics
```

- **Idempotent and crash-safe.** Every action is keyed; a crash between
  stage and commit resumes; nothing is committed that was not staged.
- **Paced.** An IO budget per tick (bytes and files) and per-tenant
  fairness. It never runs a heavy phase while a turn in the same space is
  mid-dispatch.
- **Dry-run is the same plan, printed.** `vak data gc --dry-run`, the
  admin Lifecycle screen, and `doctor` all show the identical plan object.
- **Undeclared paths** found under a Vak-owned root are quarantined for one
  cycle and listed as incidents. That is a code defect to fix, so the
  quarantine exists to make it visible, not to hide it.
- **Metrics:** last tick, duration, actions by kind, bytes reclaimed,
  blocked by hold, errors. These are shown as measured values only.

## 6. Admin console

### 6.1 Navigation

The existing groups stay (`crates/vak-admin-ui/src/App.tsx:6862-6945`).
Changes:

| Group | Today | Proposed |
|---|---|---|
| Overview | Home, Inbox | Home (adds **Data health**), Inbox |
| Work | Sessions, Commitments | **Conversations** (renamed; lifecycle-aware), **Runs** (new), **Library** (new, M8), Commitments |
| Operate | Operations Center: Posture, Live work, Runtime & pools, Channels & delivery, Automations, Providers, incidents, sandbox | the same, plus **Triggers** (moved from Configure › Integrations › Scheduled tasks), and a **Data** section: **Storage**, **Lifecycle**, **Integrity**, **Sync** (M9) |
| Configure | Integrations, Gateway, Permissions & security, Prompts, Memory, Settings | the same, plus **Governance** (**Retention & holds**, **Erasure requests**), **Spaces**, **Agents** (lifecycle panel), Security › **Keys** |
| System | Setup, FinOps | Setup, FinOps (per Agent and per run), **Backup & restore**, **Diagnostics › Traces & logs**, Security › **Audit export** |

As built at plan M8.4c-e (2026-10-08): Work is Conversations, Runs,
Library, Automations and Commitments. Automations (the word chosen for
triggers) are made and edited there, and Operations keeps an Automations
tab for what is running.

`#/operations/work/runs/<session_id>` is replaced by `#/runs/<run_id>`
(review R13). Trigger definitions (today's scheduled tasks) stay editable
under Operate › Triggers; there is one place for triggers, not two.

### 6.2 Screens

Every screen follows rule 8: measured evidence only, audience-filtered, URL
carries `workspace` and `time`, drill-downs end at raw records, objects or
receipts. "Guard" names the confirmation a destructive action needs.

| # | Screen | Shows | Actions (guard) | Milestone |
|---|---|---|---|---|
| A1 | Home › Data health | storage by class, quota pressure, reconciler last tick and errors, open erasure requests, active holds, integrity status | links only | M7a |
| A2 | Work › Conversations | the list with facets: Agent, space, audience, state (open/idle/sealed/archived/trashed), label, hold, size, last activity | archive, trash, restore, export, request erasure (preview → typed confirmation), place hold (steward) | M0 (trash honesty), M7a |
| A3 | Conversation detail | tabs: Transcript, Forensics (existing `SessionForensics.tsx`), **Lineage** (graph from catalog edges), **Lifecycle** (effective label with the chain that produced it, holds, expiry date, transition history), Receipts | same as A2, per item | M6, M7a |
| A4 | Work › Runs | Actions-style table: run id, cause, Agent, space, trigger and slot or event, status and decision (with skip reason), duration, cost, started | open, cancel (if running), re-run (creates a new Run with cause *manual*) | M4 |
| A4b | Run detail | timeline: span waterfall (M5) and records; sessions; executions with stdout objects; artifacts produced; deliveries; receipts; cost | open any linked node | M4, M5 |
| A5 | Operate › Triggers | list by kind (schedule, event, webhook, watch, source poll): next slot (in the trigger's timezone), last decision, 30-day success rate, skipped reasons, `on_crash`; editor for definitions | pause/resume, run now, backfill a missed slot (explicit, recorded), edit | M4 |
| A6 | Work › Library (the operator view of `82-library.md`'s model) | artifacts by space, kind and audience; version history; promotions; shares (inherited vs broken); comments; label | share / unshare (grant dialog showing inheritance), promote (Review path), request erasure | M8 |
| A7 | Operate › Data › Storage | bytes and counts by class × tenant/space/Agent; growth over time (daily catalog snapshots, measured); top consumers; dedupe ratio; cache and telemetry sizes vs quota | set a quota (operator), open the consumer | M7a |
| A8 | Operate › Data › Lifecycle | reconciler status; the **pending plan** (the dry-run) grouped by action with reason and policy; recent transitions; quarantine; errors | run a tick now, pause the reconciler (recorded; stewards are notified), retry a failed action | M7a |
| A9 | Operate › Data › Integrity | chain verification per record chain (last full and sampled), catalog staleness digest, object verification, key status, undeclared-path incidents | verify now, rebuild catalog (mechanical, invariant 19) | M6, M7a |
| A10 | Configure › Governance › Retention & holds | labels (rules per class); the inheritance tree tenant → space → Agent → conversation with effective policy and breaks; **impact preview** (what expires in 7/30/90 days); holds with scope, reason and items covered | create/edit label (preview impact; typed confirmation when it shortens anything), attach/break, create hold, release hold (reason required) | M7b |
| A11 | Configure › Governance › Erasure requests | queue with status; the preview (counts per class, derived items, processors outside reach, backups still holding ciphertext); progress; receipts | create (scope picker), approve (second person), cancel before execution, download receipt | M7b |
| A12 | Configure › Security › Keys | the key authority (keychain, encrypted-file with the honest note, or a KMS) and its health, last rotation, escrow bundles issued; no key material ever shown | rotate KEK, issue escrow bundle (typed confirmation; recorded) | M7b |
| A13 | Configure › Spaces | spaces with per-machine bindings, trust state, git state, default label, quota, size | bind/unbind, trust, archive, request erasure | M3b, M7b |
| A14 | Configure › Agents › Lifecycle panel | state, and the data effects of the next transition (schedules affected, endpoints cut, data retained) | pause, resume, archive, revoke (immediate; typed confirmation), request erasure | M7a |
| A15 | Configure › Security › Audit export | range and scope; record kinds included; bundle signature | create export (a Run with a receipt) | M7b |
| A16 | System › Backup & restore | backups (ciphertext; key grants wrapped; whether escrow is included); restore dry-run showing erasures that will be re-applied | create, restore (typed confirmation), expire | M7a |
| A17 | System › Diagnostics › Traces & logs | search by run id or trace fields; span waterfall; log tail filtered by trace; telemetry quota | none (read-only) | M5 |
| A18 | Operate › Data › Sync | remotes, push/pull lag, pending objects and segments, leases held, conflicts, last error | push/pull now, release a stale lease (recorded) | M9 |
| A19 | System › FinOps | existing charts, plus per Agent and per run (cost rows now carry both) | existing | M1 (data), M4 (views) |
| A20 | Configure › Agents › Mail and calendar connections | provider, masked identity resolved from vault at request time, granted capabilities, audiences, schedules/watchers, freshness, retention label, pending effects and last erasure receipt | pause, reconnect, revoke, request account erasure (impact preview → typed confirmation) | package UI; M7 erasure |

### 6.3 Client app (people, not operators)

| # | Place | Shows | Actions |
|---|---|---|---|
| C1 | Conversation menu | state and label badge | Archive, Move to trash, Export, Delete permanently (when policy allows the owner; otherwise "Ask an admin", which creates an erasure request) |
| C2 | Trash | trashed conversations with days left | Restore, Delete permanently |
| C3 | "Why is this gone?" | shown when a link or search hits a tombstone: when, by which policy or request, whether a receipt exists; never content | open the receipt (if the viewer may) |
| C4 | Library (a sidebar destination of its own; specified in `82-library.md`, which supersedes the panel described here) | artifacts this person can see; versions; comments | open, compare versions, share, promote (through Review) |
| C5 | Share dialog | current access (inherited vs explicit), roles | grant, revoke, break inheritance (named as such) |
| C6 | Runs panel per Agent | routine history with skip reasons and deliveries (replaces `TasksModal` last-run fields) | run now, pause (if permitted) |
| C7 | Settings › Your data | what Vak keeps about this person, per class; retention that applies; storage used | export all, trash all, request erasure |
| C8 | Workbench | execution states (running / settled / scrubbed), draft expiry | star (the Library's star, which keeps a draft from fading, `82-library.md` §3), reject |

## 7. API

All routes sit under the existing authentication. Mutations follow
invariant 34 (cookie mutations need a recognised `Origin`; headerless
clients need a real token). Destructive routes also require a
`confirm_digest`, the preview digest the caller just displayed, so a stale
preview can never authorise a different impact.

```
GET    /data/status                        classes, quotas, reconciler, integrity summary
GET    /data/usage?scope=&class=&since=    measured usage and growth
GET    /data/lifecycle/plan                the dry-run plan
POST   /data/lifecycle/tick                run now (operator)
GET    /data/lifecycle/transitions?…       lifecycle records
GET    /data/integrity   POST /data/integrity/verify   POST /data/catalog/rebuild
GET|POST|PUT  /data/labels[/{id}]          labels; POST /data/labels/{id}/attach, /detach
GET    /data/labels/effective?node=        resolved policy + chain
GET|POST      /data/holds  POST /data/holds/{id}/release
POST   /data/erasure/preview               → { preview, confirm_digest }
POST   /data/erasure                       { scope, confirm_digest } → request
POST   /data/erasure/{id}/approve | /cancel   GET /data/erasure/{id}[/receipt]
POST   /data/export                        { scope } → Run
GET|POST      /data/keys  POST /data/keys/rotate  POST /data/keys/escrow
GET|POST      /data/backups  POST /data/backups/{id}/restore/preview | /restore
POST   /conversations/{id}/trash | /restore | /archive | /unarchive
GET    /runs  /runs/{id}  /runs/{id}/spans   POST /runs/{id}/cancel | /rerun
GET    /triggers  /triggers/{id}/slots   POST /triggers/{id}/pause | /resume | /run | /backfill
GET    /effects  /effects/{id}           POST /effects/{id}/reconcile
GET    /search  /lineage/{node}  /nodes/{id}
GET    /library  /library/{key}             the Library (doc 82): from its L1; keys change at M8
GET    /data/sync …  (M9)
```

These replace, in the same change (invariant 30):
- `POST /sessions/{id}/archive`, `DELETE /sessions/{id}`,
  `DELETE /sessions/archived`, `POST /sessions/{id}/restore`,
  `GET /sessions?trash=true` (M0's trash, `vak_core::trash`)
- `POST /memory/cleanup`
- `POST /workspaces/forget`
- `POST /backup/export|import`
- `/agents/{id}/schedule|runs` (already deleted in M0)
- `/tasks`, `/tasks/{id}/run-now`, `/tasks/{id}/retry-delivery` (by
  `/triggers` and `/effects` at M4)
- the session-keyed Operations "runs"

## 8. CLI parity

`vak data status | usage | plan | gc [--dry-run] | verify | rebuild-catalog |
labels … | hold … | erase --scope … [--preview] | export --scope … | keys … |
backup … | cat | grep`, plus `vak runs …`, `vak triggers …` and `vak effects …`.

Every destructive command prints the preview and requires `--confirm
<digest>` (or an interactive typed confirmation). `vak doctor --repair` may
invoke only reconciler actions classed as mechanical (invariant 19).

## 9. Acceptance

- **Per screen:** a browser acceptance run (AGENTS.md acceptance contract)
  on a `/tmp` workspace shows real data, and each drill-down reaches raw
  evidence. A test asserts no screen renders a value that has no API source
  (invariant 26).
- **End-to-end scenarios:**
  - person trashes → restores → deletes permanently, and search, recall
    and prompts never show it again;
  - a steward erases a person across three chats and two Agents, and the
    receipt lists the Telegram deliveries and provider dispatches it could
    not reach;
  - a hold placed mid-erasure blocks it;
  - label shortening previews its impact and needs confirmation;
  - a restore re-applies erasures;
  - an Agent revoke cuts its endpoints within one tick while its data stays;
  - a steward erases a guest from the owner's shared conversation: the
    owner's messages stay, the guest's show the placeholder, and the chain
    still verifies;
  - erasing the conversation that made one version of a Saved artifact
    keeps the Saved version and removes the unsaved drafts;
  - a paused schedule records every skipped slot.
