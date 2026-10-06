# 81 — Pieces and Home: kept, live and hosted work

Status: **proposal, 2026-10-01. Nothing in this document is shipped.** It
records a direction discussed with the maintainer and authorizes no
implementation milestone. It builds on the data architecture
(`73-data-architecture-and-lifecycle.md`, `74-lifecycle-and-data-administration.md`),
intake (`76-intake-and-knowledge.md`), mail and calendar
(`80-mail-and-calendar.md`), the Canvas (`66-immersive-artifact-canvas.md`),
task environments and promotion (`54-task-environments-and-promotion.md`), the
learning loop (`26-learning.md`), the headless fleet
(`79-private-headless-fleet.md`) and the multi-machine proposal
(`56-personal-multi-machine-system.md`). Where it needs a change to a
non-negotiable invariant, §18 names the change; nothing here is behaviour
until that change is made.

## 0. Why

Vak makes things: a report, a spreadsheet, a research write-up, a dashboard,
a small app, sometimes a whole system. Each takes many steps — finding the
source, cleaning the data, writing the fetch code, choosing the layout, three
dead ends — and today only the final file survives as a result. The scripts,
the sources, the queries, the decisions and the fixes sit in
`.vak/scratch/` or in the session ledger, which is an audit record, not
something Vak can pick up again.

So the next ask rebuilds it. "Update the dashboard" a week later, or "the
weather again" for the tenth time today, costs the same tokens as the first
time, may make different choices, and returns something slightly different.
Work is recorded but never compounds.

Meanwhile what people want to keep is increasingly *live*: a Stack Exchange
monitor they glance at every day, the weather, the next three meetings, mail
that matters, mentions on social media. These belong on a screen that stays
current between conversations — not in a chat transcript.

This document turns finished work into a **Piece**: the result, plus
everything needed to reuse, refresh, change and run it, hosted by a small
platform inside Vak and shown on **Home**. It covers what people make, how a
piece is kept, how it runs without a model, how its network, storage and
secrets are managed, how the platform itself is operated, and what changes on
a headless server.

## 1. Principles

1. **The model builds; code runs.** The model is a compiler. It does the
   expensive, creative work once and produces ordinary code and data. Every
   later run executes that code with no model and no tokens. The model
   returns only to **change**, **repair**, **judge** (a step the owner asked
   to be judgement) or **explain**.
2. **The lightest level that does the job.** Nobody chooses an architecture.
   The agent picks the smallest level on the ladder (§3); a piece moves up
   when asked and is never rebuilt to do so.
3. **Describe, don't hand-build.** Every piece is a declarative description
   plus files. The platform reconciles reality to the description, so any
   piece can be rebuilt, moved, restored or handed over from its folder.
4. **No unmanaged network, not no network.** Pieces reach the internet, the
   owner's accounts and each other — always through gateways that enforce a
   declared policy, add credentials, and log what crossed.
5. **Two depths, one truth.** A person who is not technical sees plain
   statuses, plain actions and plain questions. A technical person can open
   the same objects to their code, logs and limits. There is never a second
   system for either.
6. **Honest freshness.** Every value shows when it was true. A broken or
   stale piece says so and keeps its last good data clearly dated; it never
   passes old data off as current.
7. **The agent operates; the owner decides authority.** The agent notices,
   explains, repairs and proposes. Anything that widens what a piece may do
   — a new site, a new account, a new secret, an AI binding, a new effect —
   is the owner's yes.
8. **Portable and owned.** A piece is a folder. It can be exported, backed up,
   moved between machines, and shared, and it never depends on hidden state.

## 2. Vocabulary

- **Piece** — a kept unit of work at some level of the ladder (§3). It has a
  stable `pce_` id (full UUIDv7, doc 73 identifier rules), an owner, a
  description, immutable **versions**, and one current version (a ref).
- **Description** — `piece.toml`, the declarative file stating what a piece
  is, runs, reads, stores, shows and may spend (§5).
- **Draft** — work still in scratch. Not a piece yet.
- **Keep** — the promotion of a draft into a piece (§4). In everyday words,
  "Keep" or "Pin to Home".
- **Job** — code that runs on a trigger and exits.
- **Service** — code that stays running (a stream listener, a watcher).
- **Function** — code a view or another piece calls on demand.
- **Trigger** — what starts a job: a schedule, opening a stale tile, an event,
  a webhook, or a person pressing Refresh.
- **Run** — one execution of a job or function, with its inputs, outputs,
  check results, egress record and cost (the Run record of doc 73).
- **Check** — a deterministic test a run must pass for its output to become
  current (the page renders, the totals are not empty, the schema matches).
- **Freshness promise** — how current a piece says it will be ("updated
  within 6 hours"). Missing it is a status, not a silence.
- **View** — what a piece shows: a document, a page, a board. Every piece has
  one **tile** (its Home summary) and may have full views opened in the
  Canvas.
- **Binding** — a resource a piece declares: storage, an AI model, another
  piece, a connection.
- **Connection** — an account linked once at platform level (mail, calendar,
  a social account, a provider API). Pieces receive **grants** to it.
- **Gateway** — the egress gateway (every outbound request) and the ingress
  gateway (every inbound request, including views and webhooks).
- **Host** — a machine running the piece platform: the laptop, an always-on
  box, a cloud VM. **Placement** is the choice of host for a piece.

## 3. The ladder

| Level | Examples | Behind it |
|---|---|---|
| **Document** | a report, a letter, a spreadsheet, a deck | the file and how it was made |
| **Live document** | a weekly report whose tables refresh, stamped "as of Monday 09:00" | the file, data blocks bound to sources, a refresh job |
| **Tile** | weather, a price, "my next meeting" | one saved call and its card |
| **Micro app** | the Stack Exchange board, a mentions watcher, a reading tracker | views, jobs, functions, storage, gateway policy |
| **System** | a product with code, a repo, tests, several services | a project, possibly deployed to external cloud services (§16) |

Every level shares the lifecycle (§4) and the description format (§5). A
document's description is a few lines; a system's is long.

Moving up is an ask, not a rebuild:

- "Make this refresh every Monday" turns a document into a live document.
- "Let me filter it by tag" turns a live document into a micro app.
- "Turn this into a real service with a database and tests" turns a micro app
  into a system.

Moving down is retirement of the parts no longer needed, recorded as a
version.

A **tile** deserves its own note. The ledger already records each call's tool
and arguments exactly. A tile pins the **call**, not the answer: refreshing
it repeats the call and draws the same card from the new result. Asking for
the weather ten times a day becomes ten API calls and no model calls.

## 4. Lifecycle

```
 draft ──keep──▶ live ──change──▶ candidate ──promote──▶ live (new version)
   │               │                                       │
 (scratch)       pause / resume                         rollback
                   │
                 retire ──▶ archived (restorable) ──▶ erased (doc 74)
```

### 4.1 Keeping: from scratch to a piece

Keeping is more than saving: the owner is approving code that will run while
they are not watching. It happens only when the owner asks ("keep this",
"pin this", "make this live") or accepts the agent's offer. The agent may
offer when a draft looks reusable; it never keeps anything on its own.

When asked to keep, the agent **compiles** the draft:

1. **Separate the repeatable from the creative.** Fetching, cleaning,
   computing and laying out become code in the piece. The conversation, the
   dead ends and the exploration stay in the ledger, linked by provenance.
2. **Name the inputs.** What a person would change — date range, region, tag,
   city, source file — becomes `inputs` with defaults.
3. **Name the needs.** Every site the code reaches, every connection, every
   secret by name, any AI binding with its budget.
4. **Write the checks.** What proves a run worked.
5. **Write the README.** In plain words: what this is, how it was made, what
   to adjust, how it refreshes, what can break it.
6. **Prove it.** Run the compiled piece once from a clean environment, with
   only its declared needs, and compare its output with the draft. A piece
   whose clean run fails is not offered.
7. **Show the approval sheet** (§13.1) and wait for yes.

Judgement that cannot be compiled stays judgement, declared as an AI binding
(§11). The agent prefers code: a judgement that can become a rule ("from my
manager, or mentions a deadline") becomes a rule, with the model consulted
only on the cases the rule leaves open.

### 4.2 Changing

A piece is changed in its authoring Agent's own conversation, with the piece
attached (`82-library.md` §6), not in a conversation of its own (doc 64).
"Add answer counts per tag" there produces a candidate version through the
existing Review path (doc 54): new code, new description, and a preview
built from real data. The
current version keeps running until the candidate is promoted. A candidate
may run alongside the current version so the owner compares both outputs
before switching. Rollback is one action.

A candidate that widens the description — a new site, connection, secret,
AI binding, effect, trigger rate or budget — shows the difference in plain
words and needs the owner's yes, whoever proposed it.

### 4.3 Retiring

Retiring stops triggers and services, keeps data and versions, and moves the
piece to the archive. It can be restored. Erasure follows the lifecycle rules
of doc 74.

## 5. The description

`piece.toml` at the piece's root. Additive within a major version: readers
ignore unknown fields; writers preserve them.

```toml
[piece]
name = "Stack Exchange monitor"
level = "app"                       # document | live_document | tile | app | system
summary = "New and unanswered questions in my tags"

[inputs]
tags = { default = ["rust", "tokio"], label = "Tags to watch" }

[jobs.fetch]
run = "jobs/fetch.py"
triggers = ["schedule:0 */6 * * *", "on_open:stale>1h", "manual"]
timeout = "2m"
checks = ["checks/non_empty.py"]

[functions.filter]
run = "functions/filter.py"

[egress]
allow = ["api.stackexchange.com"]
rate = "100/hour"
max_response = "5MB"

[storage]
kv = true
sql = "data.db"
snapshots = { keep = "90d" }

[ai]                                # absent: this piece never calls a model

[views.board]
entry = "views/board.html"
tile = "views/tile.json"

[publish]
events = ["new_question", "spike"]

[freshness]
promise = "6h"
catch_up = "latest_only"            # latest_only | every_missed | none

[budget]
runs_per_day = 10
egress_per_day = "50MB"
storage = "200MB"

[placement]
prefer = "always_on"                # any | always_on | this_device | <host name>
```

A piece's folder:

```
<piece>/
  piece.toml
  README.md            plain-language how-to, written at keep time
  jobs/ functions/ services/ views/ checks/
  env/                 lockfile and pinned runtime image digest
  data/                storage (not exported unless asked)
  provenance.json      originating conversation, turns, and draft executions
```

Levels use subsets. A document has `[piece]`, its output file and its
provenance. A tile has `[piece]`, one recorded call, a view, and a trigger.

## 6. Runtime

### 6.1 Compute

- **Jobs** run on triggers and exit. They scale to zero.
- **Services** stay running, with restart policy and health probes.
- **Functions** run on calls from views or other pieces, under a deadline.

Every execution runs isolated in an environment built from the piece's
lockfile and a pinned runtime image. The first implementation uses
containers on every host that has them, behind a runtime interface that
admits WebAssembly or microVMs later without changing the description
(decision §20.2).

### 6.2 Triggers

| Trigger | Meaning |
|---|---|
| `schedule:<cron>` | time-based; missed slots follow `catch_up` |
| `on_open:stale><d>` | refresh when someone opens the tile and data is older than `d` |
| `event:<subject>` | another piece or a source published an event (§9) |
| `webhook:<route>` | an outside service called the piece's signed route (§7.2) |
| `manual` | someone pressed Refresh, or asked in chat |

Triggers resolve through the one trigger model, `vak_core::triggers::Trigger`
(shipped at data-architecture M4.3, claimed per slot at M4.4), and the kinds
above join its `TriggerKind` (§20.5, settled by plan revision 3, L7; doc 73
§8). A piece's runs are run records, its outbound actions effects, and its
source positions cursors, as M4 shipped them.

### 6.3 Resources and priority

Each piece has CPU, memory, disk, run-count and egress limits. Pieces run in
a lower priority class than conversations: the owner's turn with Vak is never
slowed by a piece. A piece that exceeds a limit is throttled, then paused with
a reason; it never takes resources from the conversation.

## 7. Network

### 7.1 Egress gateway

Piece code has no route to the network except the egress gateway. The
gateway:

- allows only the exact hosts in `[egress] allow` (no wildcards; resolved
  addresses are pinned per request to defeat DNS rebinding; private and
  link-local ranges are refused unless the owner adds a named local host);
- enforces rate, response-size and daily-volume limits;
- **adds credentials** for granted connections and named secrets, so piece
  code calls the API and never holds the token;
- records every request's metadata (time, host, path, method, status, bytes,
  run) — bodies are not stored by default;
- applies redirect rules: a redirect to a host not on the list fails.

Exfiltration through an allowed host that accepts arbitrary data (a paste
site, a generic storage API) is a real risk. The approval sheet labels such
hosts, and an allowlist containing one requires an explicit owner
acknowledgement.

### 7.2 Ingress gateway

Everything inbound arrives through the ingress gateway:

- **Views** are served at `/pieces/<id>/…` on a preview origin of their own
  (doc 66 §3.2), behind the owner's session.
- **Live data** streams to a view from the piece's storage (SSE) when a run
  completes.
- **Functions** are called by the view through the gateway — "refresh now",
  "filter to rust", "mark read". A view never reaches the internet itself; it
  reaches its own piece.
- **Webhooks** are the one public door: per-piece routes with an unguessable
  path, verified signatures where the sender supports them, rate limits, and
  a body-size cap. A webhook only *triggers* a job with the payload as data.

### 7.3 Between pieces

A piece may call another piece's functions or read its published data only
through a declared binding (`[bindings] weather = "pce_…"`), granted by the
owner. The morning briefing binds to the weather, calendar and mail pieces
and composes them. Nothing is reached ambiently.

## 8. Storage

A piece's KV, database, snapshots and time series are doc 73's
**Application** class: owned by the piece, keyed per piece, always backed
up, and erased with its piece or Space. Its declared snapshot retention is
a label on the piece (doc 74 §3.1). Each piece declares what it needs:

- **KV** — state and cursors ("last seen question id").
- **SQL** — one SQLite database per piece.
- **Snapshots** — dated, immutable outputs of successful runs, so "what did
  this show fifteen days ago?" has an answer. Retention is declared.
- **Files** — generated documents and assets, as versioned objects (doc 73).
- **Time series** — optional, for metrics a view charts.

Snapshots and files are indexed into the catalog (doc 73), so the owner and
the agent can search across pieces. Storage counts against the piece's quota
and is included in backups.

## 9. Events

Pieces publish declared events to the bus (`crates/vak-bus`):
`pce_…/new_question`, `mail/important`, `weather/rain_alert`. Home, the
Inbox and other pieces subscribe. An event carries a small payload and a
reference to stored data, never a secret. Events are the way pieces compose
without polling each other, and the way a piece raises attention (§12.3).

## 10. Connections, grants and secrets

- Accounts — mail, calendar, social, a paid API — are connected once at
  platform level (doc 80's account linking generalised). A connection and
  its grants are doc 73's Connection: privileged Desired state, with the
  credentials in the Secret class. Every effect is an effect record with an
  idempotency key and a receipt (doc 73 §8).
- A piece receives a **grant**: a connection plus a scope (`read`, `draft`,
  `send`, `post`, …) plus optional filters (one mailbox label, one calendar).
- Revoking a connection or a grant stops it everywhere immediately; affected
  pieces show **Needs you** with the reason.
- Secrets are referenced by name and injected by the gateway (§7.1) or, where
  a library must hold one, into the run's environment for that run only.
- **Effects** — sending, posting, booking, paying — are functions with an
  effect class. By default each effect asks the owner at the moment it
  happens. An owner may pre-authorise a narrow class ("post my daily summary
  to my own Telegram") as an envelope (doc 47); irreversible or
  money-moving effects always ask.

## 11. The AI binding

A model is a binding like storage:

```toml
[ai]
model = "local-small"     # a route, resolved like any other
budget = "5k tokens/day"
purpose = "rank new mentions by importance"
```

- A piece without `[ai]` never calls a model. Its tile says **"0 tokens per
  run"**.
- Deterministic filtering and deduplication run before any model call; the
  model sees only new, unresolved items.
- Fetched content is data. A model call inside a piece receives external
  content as quoted evidence and may produce only the piece's declared output
  shape; it cannot trigger effects, change the description or reach a tool.
- Spend counts in FinOps under the piece's id and stops at its budget.

## 12. Views, tiles and Home

### 12.1 Home

Home is the front door, and it stops being a blank chat once there is
anything to show:

- **Tiles** for the owner's pieces, arranged by the owner, with a quiet
  default order (needs you, then recently changed, then the rest).
- **Needs you** — the Inbox items that ask for a decision.
- **Briefing** — the morning summary, itself a piece.
- **Ask** — the conversation, one tap away.

This is a board of the owner's own things, empty until they keep something.
It is not the "dashboard of every capability" that doc 61 rules out.

### 12.2 Tile contract

Every tile has: a title; a status (§13.2); "as of" time; a summary in one of
the closed presentation primitives (doc 67); up to three actions; and a cost
line when technical details are on. Opening a tile shows the full view in the
Canvas.

### 12.3 Attention

A piece raises attention only through declared events routed to the Inbox and
the owner's channels (Telegram etc.), deduplicated, with a reason and
actions: *"Stack Exchange monitor: 40 new questions in rust in an hour.
[Open] [Mute for today]"*.

### 12.4 Live documents

A live document's data blocks are drawn from snapshots and stamped with their
time. Refreshing writes a new version of the document; the previous one stays
readable. Exported files (Word, PDF, Excel) carry the "as of" stamp.

## 13. Two depths

### 13.1 The approval sheet

At keep time, and whenever a change widens a piece, the owner sees:

> **Stack Exchange monitor**
> Reads **api.stackexchange.com**. Uses **no AI**. Updates **every 6 hours**,
> and when you open it if older than an hour. Keeps **90 days** of history,
> about **20 MB**. Runs on **your server**. Sends you an alert on a spike.
> **[Keep it]** **[Change something]** **[Not now]**

### 13.2 Status vocabulary

| Plain status | Means |
|---|---|
| Up to date | last run passed its checks inside the freshness promise |
| Updating | a run is in progress |
| Out of date | the promise was missed; the reason is one tap away |
| Needs you | a decision, a revoked grant, a failed repair, a widened change |
| Paused | stopped by the owner or by a limit, with the reason |
| Off | the host it runs on is unreachable |

### 13.3 What each depth sees

| Concern | Everyday | Technical details on |
|---|---|---|
| What it is | name, summary, README | `piece.toml`, code, versions, diffs |
| Is it working | status, "as of" | run history, check output, logs |
| What it reaches | plain sentence | gateway request log, allowlist, grants |
| What it costs | "0 tokens per run" | tokens, egress, CPU, storage per run |
| Fixing | **Fix it** / **Leave it** | the proposed patch, a shell in the environment |
| Where it runs | "on your server" | host, image digest, resources, placement rules |

Command line: `vak pieces list | show | runs | logs | refresh | pause |
resume | rollback | move | export | import | doctor`. The same API backs
Home, the admin console and the CLI.

## 14. Operating the platform

### 14.1 Reconcile

Each piece's description is desired state. One level-triggered reconcile loop
per host brings observed state to it: build missing environments, register
triggers, start services, roll back a version whose checks fail on promotion.
Events make it run sooner and never replace it. A restart, a crash or a host
move converges back without manual steps.

### 14.2 Monitoring

- **Per piece:** check results, freshness against the promise, errors, cost,
  storage, egress.
- **Platform:** scheduler, both gateways, the bus, the runtime, disk, memory,
  queue depth, and each connection (expired token, exhausted quota).

Platform and piece incidents land in the Operations Center (doc 28,
`/ops/center`) as evidence, never as synthetic numbers.

### 14.3 Repair, in three tiers

1. **Mechanical, automatic.** Retry with backoff, restart a service, release
   a stuck lease, rebuild an environment from its lockfile. No code changes.
2. **Agent repair.** The agent reads the logs and failing checks, explains the
   cause in plain words, and proposes a candidate version. The owner approves
   it, or has pre-approved repairs that do not widen the description
   (§20.4).
3. **The owner.** Anything that widens access, or that the agent cannot
   explain.

While broken, the last good data stays visible and clearly dated.

### 14.4 Updates

- **Vak updates.** Pieces keep running. Each piece's runtime image is pinned.
  Before switching, the new Vak runs every piece's checks against the new
  platform; failures keep the old runtime for that piece and raise an item.
- **Dependency security patches.** Vak rebuilds the environment, runs the
  checks, and promotes on pass; otherwise it raises an item.
- **Piece code.** Through candidates (§4.2).

### 14.5 Doctor

`vak doctor` covers the platform and every piece: environments, gateways,
connections, triggers, freshness, quotas, disk. `--repair` applies only the
mechanical fixes of §14.3 tier 1 and re-reports. Everything else becomes a
plain-language Inbox item.

### 14.6 Admin

Quotas per piece and in total; usage and cost; connections and their grants;
retention; backup and restore of piece data; placement; export and import;
the archive. Admin is where a technical owner tunes; a non-technical owner
never needs to visit it.

## 15. Placement and headless operation

### 15.1 Hosts and placement

A piece runs on one host at a time, chosen by `[placement]` and the host's
capacity. `always_on` pieces belong on a machine that does not sleep. When a
laptop holds such a piece, Vak offers to move it: *"Move this to your server
so it keeps updating when your laptop is closed?"* The owner edits from any
device; the piece runs where it is placed. Moving is: stop, snapshot,
transfer folder and data, reconcile on the new host, verify, switch. A piece
never runs on two hosts at once (a lease with fencing, doc 80's rule; the
epochs are doc 73 §8's).

A laptop that slept reports what it missed — *"missed 15 runs: this computer
was off"* — and catches up per `catch_up`.

### 15.2 Headless on AWS

The current headless EC2 installation is the natural home for always-on
pieces.

- **Ingress.** Home and tiles come through the server's web client behind the
  owner's login (doc 48, doc 78). Webhooks use the signed public routes of
  §7.2 through the server's trusted hostname.
- **Egress, two layers.** The egress gateway enforces each piece's policy;
  AWS security groups or a NAT with a firewall form the outer wall.
- **Secrets.** The encrypted credential store today; a KMS- or Secrets
  Manager-backed credential backend is a natural addition there.
- **Isolation.** Containers on Linux from the start; microVM isolation on
  instance types that support it.
- **Capacity.** Admin shows headroom ("pieces use 40% of memory") and
  recommends a size from doc 79's sizes.
- **Attention without a tray.** Items go to the web Inbox and the owner's
  channels.
- **Disaster recovery.** Piece data and snapshots back up to object storage
  (S3). A replacement instance restores the data and redeploys every piece
  from its description, then verifies each with its checks. This is the
  return on Principle 3.
- **Multi-host.** Laptop and server form one personal system (doc 56); the
  owner sees one Home whatever host serves a tile.

## 16. Systems

At the top of the ladder a piece is a project: a git repository, tests,
several services, perhaps a database. Two shapes:

- **Hosted inside Vak**, using the same runtime, gateways and storage, for
  personal tools.
- **Deployed outside Vak** to real cloud services (functions, databases, a
  website) through infrastructure-as-code the agent writes and the owner
  reviews. Vak keeps the description, runs deploys as jobs with their own
  credentials grant, and monitors the deployed system from Home through
  health checks against it.

Deploying outside is always an owner-approved effect.

## 17. Security

| Threat | Response |
|---|---|
| Malicious or buggy piece code | isolated runtime; no ambient network, files or secrets; resource limits |
| Exfiltration | exact-host egress, labelled data-accepting hosts, volume limits, request log |
| Prompt injection from fetched content | content reaches a model only as evidence through the AI binding, with a fixed output shape and no tools or effects |
| Privilege creep through revisions | any widening of the description needs the owner, whoever proposes it |
| Forged webhooks | unguessable routes, signature verification, rate and size limits; webhooks only trigger |
| Credential leakage | gateway injection; secrets never in the folder, logs, snapshots, events or exports |
| Supply chain | lockfiles and pinned images; patched builds must pass checks before promotion |
| Runaway cost | per-piece budgets for runs, egress, storage and tokens; pause on breach |
| Silent failure | checks, freshness promises, honest statuses, attention events |

Owner approval is required for: keeping a piece; any widening of its
description; moving it to another host the first time; enabling a public
webhook; each effect outside a pre-authorised envelope; deploying outside
Vak.

## 18. Invariants this changes

- **Invariant 13 and 35 (network in restricted modes and previews).** Today
  scratch execution and previews have no outbound network. Pieces add one
  managed path: a kept, approved piece's code may reach exactly its declared
  hosts through the egress gateway. Drafts in scratch keep today's rules;
  only promotion grants the policy. The invariant text would gain a clause
  stating this, and the gateway becomes the enforcement point.
- **Invariant 12 (secrets are not ambient).** Unchanged in spirit; the
  gateway's credential injection is the recipient-scoped injection it
  already requires.
- **Invariant 37 (agent ownership).** Pieces need an owner model (§20.6,
  resolved by `82-library.md`).
- **Invariant 38 (scheduled work is an automation, a `Trigger`).** Satisfied
  provided pieces add trigger kinds rather than a second schedule model.
- A new invariant would state Principles 1, 4 and 7: pieces run without a
  model unless they declare one; no piece reaches the network except through
  its gateway policy; widening a piece is always the owner's decision.

## 19. Fit with existing work

| Doc | Relationship |
|---|---|
| 73 / 74 data architecture | a piece is an Artifact with a recipe; versions, objects, catalog, retention and erasure are theirs; triggers, cursors, effects, fencing, connections and the Application class are doc 73 §3, §5 and §8's. Pieces depend on M4 (triggers, effects), M6 (catalog) and M8 (artifacts) |
| 82 Library | a piece is an artifact that is Live, listed in the Library with everything else; the Library's Keep live is this document's Keep |
| 76 intake | a Source (host-owned connector) and a piece job (owner-approved code) both feed the catalog; a piece may bind to Sources rather than re-fetching |
| 80 mail and calendar | its routines and account linking become pieces and connections on this platform; its operating modes are §6.2's triggers |
| 54 task environments | drafts and candidates use its environments and Review path; keeping is a new promotion target |
| 26 learning | "make another one like this" generalises a piece into a template or skill through its review queue |
| 66 Canvas | full views open there; preview origins serve piece views |
| 61 adaptive experience | Phase 3's Home briefing is built on this Home |
| 28 operations | piece and platform incidents are Operations Center evidence |
| 56 / 79 | placement, multi-host and the headless fleet carry pieces |
| 51 feed system | its pollers become Sources or pieces; it is not kept as a third path |

## 20. Open decisions

1. **Names.** "Piece" for the object, "Keep" and "Pin to Home" for the
   action. "Pin" alone is avoided internally because task, model and route
   pins already use it (invariant 17).
2. **Isolation.** Containers first behind a runtime interface (recommended),
   or WebAssembly first for lighter tiles.
3. **Exposure.** Private to the owner's devices with signed webhooks as the
   only public door (recommended), or optional public views for sharing.
4. **Repair envelope.** Whether agent repairs that do not widen the
   description may auto-promote after passing checks, or always ask
   (recommended: ask, with an opt-in per piece).
5. **Triggers.** Settled by data-architecture plan revision 3 (L7) and
   shipped at M4.3: `Trigger` replaced `TaskDef`, and this document's
   triggers are its kinds. One survives.
6. **Ownership.** Resolved by `82-library.md` §1 decision 4: owned by the
   Space with the authoring Agent recorded (from M3b, which these phases
   follow), so any Agent can refresh or repair.
7. **Sharing.** When pieces are shared with other people, the share model is
   doc 73 §10's grants, and a viewer never inherits the owner's connections.

## 21. Phases

Each phase ships whole: code, tests, docs, screens, and removal of what it
replaces. None starts before data-architecture M6 and M8 have landed.

| Phase | Delivers | Done when |
|---|---|---|
| P1 Foundation | description format, keep flow with clean-run proof, runtime, egress gateway, storage, documents, live documents and tiles on Home | a researched weather tile and a live report refresh on schedule with no model call, survive a restart, and show honest staleness after the host was off |
| P2 Operate | checks, freshness promises, statuses, three repair tiers, updates with check gates and rollback, doctor, admin quotas | a piece whose API changes shape goes Out of date, keeps its last data, and returns to Up to date through an approved agent repair |
| P3 Connect | connections and grants, effects with approval, events and bindings, ingress functions and signed webhooks, micro apps | the Stack Exchange monitor and a mentions watcher run as apps; a briefing piece composes weather, calendar and mail through bindings |
| P4 Place | hosts, placement, moving, headless specifics, object-storage backup and full redeploy | a replacement server restores and redeploys every piece from descriptions and data, and each passes its checks |
| P5 Systems | projects as pieces, external deploys through reviewed infrastructure-as-code, external health on Home | a system deployed outside Vak is updated, monitored and rolled back from Vak |

## 22. Non-goals

- Keeping all scratch work. Drafts stay disposable; only kept work persists.
- A marketplace of public pieces. Sharing follows doc 73's grants.
- A model running continuously. "Live" means code running on triggers.
- A general-purpose cloud for other people's workloads. Pieces serve their
  owner.
