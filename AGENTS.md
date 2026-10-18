# vak — Agent Engineering Contract

Rules for every AI agent (and human) working in this repository.

## Identity

The public product name is **Vakyartha** (https://vakyartha.com), approved on
2026-09-25. The `vak` command, crates, identifiers and data paths keep their
internal names. Brand assets and platform exceptions are in `docs/brand/README.md`.

vak is a Rust general-purpose agent harness. Thesis: strong safety, transparent
operation, extensible integrations, and a simple core.
When a feature request conflicts with simplicity, resolve it as an extension,
not core.

The workspace `Cargo.toml` version is the only authoritative version stamp —
`scripts/check-version.sh` verifies every copy of it, and this document
deliberately does not carry a second one to drift. The supported baseline is
`8.0.0`, the data baseline (invariant 29); nothing below it is read, repaired,
or migrated. Two
earlier version lines are retired: the original `0.1.0`–`0.11.51`, and the
short `0.2.0`–`0.2.4` line created by the reset in `fc9c78f`. `1.0.0` sorts
above both, so version ordering is meaningful again. CHANGELOG.md is the
release record; this section does not restate it.

### Reading the design docs

Start every session at `docs/README.md`: it says which one place answers
each kind of question. `docs/design/` is the architecture record, and
`docs/design/INDEX.md` lists every design doc with its status word
(`proposal`, `in progress`, `implemented`, `reference`, `superseded`,
`historical`), generated from each doc's `Status:` line, which is the truth
for that doc. Read a doc's Status before assuming it describes shipped
behaviour. A few numbers are used twice (`30-output-engineering` and
`30-render-architecture`, `51-feed-system` and `51-retired-tools`, the two
`62-universal-delegation-*` files); cite the full filename, never the number
alone.

Core layers, one document each: 01 LLM · 02 sessions · 03 agent loop ·
04 tools · 05 config · 07 prompt · 08 permissions · 09 extensibility ·
10 flows · 11 planner · 12 evals · 13 server · 14 checkpoints · 68 context
(17 is superseded by it). 00-roadmap.md holds the phase history.

Read before changing behaviour in these areas:

- **Failure handling** — `15-reliability.md` (the failure-handling matrix).
- **Security** — `24-agent-security.md` (threat model and priority order). All
  security work follows it.
- **Release** — `32-release-engineering.md`. Never move a version backwards.
- **Always-on platform** — `22-gateway.md` (gateway/approvals), `23-memory.md`
  (recall), `25-docker-sandbox.md` and `54-task-environments-and-promotion.md`
  (execution backends, candidate promotion), `26-learning.md` (learning loop),
  `28-operations.md` with `docs/hosting.md` (durable services),
  `31-network-resilience.md` (the four-plane network contract and the sync
  plane), `53-distributed-bus.md` (`crates/vak-bus`).
- **Surfaces** — `33-admin-console.md` (admin), `48-web-client.md` (browser and
  the `[server]` exposure rules), `78-headless-identity.md` (owner passkeys,
  recovery, browser sessions), `55-rich-terminal-surface.md` (`vak term`),
  `34-channel-onboarding.md` (channels, multi-bot identity),
  `38-voice-personality.md` and `49-live-voice.md` (voice),
  `29-personal-os.md` (the personal-use surface).
- **Output and presentation** — `30-output-engineering.md` (the delivery
  contract), `30-render-architecture.md` (cross-surface rendering),
  `57-adaptive-presentation-runtime.md` (packs),
  `67-presentation-renderer-guide.md` (start here when adding a renderer).
- **Decision and capability layers** — `86-model-plans-runtime-checks.md`
  (the model's plan and the runtime's receipt check; read first),
  `87-composed-answers.md` (answer blocks), `88-control-plane.md` (decision
  points, step contracts, the correction ladder; `vak-control`,
  `vak-judge`), `41-capability-registry.md`, `42-managed-work-contracts.md`,
  `45-prompt-layers.md`, `47-commitment-kernel.md` (the intent kernel),
  `50-call-and-evidence-contract.md`, `52-outcome-directed-runtime.md`,
  `43-governed-self-evolution.md`, `44-shared-config.md`,
  `39-plugin-ecosystem.md`, `40-harness-engineering.md`.
- **Tool arguments and structured output** — `89-constrained-tool-arguments.md`
  with `docs/audits/cards-in-answers-2026-10-11.md`; see "Tool arguments are
  written, then read" below.
- **Files across turns** — `90-file-state.md`. Decide whether a version is
  behind the workspace only from `file_state::behind`, and give every new
  version the parent it was really made from, never the newest head by
  default.
- **Data** — `73-data-architecture-and-lifecycle.md` (the model) and
  `74-lifecycle-and-data-administration.md` (lifecycles, erasure, screens);
  see "Data architecture" and "The data rules" below.
- **Intake** — `76-intake-and-knowledge.md` (sources, connectors parsed in
  the worker, detection that labels and never drops).
- **Library** — `82-library.md` (artifacts, versions, sharing).
- **Office documents and PDF** — `72-openxml-documents.md` and
  `77-pdf-documents.md` (the file-in, cite, redline, review, file-out loop).
- **Mail and calendar** — `80-mail-and-calendar.md`, with the provider
  matrix and stages in `docs/plans/mail-calendar-implementation-plan.md`.
- **Workers** — `84-worker-questions-and-control.md` (questions, the
  `workers` tool, pause and stop now); the direction for workers that
  outlive a turn is `100-workers.md` (proposal, deferred).
- **Turn graph** — `85-turn-graph.md` (turn ids, turn and recall links).
- **Experience** — `64-agent-owned-platform.md` (the product model; see
  below), `61-adaptive-assistant-experience.md`,
  `65-universal-adaptive-platform.md`, `66-immersive-artifact-canvas.md`,
  `70-calm-agent-experience-implementation.md` (the four-screen visual
  reference), `75-visual-refresh.md` (see "Visual refresh" below),
  `58-admin-ia-and-ui-goal.md`, `59-reference-ui-acceptance.md`.
- **Install and onboarding** — `46-stabilization-install-and-onboarding.md`,
  `36-first-run-onboarding.md`, `37-distribution.md`, `35-tavily.md`.
- **Proposals** — `100-workers.md` (work handed to a worker and kept
  running, deferred on 2026-10-18; its stages and gates are `docs/plans/workers-plan.md`, and it
  supersedes `81-pieces-and-home.md`), `79-private-headless-fleet.md`:
  designs not built; they use the data architecture's shared primitives (triggers,
  cursors, effects, fencing, connections, principals) rather than their own.

### Keeping the record current

Every change keeps the record true in the same commit, for people and
agents alike; `docs/README.md` ("Keeping it current") says what to update
for each kind of change. In short: a design doc's `Status:` line when its
built state changes (then `python3 scripts/check_records.py --write`), the
CHANGELOG's `## Unreleased` for anything a person can notice,
`docs/plans/open-items.md` for work left undone or closed,
`docs/decisions.md` for a maintainer's decision, and the one handover at the
end of a session. `scripts/check_records.py`, run by `scripts/gate.sh` and
`scripts/release.sh`, fails when a status word is missing, the index is
stale, there is not exactly one handover, the tracker or decision log is
missing, or code changed since the last release with no CHANGELOG entry.
Docs are kept, never deleted: a finished or replaced doc changes its status
word, and a past section is marked `> **Historical record**`.

### The blueprint: what the system is, what is happening, what needs attention

`blueprint/` is the one place to see the whole system:
[blueprint/README.md](blueprint/README.md) (renders on GitHub),
`blueprint.html` (diagrams) and `blueprint.json` (for agents), generated
from one model by `python3 blueprint/harness/run.py` on every build. It
measures the code and its history, reads the record where it is written
(decisions, open work, specs, unreleased changes) and lists only the
findings that cost something today. It maps the system twice: crates on
layers, and subsystems (what each part is for, across crates), where every
source file, script, package, route and `vak` subcommand belongs to exactly
one subsystem (`blueprint/harness/subsystems.py`). Its Plant tab draws the
subsystems, the connections measured between them and the flows through
them as an animated plant diagram, with a full-screen operations view. Its
rules are
[blueprint/AGENTS.md](blueprint/AGENTS.md) and its methods are in
`blueprint/methods/`: reviewing the architecture with evidence
(`architecture-review.md`), keeping the record true (`record-drift.md`),
and adding a check (`adding-a-check.md`).

Keep it current: commit the regenerated views in a commit of their own
right after a change that moves them (`scripts/release.sh` refuses stale
views), resolve every `needs review` line in the same change (a new
module, route or subcommand gets its subsystem then), and when a
change fixes, invalidates or adds a finding, edit `blueprint/content.toml`.
Start a session by reading its "Needs attention" section.

### What is authoritative

**`64-agent-owned-platform.md` is the authoritative product model.** Agents own
conversations, private state under `<data home>/agents/<agent_id>/`
(`vak_config::paths::agent_home`: session ledgers, memory), their own
workspace (`vak_config::paths::agent_workspace`: the base workspace for the
built-in `vak`, `tenants/<tenant>/workspaces/<space>/<agent_id>/` in the
data home for any other), lifecycle, channel targets, scheduled work, request admission, and
delivery provenance. Bots are transport identities and channels are
endpoints; internal tasks are implementation details behind the Agent
conversation. Global infrastructure (gateway, operations, FinOps, tasks, the
archive and the trash) stays shared at the top of the data home via
`Core::shared_scope()`; search and lineage are the data catalog in the
data home (`tenants/<tenant>/catalog/`). Doc 64's
topology section draws the tree. Invariant 37 states the enforceable half of
this.

Parked, and not to be assumed shipped: release supply-chain hardening
(SBOM/signing) and the capacity-exhaustion Ask type. The personal-use
completion pass is shipped per `29-personal-os.md`, with enterprise deferred.

### Data architecture (done: M0 to M9, 2026-09-25 to 2026-10-09)

The data-architecture plan is complete as cut for one owner; 7.0.0 was its
data baseline, and 8.0.0 (the card forms) is the current one (invariant 29).
What it built, for everything Vak writes:
- typed ids, principals and a trace key with its actor on every record;
- a content-addressed, encrypted storage substrate with fencing epochs and
  a pluggable key authority;
- a tenant/space layout, the 7.0 data baseline;
- one trigger model, a run record for every unit of work, and effect
  records for every action outside Vak;
- structured telemetry that carries no content;
- one data catalog for search and lineage;
- a lifecycle reconciler with retention labels, holds and crypto-shred
  erasure, committing by default;
- versioned artifacts with inherited sharing (the Library);
- admin and user screens for all of it;
- a folder remote that two machines take turns writing (sync).

The record of how it was built is `docs/plans/data-architecture-plan.md`
(decisions L1 to L12, each milestone's design, steps and exit tests) with
its acceptance runs in `docs/audits/acceptance-m*`; the model is
`docs/design/73-data-architecture-and-lifecycle.md` and
`docs/design/74-lifecycle-and-data-administration.md`. What it leaves open
is in `docs/plans/open-items.md`. No later step starts unasked.

**The data rules. Every change follows them:**
- Build no second schedule or trigger model: scheduled work is a
  `vak_core::triggers::Trigger` (plan M4.3), a Document; docs 76, 80 and 81
  use that model. Start a trigger's work only through
  `triggers::claim_due` (plan M4.4), never with a lock, lease or in-memory
  set of your own, and remove one only through `triggers::delete`, which
  takes its claim with it. Nothing about a run is written back onto a
  trigger: its last run, delivery and check time are queries over its run
  records. Remove a ref (`Store::remove_ref`) only when its name is never
  reused, because a name made again starts over at generation 1.
- Build no second identity for "the root of this work": durable accounting
  keys by M1's `RunId` (the reliable-work plan's E1 waits for it).
- Resolve every new path through `vak_config::paths` and `vak_config::scope`,
  and reach a `Core` home through `Core::scope()` or `Core::shared_scope()`
  and their accessors; `Core` has no raw home getter. A project-layer path is
  a `WorkspaceScope` accessor or a `vak_config::scope` constant, never a
  `.vak` literal.
- Declare every new durable file in `vak_core::state::REGISTRY`, including
  each new subpath of an Agent home as an `agents/{agent}/…` entry with its
  real kind.
- Give new records full UUIDv7 ids, never clock-derived or truncated ones.
- Record an actor where a contribution has one; never infer authorship.
- Keep conversation content out of logs: report through `tracing` with a
  literal message and allowlisted fields (`vak_telemetry::ALLOWED_FIELDS`:
  ids, kinds, sizes, digests, durations, outcomes), and an error as its
  `error_kind`, never its text. Paths, URLs, file names, titles and
  prompts are content. Only the CLI and the apps write to a person's
  stderr. Whoever opens a run does its work in `vak_session::runs::span`;
  a new spawned task carries its span (`tracing::Instrument`), and a
  process with no log of its own returns its captured lines for its
  caller to `vak_telemetry::forward`.
- Read a session for a person or the model through a path that honours the
  trash (`open_historical_session`, `Core::open_session`, or
  `vak_core::trash`), never by opening its ledger directly. A ledger is a
  directory of record segments; read its bytes only through `SessionLog`
  (`read_header`, `scan`, `text`), never as a JSONL file. Every frame is
  sealed under the conversation's scope key (`vak_session::keys`): never
  write a ledger frame without it, add no plaintext path, and reach a
  scope key only through `TenantObjects::create_scope_key`, `scope_key`
  and `destroy_scope_key`.
- Put a large payload in a ledger as a tenant object (`SessionLog::put_object`,
  `Core::objects`), never inline; a ledger Core runs a turn on has its
  store attached.
- Write a side ledger only through `vak_session::chain::RecordChain`;
  never open a JSONL file to append to, and never compact one. A row
  leaves a chain only with its whole sealed segment, dropped by the
  lifecycle reconciler (`RecordChain::drop_segment`); nothing rewrites or
  removes a single row. A row in
  a chain that many conversations share holds what a person or a model
  wrote as an object of the scope that owns it
  (`vak_session::content::seal_fields`, `seal_fields_under` or
  `seal_except`, read back with `restore`), never inline; its reader
  handles `Restored::Erased`. A chain that belongs to one conversation is
  written with `RecordChain::of_conversation`. Decode a stored ledger
  entry only with `SessionLog::decode`, and keep what a person other
  than the owner wrote under their `contributor_scope`. A tool whose
  result comes from a connected provider account says so with
  `ToolOutput::from_account`; never return such data without it.
- Find a session, a run or anything else by id through the data catalog
  (`Core::catalog`, `open_node`, `session_dir`), never by walking
  directories, and search only through `Catalog::search` with the
  caller's `Audience` (invariant 37). A new kind of record that a person
  may search for or trace is a catalog source in the same change.
- Take what an outside source returns only through `vak_core::intake`:
  a closed `vak_intake::Connector`, fetched with webfetch's guard and
  parsed in the worker. Never run a feed script, and never drop an item
  detection holds; label it. Only a person's own view sets
  `vak_catalog::Audience::held`; an Agent read never does.
- Record a deliverable only through `vak_core::artifacts` (a tool's
  `Tool::artifact` claim with `declared`, or a person's save), never a
  list of files of your own; a new version names its parent, and nothing
  overwrites a version.
  Decide whether bytes are already a version and record one only inside
  one `Artifacts::transaction` (`file_state::observe` and the call sink
  do): its rows are appended only if the chain did not move since it
  read (`RecordChain::append_all_after`, under the chain's writer lock),
  else it decides again, so no writer in any process on the data home
  records the same bytes twice. Never take a lock of your own for it.
- Address reviewed work by artifact id and version
  (`/library/{id}/versions/{version}/…`, `Version::proposed`); add no new
  route or client call keyed by a session's candidate id, and note every
  candidate, promotion and undo record with `library::note_review`. A
  comment on reviewed work is a `Commented` row in the version's thread,
  never a ledger activity or a list of its own. A route a guest may
  reach is checked through the real layers in its test
  (`require_bearer` and `enforce_participant_audience`), never by
  calling the handler alone.
- Say what a file or a version is now only from
  `file_state::version_states` and `file_state::needs` (served as `now`
  and `needs` by the Library routes), never from a client's own reading
  of the record.
- Decide what a client shows from where a file is or what a record
  says, never from a directory's name in a path: a test for
  `.vak/scratch` hid Review and Undo once drafts left the project tree.
- Remove, expire or prune nothing on a rule of your own: a retention
  limit is a rule in the install's label (`lifecycle::retention_label()`,
  never `Label::default_tenant()` where a pass or a screen decides), what it applies to is an
  observer in `vak_core::lifecycle`, and a class with no observer is
  reported unobserved, never as nothing due. Observing writes nothing.
  Only `Core::lifecycle_tick` removes on a rule, only for a class in
  `lifecycle::COMMITTED`, never when `[lifecycle] mode` is `observe`, and
  it records the transition before it touches the item.
- Erase only through `Core::erase_conversation`, `Core::erase_draft`,
  `Core::erase_guest`, `Core::erase_account`, `Core::erase_agent`,
  `Core::erase_project`, `Core::erase_person` and `Core::erase_install`
  (and, later, their siblings for other scopes): never destroy a key, delete a ledger or
  remove catalog rows on your own. It acts on what a preview showed,
  yields to a hold, and always writes a receipt that names what it did
  not reach. A ledger's bytes are never the thing removed. A server route that erases ends by dropping the live handle of every conversation it touched, because an open conversation is readable in memory; its test reads the conversation back through the server with the handle attached.
- Move data between machines only through `vak_core::sync` (push, pull,
  hand-over, take-over): never copy a data home, a refs database or a
  key by hand. A push adds blobs and replaces the index last; a pull
  verifies before it writes and ends with `Core::settle_imported`; only
  the machine the remote's lease names writes there; and no secret, and
  no key file, is ever written to the remote. A test of two machines
  drives a second data home with the `vak` binary, because a pull fences
  the process that made it.
- Restore a backup only through `Core::backup_restore`, never by
  copying files into a data home: it is what applies every recorded
  erasure again and moves the writer epoch. A test that restores has a
  test binary of its own, because a restore fences the process.
- Decide who may reach an artifact or a conversation only through
  `vak_core::grants` (and, for search, `Audience::principal`); never keep
  a list of who may see something of your own.
- A decision on the turn path reads a ref or a rollup
  (`vak_session::rollup`), never a replay of a chain or a ledger: a new
  projection a turn needs is a rollup Document over its chain.
- Put a record about a session in that session's Agent home
  (`session_agent_scope` in vak-server), never the serving Core's.
- Keep Agent-authored state as Documents (`vak_session::documents`), never
  as a file written beside the data. The object scopes `tenant` and
  `agent:<id>` are Documents' alone: put nothing else under them, because
  collection releases whatever no live Document names there; a test that touches one isolates its
  home first.
- Key anything about a workspace by its space id (`vak_config::spaces`),
  never by its path. Only `spaces::bind` writes the registry: a writer of
  space-keyed state binds its folder first (`SessionPath::new_session_file`
  and the memory and proposal writers do), a reader never binds, and
  a ledger or Document under an `unbound-` key is refused
  (`spaces::require_bound`). An Agent workspace is its space's; a run
  environment is `env-<run>`, never a space.
- A reader that decides something from a record chain (a status, whether a
  grant is revoked) fails on a row it cannot decode; it never skips one.
- Open a run before any work it causes: mint the key, then
  `Core::runs().open` or `begin` (an `OpenRun` ends the run on every path
  out), and settle it with how the work ended. Key anything a run
  produces (a checkpoint, a result) by its run id; never mint a `RunId`
  that has no record. A session header names a
  run only when one was admitted; opening a session is not a run.
- Take every action outside Vak as an effect (`Core::effects()`):
  prepare it before anything is sent, send only what `begin_dispatch`
  hands this process, and record the provider's answer. Never build a
  second queue, retry loop or single-use claim for an outbound action.
- Keep a position in an external stream as a cursor
  (`vak_session::cursors`): hold the owner, move the position by CAS,
  and record a resync as a gap. Never keep one in memory, in a file of
  your own, or behind a lock file.
- Write durable state only through the fenced paths (`SessionLog`,
  `RecordChain`, `documents`, tenant objects, refs under
  `TenantObjects::writer_epoch`), or call `vak_session::fence::check()`
  first; a new background loop skips its work while
  `vak_session::fence::is_fenced()`. Never move a ref with the store's
  current epoch: that is the epoch a fenced process must not have.

### Visual refresh (V1 to V3, V4.1, V4.2 and V4.4 done; V4.3 and V4.6 wait on the maintainer, then V4.5)

One visual system for the shared client (`crates/vak-client-ui`), so for
both the desktop and the web app:
- readable type: 16px conversation text and 14px controls at 100%, nothing
  under 12px, with the bundled Inter as the default and the system sans and
  Newsreader as options;
- plain words in place of engineering terms in every everyday string;
- one Show technical details setting, off for new installs, which hides
  detail, never capability or safety state;
- the **Ink and Saffron** brand: indigo actions, the Songbird mark's saffron
  as the one accent for live states, neutral light surfaces, a charcoal dark
  theme, and six theme choices.

The tracker is `docs/plans/visual-refresh-plan.md` (decisions, stages V0 to
V4, each item's "done when", the progress log); the design is
`docs/design/75-visual-refresh.md`; the mockups are
`docs/assets/visual-refresh-2026/visual-refresh.html`. Open: V4.3 (drag,
double-click and full screen in the running desktop app), V4.6 (who draws
the character glyphs), then V4.5 (close out); see `docs/plans/open-items.md`.

A visual item is done when its "done when" holds in the running app in a
browser at 1440 × 900 and 390 × 844, light and dark; a build or typecheck
alone is not done. Each change replaces what it supersedes in the same
commit (invariant 30).

**The visual rules. Every change follows them:**
- `DESIGN.md` describes the target system (V2.1); the stylesheet moves onto
  it in V2.2 to V2.4. Build new work to `DESIGN.md`, not to the old values
  still in `styles.css`.
- Name things in user-facing strings by what people recognise (doc 75 §7).
- Add no font size under 12px and no hex literal outside a theme block.
- Edit a selector's existing rule instead of adding an override later in
  `styles.css`.
- Keep IDs, paths, byte counts and hashes out of everyday screens.
- Follow the generated platform exports in `docs/brand/README.md`. Preserve
  the Songbird silhouette and complete app tile across themes; light, dark and
  one-ink lockups come from the SVG master. Only the macOS menu-bar template
  uses the approved monochrome, tile-free geometry.

## Non-negotiable invariants

1. **Model-visible means logged.** Anything that reaches a model request must
   be reconstructable from the session ledger via `derive_messages()`. New
   model-visible input ⇒ new session entry type. Tests enforce this.
2. **Append-only sessions.** Never rewrite or delete session entries.
   Branching = new entry with `parent_id`. Compaction = an entry, never deletion.
3. **Errors are values.** Tools return `is_error` outputs; providers push typed
   errors into streams; the loop returns `TurnOutcome`. Library code never
   panics on bad input; `unwrap`/`expect`/`panic!` are forbidden outside tests.
4. **Every in-process streaming event carries delta AND snapshot.** Consumers
   inside the runtime choose their abstraction level; never force
   re-derivation. The client wire is a projection: `client_events::project`
   (vak-server) sends deltas only and only events a person is meant to see,
   and presentation frames carry a snapshot only when a stream opens, a run
   settles, or a consumer resyncs; the snapshot endpoints are authoritative.
   Under backpressure events coalesce (`StreamEvent::try_merge`), never drop.
5. **Abort preserves partial output.** Cancellation tokens thread through every
   async call; partial results survive.
6. **Unsafe is denied** workspace-wide except process-group kill in
   `vak-tools/src/bash.rs` (annotated).
7. **Transient provider failures retry within the turn's route ladder,
   which is planned fresh on every turn.** The turn ladder is assembled
   at dispatch time from `Core::plan_route_ladder()` using the live
   evidence ledger, session belief state, warm discovery cache, and the
   operator's current `effective_route()` — never from a session-frozen
   snapshot. Walking the turn ladder never changes the effective route.
   Retries honor `Retry-After` under a per-step watchdog deadline;
   run-level endurance re-attempts the same turn after cancel-aware
   backoff when nothing was committed. Informed transience (429 with
   Retry-After, explicit overload) feeds endurance but does not trip
   the shared circuit breaker; blind failures (network loss, deadlines,
   truncated/malformed streams) do. An open breaker fails fast;
   endurance paces its waits to the remaining cooldown so the half-close
   probe gets through. Never retry user aborts. The session header's
   `FrozenContract.route_ladder` / `.provider` / `.model` fields are the
   **initial admission snapshot** for audit only; `WorkReceipt` is the
   authoritative per-turn dispatch record.
8. **Secrets never enter git.** API keys live in the project secret scope or
   the canonical shared secret scope named by `vak_config::user_env_path`
   — resolved through `vak_config::credentials` to an OS-native secret
   service or an encrypted-file fallback, never a plaintext file, so there
   is nothing here for git to accidentally pick up. Point lookups go
   through `vak_config::read_env_file_var/get_var`. Real environment
   variables take precedence over a stored secret. Never hardcode, echo,
   or commit keys. No API writes a secret to a file: a secret an endpoint
   accepts (a provider key, a bot token, bus credentials) goes to the
   credential store (`bus_credentials_never_written_to_a_file`). Keys are
   user-supplied and user-revocable: `Core::set_provider_key` /
   `remove_provider_key` own the whole lifecycle, and both invalidate the
   cached provider client and the discovered-model cache.
9. **Model catalogues are discovered, never hardcoded.** The set of models a
   provider offers is a property of the user's key, not of our source tree —
   a baked-in list hides models shipped yesterday and offers ones the key
   cannot reach. `vak_llm::models::list_models` asks the provider
   (`GET /models`, paginated where the provider pages) and
   `Core::discover_models` memoises it for 5 minutes. When discovery fails,
   surface the reason; never substitute a static list. Endpoint *hosts* are
   configuration and may have defaults; model *ids* may not.
   All provider catalogues use the provider's published HTTP API through
   VAK-owned adapters. No provider SDK or provider-specific control-plane
   client is part of the runtime. A discovered model is a selectable route;
   invocation errors are reported by that provider through the shared error
   contract. Never add a vendor-only preflight gate or SDK credential chain.
10. **Restricted filesystem access is workspace-rooted.** In read-only and
    workspace-write modes, automatic read/glob/grep access must resolve inside
    the canonical session workspace; traversal and symlink escapes fail
    closed. Any exception requires an explicit scoped rule or FullAccess.
    The one built-in exception: a call may reach its own Agent's drafts,
    named `.vak/scratch/<agent>/<execution>/…` and stored in the space's
    execution root in the runtime directory (`vak_tools::drafts`),
    canonicalised; another Agent's drafts and any symlink or `..` escape
    stay refused. Never weaken this with string-prefix checks or check only
    one file tool.
11. **Permission changes revoke old capability.** A runtime mode change must
    cancel in-flight main and side runs and reject pending approvals before the
    new mode is reported. Never let an agent continue with a stale FullAccess
    or sandbox snapshot.
12. **Secrets are not ambient tool state.** Bash and MCP subprocesses receive a
    small operational environment allowlist, not the parent environment.
    Provider, gateway, and connector credentials require explicit,
    recipient-scoped injection; never restore blanket environment passthrough.
13. **FullAccess is an explicit human trust decision.** It is unsandboxed by
    design and must never be selected automatically after a denial, tool
    failure, retry, prompt request, or model recommendation. Restricted-mode
    network denial and approval prompts are independent layers; an approval
    never silently disables the OS sandbox. A restricted mode has two
    network allowances. A dev-server preview the user configured
    (`.vak/launch.toml`): its worker may listen on a port
    (`Sandbox::listening_variant`), and outbound connections stay denied.
    And an MCP server with declared `hosts`: it may connect to its own
    egress proxy's loopback port alone (`Sandbox::proxied_variant`), and
    the proxy tunnels only to those hosts (docs/design/09-extensibility.md
    "Network"); a backend that cannot confine a process to a port refuses
    the server rather than run it unconfined.
    Seatbelt cannot limit listening to loopback, so a preview server that
    binds every interface is reachable from the LAN, as it would be run by
    hand; agent commands never get the allowance. Landlock keeps previews
    closed until the same rule is implemented and verified on Linux.
14. **Model tools cross a broker boundary.** Built-in filesystem and Bash tools
    execute through the versioned `__tool_worker` protocol in a disposable
    process group; MCP servers execute as separately sandboxed workers. Raw
    built-in tool construction is reserved for the worker implementation and
    deterministic unit fixtures. Task orchestration and session search are
    narrow broker-owned capabilities: never give a worker the policy engine,
    approvals, provider credentials, session store, or control-plane handles.
    A missing worker or unavailable restricted sandbox fails closed. A
    command-scoped backend such as Docker must be applied by the broker to the
    validated Bash command; never try to execute a host worker binary inside
    an image that does not contain the pinned worker artifact. A parser of
    untrusted file formats never runs in the server, desktop or gateway
    process: `doc_read` and `office_apply` are worker tools, and every target
    verifier and every Review of an Office file or PDF runs in a worker task
    (`VerifyTargets`, `OfficeReview`) under a read-only, network-denied
    sandbox rooted at the files being read, whatever the session's mode,
    within a deadline; `OfficeNarrow`, which writes a narrower version of a
    draft, may write only that version's fresh staging directory, and
    `OfficeApply` (`vak office apply`) only the directory of the new file it
    writes. A worker that cannot answer fails every planned check;
    a check never passes because verification could not run.
15. **Unattended surfaces fail closed.** The gateway ships disabled, cannot be
    enabled by untrusted project config, and chat-driven turns auto-deny
    escalations unless an explicitly configured approver surface answers a
    forwarded gate inside its window — silence, timeout, or missing delivery
    credential always means no, and a late reply resolves nothing. Never turn
    a denial into permission, forward gates through ambient state, or let
    verdict-shaped chatter from non-approver chats resolve anything. An empty
    `gateway.chat_allowlist` fails closed (rejects every chat) unless the
    operator explicitly sets `chat_allowlist_open = true`; never restore
    "empty means allow all" for compatibility. The live source of truth is
    `<sessions_home>/gateway/allowlist.json` (docs/design/34), seeded once
    from `gateway.chat_allowlist` — an unknown chat lands as a reviewable
    `pending` entry rather than a flat, dead-end rejection; a `denied` entry
    stays sticky and never re-prompts. Inbound channel bridges (Telegram,
    Discord, Slack) build their payload through `InboundChannel` +
    `InboundRequest::new` (`vak-server/src/gateway.rs`), which rejects an
    empty or placeholder `chat`/`sender` — a bridge that reuses one fixed
    identity for every remote user would otherwise defeat the allowlist
    silently. A pooled per-workspace `Core` (multi-tenant CorePool,
    docs/design/34 Phase 2) goes through the exact same trust/permission
    resolution as a local `vak` run in that workspace — pooling never
    grants a channel more access than approval already gated it into.
16. **Every execution path authorizes before dispatch.** Agent turns, task
    children, static flows, dynamic plans, evals, server runs, desktop runs,
    and browser-driven runs must use the same permission decision and
    brokered registry. A direct flow
    node or convenience SDK path may not call an effectful tool before
    evaluating `PermissionEngine` and resolving `Ask` through its approver.
17. **Configuration has one contract across every surface.** Persistent
    workspace preferences changed through authenticated `/config` are written
    atomically to `.vak/config.toml` before the live Core override is
    applied. `/health`, `/config`, `/providers`, admin snapshots, and session
    transcripts must report effective values and their source where relevant.
    CLI flags, task pins, heartbeat pins, and worker pins are scoped
    overrides and must remain visibly non-global. Provider and model resolve
    per turn from the live `effective_route()` (invariant 7); the session
    header's route is the admission snapshot for audit, each turn's dispatch
    is recorded in its `WorkReceipt`, and changing workspace defaults never
    rewrites an existing ledger. Provider and model are one atomic route at
    every read/write boundary. Admission refreshes persisted defaults across
    processes unless an explicit scoped pin is present. Gateway bindings
    expose route provenance and stale reasons; a changed explicit channel
    route override rotates the binding to a new session while preserving the
    old append-only ledger. Persisted max-turns, theme, MCP,
    hooks, and permission mode use the same cross-process refresh; permission
    changes revoke active capabilities before apply — true today for
    `state.sessions` (`apply_permission_mode`, vak-server/src/lib.rs; a
    gateway-registered handle is in that same map, so the *current* turn on
    a channel is reached too) and for the gateway's own default workspace
    (its `CorePool` entry is `state.core` by shared `Arc` identity, not a
    copy). This also holds for other workspaces served by the gateway: warm
    pool entries recheck their persisted security ceiling on cache hits and
    cancel the old permission lease before narrowing.
18. **Durable services use the canonical default workspace.** Generated
    launchd/systemd units for the gateway and channel bridges must execute from
    `vak_config::paths::default_workspace()` (`~/vak-home` for each account),
    never the directory where `self services-sync` happened to run. Per-chat
    workspace overrides remain explicit gateway state. Units must also carry
    the invoking user's non-secret `HOME`; a sanitized manager environment may
    otherwise resolve the workspace as the platform data home and migrate its
    `.vak` directory away. Canonical path resolution must never use the
    current working directory as a missing-home fallback; GUI launches use the
    OS account home and otherwise fail closed to an absolute path.
19. **`doctor` diagnoses; `doctor --repair` only acts on checks with a known
    mechanical fix**, then re-collects and re-prints the report. A check
    with no mechanical fix (provider auth, config warnings) is left for the
    operator — `--repair` must never guess at those or paper over a failure
    it can't actually resolve. `scripts/vak.sh <verb>` is a thin dispatcher
    over `scripts/build.sh` / `scripts/release.sh` / `vak self <verb>` /
    `vak doctor` — it routes, it never reimplements a verb's logic. `vak data verify` is the
    same kind of check for stored data: it reads and reports and changes
    nothing, and `vak data rebuild-catalog` is its one mechanical repair.
20. **Channel capability overlays are restrictive and execution-scoped.** An
    allowlist entry may inherit or narrow built-in tools, MCP server/tool
    patterns, visible skills, and hooks; `None` inherits, `Some([])` blocks
    that category, and deny patterns win. The overlay is persisted with the
    binding, included in the `CorePool` identity, filtered from advertised
    surfaces, and enforced again at MCP call time. Skills are visibility
    controls, not a replacement for permission rules. Secrets remain owned by
    the workspace integration and are never assigned to a channel overlay.
21. **A GET that seeds a same-shape PUT must report only the layer that PUT
    writes, never a merged/effective view.** `GET /config/hooks` and `GET
    /config/mcp` report the *project* `.vak/config.toml` alone, not
    `state.core`'s global+project merge — because their `PUT` always
    replaces the whole project-layer list/map with whatever the matching
    `GET` last returned. Reporting the merged view would silently write an
    inherited global entry into the project file on the very next save. For
    a name-keyed map (MCP servers) that only forks a copy; for a bare `Vec`
    merged by `extend` with no dedup (hooks, before `HookConfig::enabled`
    and this rule existed) it duplicated without bound and re-fired a
    side-effecting command once per copy. Any new config surface with this
    read-then-full-replace shape must follow the same rule. A disabled hook
    is data, not an absence — it stays in the file (`enabled = false`),
    because deleting it on disable is indistinguishable from deleting it on
    purpose, and undoing "disable" must not require retyping the hook.
22. **A workspace/session-scoped mutation and its bulk form must share one
    boundary check.** Data-home-level state (a conversation's lifecycle
    state, `vak_session::conversation_state`, and the gateway allowlist)
    is keyed globally by id, not
    per-workspace, but a session's ledger file — and therefore what a
    *single* mutation on it may reach — lives only under this process's own
    `sessions_dir(sessions_home, cwd)`. A bulk endpoint over the same
    sidecar (e.g. delete-all-archived) must re-derive and apply that same
    per-item existence check, not iterate the sidecar's full keyspace on
    the assumption that every key belongs to this workspace.
23. **A bot is its own identity, not a synonym for its surface.**
    Multi-bot-per-channel (docs/design/34 Phase 5) means `telegram`/
    `discord`/`slack` name a *transport*, not a credential slot — more than
    one `Bot` (`vak-server/src/gateway.rs`) can share a surface, each with
    its own token, policy, permission_mode, and route. Capability/permission
    resolution is a three-tier chain, bot → chat → workspace, composed with
    the same override-or-inherit convention every other tier here already
    uses: `ChannelPolicy::merge` for allow/deny (chat's `_allow` wins when
    set else falls back to the bot's; `_deny` lists concatenate, never
    override, since they only remove access), `PermissionMode::capped_by`
    chained twice (chat capped by bot capped by workspace — never
    escalates), and route falling through chat → bot → legacy binding →
    workspace default. `AllowlistEntry.inherit_bot_policy` is the literal
    "break inheritance" switch: `false` skips the bot tier entirely and the
    chat resolves purely against the workspace, same as before bots
    existed. A `Bot`'s token is never returned by the admin API once set,
    matching the legacy per-surface token's own never-shown-again property.
24. **One physical chat served by several bots gets one identity per bot,
    never one shared identity.** A key is `surface:chat` for a legacy/
    single-bot chat, or `surface:chat:bot_id` once a bridge's inbound
    payload names its own bot id (`gateway_inbound`, `vak-server/src/
    gateway.rs`) — the bot id is a third segment, not folded into `chat`,
    so `legacy_key_for` can strip it back off. Two bots in the same
    physical chat therefore get two independent `AllowlistEntry` rows, two
    independent sessions/bindings, and two independently-resolved policy
    chains (rule 23) — never one shared conversation governed by whichever
    bot's row happens to exist. `allowlist_resolve_inbound` is the one
    place a bot-scoped key is minted: seen for the first time, it inherits
    an already-`Allowed` legacy `surface:chat` row's
    workspace/policy/route (a chat approved before bots were scoped into
    the key, or a `chat_allowlist` config row, must not need a needless
    re-approval the moment a bot id starts arriving) rather than starting
    a fresh pending review — this is a real map mutation, not merely a
    read, so it must persist the allowlist file like any other write.
    Outbound delivery mirrors the same rule: `AdapterRegistry`
    (`vak-server/src/delivery.rs`) resolves a bot-scoped target through a
    `(surface, bot_id)`-keyed adapter map, never the single per-surface
    fallback, so a reply is never sent under a different bot's token than
    the one the chat is actually bound to — a target naming an
    unconfigured bot must fail loudly, not silently fall back to some
    other bot's identity. Any new code that builds or parses a delivery
    target or allowlist key must preserve this three-segment shape rather
    than assuming exactly `surface:address`.
25. **Optional integrations never gate turn admission, and service managers
    own recovery.** A configured MCP server is advertised by name during
    prompt construction but is spawned and discovered only when the lazy
    MCP meta-tool is invoked — never by the capability registry, admission,
    or prompt assembly. The per-`Core` pool (`vak_mcp::McpManager`) reuses a
    connection across sessions, shuts it down after `IDLE_TTL` unused, and
    never respawns without demand; what demand observed (catalog, last
    failure) reaches the registry as declared data. A missing or wedged
    integration must not delay recording the user message or dispatching
    the provider, and abandoned MCP clients must terminate their child
    process. For durable services,
    launchd/systemd PID state is process truth and `KeepAlive` is the sole
    automatic restart owner. Health probes describe readiness; a transient
    timeout may notify but must never authorize a competing restart.
26. **The Operations Center is an evidence projection, never a synthetic
    dashboard.** `/ops/center` derives resource state from live session
    handles, gateway/CorePool snapshots, service-manager probes, durable task,
    effect, security, and health data. Missing or unavailable state remains
    explicit; operational metrics must never be mock, sample, or placeholder
    values. Operations URLs preserve `workspace` and `time` query context, and
    every internal drill-down link carries that context forward. Resource
    detail views stop at the raw session ledger, provider work receipts, effect
    records, incident evidence, or manager state. Incident fingerprints
    reconcile open, resolved, and reopened records in
    the `operations/incidents` record chain; service and delivery actions
    append before/after verification receipts to
    the `operations/actions` chain. Blocking service probes such
    as `reqwest::blocking` must run on a blocking worker, never inside an async
    request handler. Desktop and tray Operations Center links use the same
    bound listener port and bearer/cookie authentication as the admin console.
27. **User configuration is the base layer; narrower scopes store only
    intent.** The Shared layer (`~/vak-home/.vak/config.toml` plus its
    shared secret scope and other capability stores) is inherited by every
    workspace. A project layer
    may inherit, replace a same-named item, add an item, or explicitly disable
    inheritance; it must never receive a copied snapshot of effective user
    values. Session, task, bot, chat, and worker pins resolve after the
    workspace and remain scoped. Admin, Desktop, CLI, and server APIs use the
    same explicit `user`/`project` vocabulary and expose provenance. A GET used
    to seed a write returns that exact layer, never the merged projection.
    Secrets follow the same lookup chain but stay outside TOML: Agent private
    secret scope → project secret scope → Shared platform secret scope, with
    a real environment variable above them all (invariant 8). Secret scopes are never literal `.env` files — they
    resolve through `vak_config::credentials` to an OS-native secret service
    (macOS Keychain / Windows Credential Manager / Linux Secret Service) or,
    when none is reachable (the common headless case), an encrypted-file
    fallback; no plaintext secret file is ever written
    (docs/design/44-shared-config.md, "Secrets Chain"). An Agent or project secret
    must never enter a process-global override map where another pooled
    workspace or agent could observe it. A config write from one process must
    reach an already-running `Core`/UI in another without a restart
    (docs/design/44-shared-config.md, "Liveness"). Curated integrations are
    executable definitions backed by real packages; the product must not
    advertise mock, placeholder, or TODO capabilities.
28. **The prompt is layered; its safety floor only grows.** The shipped
    prompt is a seed, not a constant (docs/design/45). `identity`,
    `operating_rules`, `guardrails`, and `surface_note` are user-editable and
    resolve through the same seed → Shared → project → surface → bot → chat →
    agent-role chain everything else uses. The capability contract, the
    `Surface:` line, and the skill/MCP inventories are **code-owned**:
    `PromptBlock` cannot name them, so no API can accept an edit to one — they
    describe the callable interface as it actually is, and a user who could
    edit them could only make the model wrong about its own tools.
    `identity`/`operating_rules` fall through narrowest-wins like `route`;
    `guardrails`/`surface_note` concatenate and de-duplicate like
    `ChannelPolicy` deny lists, so nothing narrower can remove what a wider
    layer said, and a surface note appends to the generated line rather than
    replacing it. File presence is the inheritance switch: there is
    deliberately no way to spell `guardrails.inherit = false`. A project layer
    is untrusted config until the workspace is trusted — its identity, rules,
    surface notes **and guardrails** are demoted exactly like
    `hooks`/`allow`/`mcp.servers`: free text can say anything, so restriction
    that must hold without trust is a structured `deny`/`ask` rule.
    Guardrail text instructs and never enforces; `PermissionEngine`, the
    broker, and the sandbox are the boundary, and no surface may word it
    otherwise. The prompt is resolved per turn, never frozen per session:
    every turn reads the layers afresh, so an edit applies from the next turn
    of every session without a rotation, and the prefix that turn sent is
    recorded in its `TurnCapabilitiesBound`. Every winning contribution at
    admission is recorded in `FrozenContract.prompt_layers` with a digest,
    ordered by layer breadth, never alphabetically; that list is an audit
    snapshot, not authority. An empty list means *unknown baseline*, not
    *everything changed*. An inbound message may never write a prompt layer.
29. **8.0.0 is the supported baseline.** No code may accept, migrate, or
    special-case state written by an earlier version. A data home, install
    manifest, sync remote, backup, gateway store, or config file that
    predates the baseline is refused by the one shared message
    (`vak_core::baseline`; a data home names the version that created it in
    its `home-version` file, and one holding records (a tenant object
    store) without that file is older than any baseline; a sync index and a backup manifest name the
    version that wrote them; `8.0.0-dev` is the baseline line) — which
    names the file, says the
    install predates the baseline, and gives the single command that
    resolves it (`vak self uninstall --purge`, then install and run setup).
    It is never partially read, never repaired in place, and never migrated.
    The update feed offers no version below the baseline, so `self update`
    from an older line reports the baseline rather than resolving. A
    compatibility branch for a pre-baseline shape is a review failure, not a
    kindness: the tolerances this replaces grew back one sympathetic commit
    at a time, and that is exactly what this invariant exists to stop. From
    the baseline forward the contract is additive-only within a major
    version — a field may be added, never removed, renamed, retyped, or
    redefined; readers ignore unknown fields; a writer that parses, mutates,
    and rewrites a file preserves fields it did not understand; a file whose
    schema exceeds the supported version is refused loudly rather than
    half-read; and ledgers stay append-only forever (invariants 1 and 2), so
    new information is a new entry type and never a changed one. Within a
    major version no migration exists because none can be needed. See
    `docs/design/46-stabilization-install-and-onboarding.md` Part VII.
30. **One canonical way per capability.** A second way to set a token,
    address a chat, name a workspace, or write a setting is not a
    convenience — it is two contracts that must agree forever, and the
    record shows they do not. A new mechanism *replaces* the one it
    supersedes in the same change that introduces it; it never joins it.
    "Kept for compatibility" is not a justification a review accepts.
    Removing the older path is part of shipping the newer one, including
    the tests that pinned it and the doc paragraphs that described it.
31. **Capability changes never require a restart or a session rotation**
    (docs/design/41-capability-registry.md). vak is a daemon with sessions
    that live for weeks, so every capability mechanism must be
    *level-triggered*: one idempotent reconcile loop moves observed state
    toward desired state, and events are hints that make it run sooner, never
    the source of truth. A dropped hint costs one tick of latency and never
    costs correctness. Additions take effect at the next turn boundary of
    every live session via a published epoch; revocations take effect
    immediately and fail closed. Two corollaries a review enforces: an
    observed failure is a reason with a retry, never data — it must never
    introduce a callable name and never prevent the next demand's retry — and a
    capability the operator installed must be reachable without editing the
    harness, which means the host defines a vocabulary and capabilities are
    data classified against it, never a table of instance names.
32. **Intent narrows, never widens** (docs/design/47-commitment-kernel.md).
    A resolved engagement may lower a budget, lower a permission ceiling, or
    *raise* an approval floor. It may never grant a tool, touch the route
    ladder, cap the turn budget or delegation, raise a cap, or lower a floor,
    and it asks nothing of a provider: what a step asks for (reasoning,
    sampling) is the step contract's, from its purpose and configuration
    (docs/design/88-control-plane.md §4), never a reading:
    those caps only ever removed capacity from requests the reader got wrong.
    This is a property of the types rather than a rule to remember: `Limits`
    is a meet semilattice whose top element reproduces pre-kernel behaviour,
    `Limits::meet` is the only composition operator offered, `is_at_most`
    states the invariant as a predicate, and there is deliberately no `join`
    to reach for by accident. Intent never gates the permission engine —
    permission is evaluated exactly as before and intent may only add a
    requirement on top, so a resolution bug cannot authorize anything.
    An uncertain reading resolves to the explicit *orienting* engagement
    (the general posture). A reading never removes a
    capability at all: admission is policy only, and which admitted tools
    are *loaded* follows each tool's own declaration, never the reading
    (docs/design/88-control-plane.md §12.1). A request resolves
    to its **strands** (one reading per part, with relations and cross-turn
    lineage); the turn's engagement meets every authority-bearing limit
    across them. The reader is the fixed one alone: no model makes a
    reading. A weak reading keeps the risk its words stated: it is never too
    weak to raise caution. The tier-1 lexicon is pinned to `RESOLVER_VERSION`
    by a digest test, and nothing in `resolve`/`derive` reads a clock. An
    envelope is pre-authorization *within* existing authority, never a grant
    of new authority: it narrows the strands that serve its commitment and
    covers actions one gate at a time, read fresh at each, and irreversible
    work and an operator's ask rule reach a human whatever was delegated.
    Control authority comes from the
    channel, never from text: `ControlSource` is stamped by the transport,
    human free text is always steering, and only an explicit command
    (`/stop`, `/pause`, `/goal replace …`, a bare `stop`) is control. An
    Agent controls only its own workers (the `workers` tool filters by the
    parent session and reports another session's worker as unknown, never
    as forbidden), and a worker's question (`ask_parent`) and any answer or
    message sent to a worker are information, never permission: they change
    what the worker knows, and every gated call still reaches the approver
    and the permission engine. A surface nobody watches ends a question at
    once rather than blocking (docs/design/84-worker-questions-and-control.md).
33. **The runtime evaluates satisfaction; the model never does.** The model
    may propose criteria; it may not mark one passed. A criterion's
    evidentiary strength comes from how it was established — a command the
    runtime ran is `Observed`, an external receipt is `Attested`, the model's
    own judgement is `Asserted` however emphatically phrased. A commitment
    may not close `fulfilled` below the strength its `evidence` axis demands,
    and the ledger refuses the event at append time: a ledger that can record
    a lie is not an audit trail. Failure verdicts are deliberately
    unconstrained, so the record can always tell the truth about work that
    went wrong.
34. **Network exposure is explicit, never inferred**
    (docs/design/48-web-client.md). The server binds loopback and pins the
    `Host` header to loopback names; reaching it by a real hostname requires
    that name in `[server] trusted_hosts`, and a non-loopback `bind` with an
    empty list REFUSES TO START rather than booting and then rejecting every
    request with a 421 nobody can diagnose. `trusted_hosts` takes exact
    names — a wildcard there is a DNS-rebinding hole with extra steps, and
    entries containing one are dropped with a warning. `[server]` is
    privileged in full: an untrusted project cannot choose an interface,
    relax the host check, or open a shell. A cookie alone never authorizes a
    mutation — a state-changing request carrying one, and every WebSocket
    upgrade (a GET with no CORS, which a page on another localhost port
    could otherwise make with the cookie), must also carry an
    `Origin` we recognise, while a request with no `Origin` at all (curl, the
    CLI, a bridge) must carry a real header token. `?token=` is
    loopback-only: it exists for the one client that cannot set a header on
    an in-process connection, and on anything reachable by a hostname it is
    just a credential in an access log. The web terminal is off by default
    and loopback-pinned when on, because every other effect the client can
    reach is permission-gated and a shell is not.
35. **Workspace execution works in the workspace, keeps its runtime state in
    the space's execution root outside the project, and is observable in
    Workbench.** `bash` runs
    in the canonical workspace (or a `cwd` inside it) — the same view
    `read`/`write`/`edit` address, so what one tool writes the next can read.
    A per-execution empty scratch cwd was shipped and removed: a file the
    shell wrote was invisible to `read`, and a small model looped rewriting
    it. Execution processes run with scrubbed environments (`env_clear`),
    passing only minimal operational paths (`PATH`, `HOME`, virtual
    environment paths) and zero parent credentials or model API keys. Runtime
    state never lands in the project tree: temp files (`TMPDIR`) go to
    `<runtime>/executions/<space>/<agent_id>/<execution-id>/tmp`, and tool
    caches and bytecode (`XDG_CACHE_HOME`, `PYTHONPYCACHEPREFIX`, the pip
    and npm caches) to `<runtime>/executions/<space>/<agent_id>/cache`
    (`vak_config::scope::execution_dir`); a write sandbox grants exactly the
    workspace and that space's execution root. Drafts live there too: a
    draft is still named `.vak/scratch/<agent_id>/<execution>/<path>`, and
    `vak_config::scope::draft_location` says where it is. Files a command creates or changes in
    the workspace are the work itself and are reported as Workbench
    artifacts; a candidate is exported only from an execution whose
    drafts are in the execution root. All executions stream live stdout, stderr, package
    detection events, and status directly to the Workbench panel for full
    operator observability. Egress and permissions follow the broker security
    model, failing closed when unapproved; read-only mode still denies every
    write. Frontend client preview frames are sandboxed and sit
    inside error boundaries. Markup from the conversation, and a page the
    server cannot serve from an origin of its own, runs in an opaque origin
    (`sandbox="allow-scripts allow-forms"`, no network). A page with files
    behind it, and a dev server, runs on an origin of its own, on a loopback
    port and framed under the other loopback name than the app's, so it shares
    no origin, cookie or storage with the app; the preview origin answers only
    the files its scope names under an unguessable path prefix and refuses any
    request that did not arrive by a loopback name
    (`crates/vak-server/src/preview.rs`, docs/design/66 §3.2). The client, not
    the data describing a preview, chooses the sandbox.
36. **Context is measured, never assumed, and nothing model-visible is cut
    blind** (docs/design/68-context-engine.md). Every number that shapes a
    request — window, usable instruction horizon, tokens per char, prefill
    rate, cache behaviour — comes from a per-model `CapacityProfile` probed
    at bind time (majority-of-three samples per rung, over filler shaped
    like real turns) and revised from every receipt; feedback never widens a
    horizon. The turn is the unit: a turn is never split, the open turn is
    verbatim, a closed turn projects at `Full` (its real `tool_use`/
    `tool_result` pairs, thinking dropped, results replaced by a
    schema-driven digest and cards by their short ack — never a
    prose-narrated trace, which a small model imitates as prose), `Card`
    (one `TurnCard` line) or `Packet` fidelity as its plan says. The
    `WorkingSetPlanner` carries the compact card of the turn the request
    follows (the last turn of the thread it continues, else the last
    turn), within the measured budget, and nothing older; no reading of
    the request's words decides it, and older turns are reached through
    `recall` (docs/design/85-turn-graph.md). A
    compaction packet is a cache keyed by the turn range it summarises,
    never a boundary: the projection renders it only when the current plan
    asks for exactly that range, so a packet written under a small model
    hides nothing from a larger model bound later and every request is a
    function of (ledger, bound model's profile) alone; the reset-with-
    handoff entry is the one true boundary. Every
    tool result stays reachable through `recall` by evidence id,
    presentation id or turn number. No character-count truncation anywhere.
    The system prefix (identity, contract, card catalogue, tool index) is
    byte-stable across turns; everything per-turn (temporal context, intent,
    stance, work contract, conversation thread, nudges) rides in one tail
    block attached once to the turn's directive message — placed *before*
    the user's own words, and never restating the directive, both found
    live to derail a small model — and stays there byte-identical for every
    later step of the turn: the working-set plan and tools array are each
    resolved once per turn too, re-planned only by an explicit over-length
    rejection or incremental compaction, so a message the turn already sent
    never silently changes shape underneath a replayed thinking block
    (required for provider-specific preserved-thinking checks and for cache hits).
    Only a handoff reset clears the frozen plan. Cache breakpoints/keys are
    rendered per provider. Presentations and TurnCards are hash-linked
    ledger entries, never rebuilt from tool arguments. No check reads a
    request's or an answer's words: the freshness, grounding and
    topic-mismatch gates are removed (docs/design/88-control-plane.md §5),
    and a card is refused only for a fact, such as a source URL nothing in
    the conversation retrieved, a file a preview names that is neither
    in the workspace nor among the Agent's drafts, or a command no call
    ran: a card that shows what a command printed or how it ended (the
    terminal card, the test report; `Tool::output_of_command`) takes it,
    its exit code and its duration
    from the call that ran it, never from words the model wrote, and a
    call refused before anything ran is not one: a tool says a process ran
    (`ToolOutput::ran`), recorded as `CallEffect::Ran`, failed or not. Every correction goes through the one ladder
    (`vak_control::Ladder`: told once with the evidence, then the frame
    changes or the answer stands);
    a thinking-only step is one `[empty-step]` redo unless a card
    already answered; an answer that repeats a prior turn's answer verbatim
    is nudged once and redone, and one that repeats again stands as the
    model's own with a diagnostic. A tool whose
    calls keep failing is shown its schema and then withdrawn for the turn;
    the turn itself goes on (docs/design/15-reliability.md).
    Vendor and topic names (a search provider, a weather API) are never
    behaviour keys; `vak-eval`'s banned-token gate enforces it.
37. **Agent ownership is mandatory for new work and isolates workspaces,
    memory, and execution.** Every newly admitted session, request, scheduled
    run, child/delegated run, and channel delivery has one resolved Agent
    identity plus its ConversationKey, audience, origin, and configuration
    revision. Each top-level agent (the built-in `vak` and user-defined custom
    agents) owns private state under `<data home>/agents/<agent_id>/`
    (`vak_config::paths::agent_home`), encompassing private append-only
    session ledgers under `sessions/<space id>/` (`vak_config::spaces`) and private memory under
    `memory/`, and its own workspace (`vak_config::paths::agent_workspace`),
    with quarantined execution state partitioned under
    `<runtime>/executions/<space>/<agent_id>/`. Cross-agent infrastructure (the
    gateway allowlist, bots, operations incidents, actions receipts, FinOps
    ledger, scheduled tasks, the archive and the trash) remains shared at the
    top of the data home via `Core::shared_scope()`. Agent identity is resolved
    at admission and cannot be supplied by untrusted client text; a saved
    Agent's definition and lifecycle are re-read every turn. Bots
    identify transport credentials; channels identify endpoints; neither is an
    Agent. Paused, archived, or revoked Agents and endpoints fail closed at the
    next turn boundary (a turn already running is never cancelled or edited
    mid-way) and never fall back to Vak. Internal tasks, tools,
    flows, and workers inherit or explicitly freeze Agent ownership and are
    projected back only through authorized Agent conversations
    (docs/design/64-agent-owned-platform.md). On the client presentation
    layer, Workbench execution telemetry and artifacts are partitioned per
    conversation session in `sessionWorkbenchMap`, guaranteeing that
    executions in background agents are never dropped or cross-contaminated
    when switching active chats, with serialized admission gates preventing
    dropped requests.
38. **Universal outcome presentation, polyglot document ingestion, and domain
    specialist delegation are platform-level contracts.** vak is a universal
    assistant operating with domain neutrality across engineering, research,
    writing, operations, and quantitative analysis.
    - **Document ingestion (`doc_read`)** provides structured, token-bounded,
      token-efficient extraction across Markdown, plain text, CSV, TSV, JSON,
      YAML, TOML, INI, ENV, HTML/XML, and the Open XML family (Word, Excel,
      PowerPoint and Visio, with their template and macro-enabled variants,
      subject to invariant 39) and PDF (parsed and written only by
      `crates/vak-pdf`: invisible, white, tiny and off-page text is
      labelled, and JavaScript, actions, attachments and links are flagged,
      never run or followed; a PDF changes only through an `office_apply`
      draft and Review, like an Office file, and every write is a clean
      rewrite confirmed by a re-read; docs/design/77-pdf-documents.md),
      strictly confined to the canonical workspace root (Invariant 10).
    - **Outcome presentation** treats all tables and datasets as living
      interactive surfaces: every markdown table generated in conversation
      provides client-side column sorting, search filtering, and instant CSV
      export (`InteractiveTable`).
    - **Domain archetypes (`researcher`, `writer`, `operator`, `analyst`)**
      define standard behavioral expectations, epistemic stances, and
      specialist prompts without fragmenting core execution. Workers spawned
      via `task` receive specialist archetype instructions and capabilities
      according to assigned roles.
    - **Scheduled work** is an automation (`vak_core::triggers::Trigger`),
      the one schedule model, owned by the Agent named in its `agent`, with
      at most one start per slot or event (its claim). It works in a copy
      of its folder (a git worktree, or a copy environment for any other
      folder, whose changes come back for Review), never in the folder
      itself. One that cannot run says why in the inbox (`RoutineFailed`)
      and in a run record, never silently.
39. **Office documents are hostile, lossless, labelled and self-sufficient**
    (docs/design/72-openxml-documents.md, O1–O10; this invariant states the
    part the tree enforces and grows with each phase). Open XML packages are
    parsed only by `crates/vak-ooxml`, only in the broker worker (invariant
    14), under its own bounds on entries, per-part and total bytes (counted
    as inflated, never as the ZIP directory declares them), compression
    ratio, XML depth and attributes, because the worker may have no OS
    sandbox under FullAccess. A `DOCTYPE`, a traversing, absolute or
    case-insensitively duplicate part name, and a relationship target that
    escapes the package are refused. The format is detected from the main
    part's content type, never from the extension, and a package whose
    content type contradicts its name fails verification. A write copies
    every part it did not edit as its raw compressed bytes. Document content
    is data: hidden, deleted, white, off-slide, notes and comment content is
    labelled for what it is, and external relationships are recorded, never
    followed. No macro, DDE or include field, OLE object, ActiveX control,
    Excel 4.0 macro sheet or external data connection is ever executed,
    activated or refreshed; Vak has no macro runtime and does not build one:
    macros are read, explained and flagged, and an Agent may rebuild what
    one does as a Vak automation under review. No external office application (Microsoft
    Office, LibreOffice, .NET) is ever a runtime dependency; such tools may
    serve only as CI test oracles, and their results are never shown to users
    as checks. A non-text attachment, from a channel or dropped in a client,
    never enters a prompt as bytes: it is saved to `inbox/` in the Agent
    workspace by the one `save_to_inbox` path and named, and the message
    records it as a typed attachment (`MessageMeta::attachments`) so a client
    draws the file, never the note. Every change to
    a document goes through the one `vak_ooxml::edit` engine and its typed
    ops, and `office_apply` is its only tool: an op names an anchor from a
    read of the exact file (`base_digest`), splices only the elements it
    changes, and is confirmed by re-reading the written package, or nothing
    is written. `office_apply` never writes the workspace file: its result is
    a draft in its execution's directory in the execution root, and the one Review
    path (candidate, worker verification, semantic diff, atomic promotion
    with undo) is how a change reaches the workspace. The agent loop refuses
    a shell command that names a draft the turn delivered for review, and no
    command or script is the way to change an Office file; FullAccess can
    still do it, which is what FullAccess means (invariant 13). A Word edit to an existing document
    is a tracked change authored by the runtime's Agent id, never a name the
    model supplies, and marks only the words that change: every run it does
    not split, and everything the reader does not show (footnote marks,
    images, field codes, bookmarks, hidden text, other authors' deletions),
    is kept byte for byte, and a change to a field's result is refused,
    never written; another author's tracked insertion is struck by a
    deletion nested inside it and split around new text, never rewritten.
    A new document (a file not yet in the workspace, made from a template or
    from scratch) is written clean. A file created from scratch starts from
    Vakyartha's own blank for its format (`vak_ooxml::blank`), never from bytes
    a model or a client supplies; `office_apply` takes that start only for a
    path that does not exist and a call with neither `source` nor
    `base_digest`, so a digest naming a missing file is refused as a
    mistake, and Review replays the draft from the same blank. Viewing a
    file never waits on a shared workspace. A changed Excel input or formula sets `fullCalcOnLoad`,
    and cached values read as stale until Excel recalculates. An op never
    adds, enables or strips macros: the output keeps the source's macro
    state. Shared Office workspaces are a built-in client surface over this
    same engine, never a plugin or parser. Each room names an exact session,
    candidate and path; edits are typed ops against an expected branch head,
    applied in the broker worker and frozen as immutable candidate revisions.
    The room snapshot contains metadata and candidate references only. Edit
    grants permit editing and branching, not acceptance; promotion still uses
    the owner-controlled Review path. A stale head is a conflict. Branch
    merges must detect overlapping operations and conservatively refuse
    positional Word merges when inserts/deletes could move anchors. Agent
    candidates enter a room only through verified lineage from the same base.

40. **Every unit of work has a trace key, minted once where it is admitted**
    (docs/design/73-data-architecture-and-lifecycle.md §4, plan M1). A run
    gets a `RunId` and a `TraceKey` carrying tenant, space, Agent, the
    `actor` and, for Agent work, `on_behalf_of`, from `Core::mint_trace`
    (`vak-core/src/admission.rs`), with the `Cause` its surface implies;
    nothing downstream re-derives it from ambient state. The key travels by
    value: `AgentConfig.trace`, a child span per tool call
    (`ToolContext.trace`, `TraceKey::child`, the only span constructor), the
    broker worker, `SandboxEventSink`, the bus envelope, and every durable
    row, span and envelope written under it, which names its actor
    (`Traced`, `every_ledger_row_type_is_traced`). A delegated child is its
    own run caused by the parent's call (`TraceKey::delegate`). The actor is
    the real one: the owner, a channel sender the gateway resolved, the
    Agent, or the system, never text a client supplied; derived writes record
    `derived_from`. A write site with no key in scope leaves the additive
    fields absent rather than inventing one, and never infers an author.
    A turn has one id, minted by `Core::run` and opened with
    `SessionLog::begin_turn`: the directive's entry id, the id its strands
    and threads derive from, and the `turn` of the run's key
    (`TraceKey::in_turn`). Every session entry names its turn in
    `Entry::at_turn`, stamped on append; no reader joins a record to its
    turn by position (docs/design/85-turn-graph.md, G0). The
    data dictionary for these records is `docs/reference/records.md`,
    generated by `records_reference_is_current`; regenerate it with
    `VAK_UPDATE_RECORDS=1` rather than editing it.

41. **Every action outside Vak is an effect, and an effect is sent at most
    once unless the provider drops repeats** (plan M4.5,
    docs/design/73-data-architecture-and-lifecycle.md §8). An outbound
    message, a mail send or a calendar change is
    prepared in the `effects/` chain (`vak_session::effects`) before
    anything is sent, naming its run, kind, target and an idempotency key
    (the hash of run, kind, target, payload digest and ordinal), with its
    payload stored as a tenant object. Only the process that moves the
    ref `eff/<key>` under the writer epoch (`Effects::begin_dispatch`)
    calls the provider for an attempt. An attempt ends accepted or
    confirmed (with the provider's receipt), failed, or unknown; only a
    delivery failure proven not sent (a 4xx, a refused connection, nothing
    taken) is tried again, up to `MAX_ATTEMPTS`. A mail or calendar change
    is one effect per reviewed candidate (`Effects::prepare_once`, keyed by
    the candidate alone), sent once by the request that approved it, and
    never again without a new review. A dispatched effect whose sender
    stopped before the provider answered becomes unknown
    (`Effects::recover`), and an unknown effect is never sent again by
    itself, after a restart, a restore or a handoff alike: a provider that
    drops repeats of the key (Discord's `nonce` with `enforce_nonce`)
    takes it again as the same effect, and anywhere else the owner sends
    it again as a new effect that supersedes it, or says whether it was
    sent (`Effects::reconcile`, recorded with who said so). Readers fail
    on an effect row they cannot decode.

42. **Deletion is recorded, yields to holds, follows lineage, and never
    rewrites a record** (data-architecture plan M7a,
    docs/design/74-lifecycle-and-data-administration.md §4, §5). Only the
    lifecycle reconciler removes on a rule (`Core::lifecycle_tick`, a rule
    in a `vak_lifecycle::Label`, a class in `lifecycle::COMMITTED`), and
    it records each transition before it touches the item; it runs by
    default, and `[lifecycle] mode = "observe"` makes it only show its
    plan. Record data is erased only by destroying its scope key
    (`Core::erase_conversation`, `erase_draft`, `erase_guest`,
    `erase_account`): no ledger or chain byte is rewritten or removed, so
    every chain still verifies (`Core::data_integrity`). An erasure acts
    on what a preview showed, is refused while anything in its reach is
    on hold, removes what was derived in plaintext (search rows, memory
    notes, rollups) by lineage, and always leaves a signed receipt that
    says what it could not reach. The trash hides a conversation from
    every list, search, recall, prompt and export, and ends in erasure
    after its window. A restore applies every recorded erasure again
    before it ends (`Core::backup_restore`). Live verification never
    commits or erases on the operator's real data home.

43. **The owner's data and someone else's words together put every exit
    to a person** (docs/design/08-permissions.md "Provenance"). Every tool
    result is labelled private, untrusted, both or neither from what its
    tool declares (`Tool::provenance`) and where it came from (a connected
    account, a worker, a recalled turn), recorded as
    `CallEffect::Provenance` and kept on the turn's card. A request holds
    what its open turn received and what every earlier turn its plan
    carries held; a delegated worker starts with what its parent held. Once
    a request holds both, a call that can leave the machine
    (`Tool::leaves_machine`; a shell command only where the sandbox leaves
    it the network) is an `AskSource::Provenance` ask that nothing
    pre-authorizes and an unanswered surface refuses, in every mode but
    FullAccess, which is the owner's explicit trust decision (invariant 13)
    and asks only what the owner's own rules ask. Labels only join
    within a turn, and nothing reads a word of the content to set one.

## Code rules

- Edition 2024, stable toolchain. `cargo fmt` + `cargo clippy -D warnings` must pass.
- No comments unless semantics are non-obvious; doc-comment public API items
  whose contracts aren't clear from names (invariants especially).
- No new dependencies without exact versions pinned in the workspace manifest
  and a one-line justification in the PR.
- **Agent UI is Vakyartha-owned.** Do not adopt third-party agent UI SDKs,
  runtimes, or protocol implementations (including A2UI, AG-UI, MCP Apps UI
  bridges, or equivalents). Vakyartha owns its presentation schemas,
  compiler, renderer, interaction state, actions, and event contract.
  External systems are comparison references, not implementation dependencies.
  Existing general-purpose libraries remain subject to the dependency rule
  above.
- Output presentation is schema-v2 and semantic: `vak-delivery` owns the typed
  `OutputTimeline`/`PresentationDocument` contract, exact Markdown fallback,
  capability projection, and deterministic degradation. Desktop/native clients
  render the closed AST registry; raw HTML and untrusted model-authored UI are
  never mounted. Presentation snapshot/SSE endpoints are reconnectable
  projections over the ledger and live events; legacy transcript, `AgentEvent`,
  webhook, and channel text paths remain compatibility surfaces.
  **A card is a form** (docs/design/92-card-forms.md): a shape of
  information, never a topic. The forms are `vak_delivery::FORMS` (summary,
  steps, list, details, table, comparison, stats, chart, callout, sources,
  calendar, map, controls, link, media, file preview, markup preview, diff,
  test report, terminal), and the same list is the card tools, the delivery validators
  (`validate_form`, one text form each in `structured_markdown`), the host's
  form primitives (`vak_presentation::PRIMITIVES`, plus layout) and the
  client's `FORMS` table (`vak-client-ui/src/components/cards/index.tsx`),
  checked equal by `the_card_kinds_are_exactly_the_types_the_client_draws`.
  Every card carries its form as `StructuredOutput.block`, and the client
  draws by block in one shared card shell; a kind only selects a pack on
  the host. A form's own kind is its name; a pack adds kinds to a form
  (doc 91); nothing else does. An answer uses as many cards as it has
  parts, laid out by doc 87; the model alone places a card: `replaces`
  (a presentation id of this answer) puts it in that card's place, and
  `beside_previous` beside the card before it. Every card says whether
  the answer goes on after it (`then`: `more` or `done`); a step of
  accepted cards whose last card said `done` ends the turn when the plan,
  if any, has nothing left, and no further step is asked for
  (docs/design/97-cards-end-the-answer.md). Nothing relies on the model
  choosing to say nothing. Nothing decides a card by words or counts:
  no signals read from text, no title word lists, no row thresholds. The
  contributor guide is `docs/design/67-presentation-renderer-guide.md`. A model shows
  each card by calling the card tool that owns its kind, by its own name
  and precise schema (`vak-core/src/presentation_tools.rs`, one per form;
  a shape
  that could be given one of two fields is two cards, each with that
  field required, so the wire can enforce it); one nested call of every block, tried as the `answer` tool,
  came back as broken JSON on 7 of 16 captured steps against none for
  separate calls (docs/audits/cards-in-answers-2026-10-11.md). A card call
  is the loop's own display channel (`show_card` in `vak-agent`), in every
  permission mode: checked against its schema, the plan, the sources the
  conversation retrieved and its tool's rules, then recorded as a
  `PresentationSource::ToolCall` presentation with the plan deliverable its
  `deliverable` names, and the runtime lays the answer out from those names
  (docs/design/87-composed-answers.md §2). One
  argument name is one shape on every block
  (`one_argument_name_is_one_shape_on_every_block`). A card that names in
  `deliverable` a deliverable the plan does not have is refused with the plan's
  names, and one that names none while planned cards are still unnamed is
  refused with those names; a planned card counts as shown only when a card
  names it. Text written in reply to a plan check that followed an answer
  is addressed to the runtime and is not projected (doc 86 §2.2). A table
  card whose rows are one cell each is refused as a list. `PresentationSource::Answer` is recorded
  history only; nothing writes it. A card that fails is refused with
  what to fix, and goes through the repair stages like any tool. The fence path, an inline
  ` ```vak ` fence, is the fallback when no card tool is admitted, and a
  fence carries the same arguments the block takes. A card block's
  arguments are flat beside `semantic_type`, use only the schema keywords
  every provider carries, put no data in keys, hold no list of records
  inside a record in a list, require `semantic_type` first and ask for no
  value that starts with `|` (a table is its rows, `A | B` strings) and end
  no record in a number (a research source carries its one `finding`) and
  ask for no fact twice (a media card's kind is its `semantic_type`, its
  address its one `url`) (measured, §30.1;
  `every_card_schema_is_flat_and_portable`); each shape's `build` turns them
  into the stored card. **A card tool leaves nothing to guess:** every
  argument, and every field of a record argument, is typed and described,
  one the call must send is never described as optional, the same idea has
  the same name on every block (prose is `text`, a record's name is
  `label`), a builder checks structure and never a topic, and no argument
  shape crashes a builder (`every_card_argument_is_described`,
  `no_argument_shape_crashes_a_card_tool`). Both checks run over whatever
  tools exist, because tools are added and removed at runtime; a tool is
  never made to work by naming it in the prompt. The card travels in the call's *arguments*
  (recorded untruncated in the ledger); the result is only a short ack
  or a repairable validation error naming the block and field, and the card is written
  as a `Presentation` ledger entry when it validates, which `vak-server`'s
  projection reads — never result text, of which an over-long result's
  request carries only a window. A
  worker's cards reach the conversation that delegated to it: the worker
  records them in its own ledger, `task` hands them up beside its text
  (`ToolOutput::delegated`), and the parent loop records each as a
  `PresentationSource::Delegated` presentation of the `task` call and lists
  its id in the result, so the user sees it and the parent can recall it. A
  projected card carries `OutputKind::Card`, never `Information`/`Progress`/
  `Retry`, which chat views fold away (doc 30-output-engineering). When a
  **The card path has two validation gates.** After provider decoding,
  `normalize_response_tool_uses` canonicalizes the provider call: the one argument reader for every tool
  (`vak_tools::read_arguments`, docs/design/04-tools.md "Reading
  arguments"), then the tool's own `Tool::fill_defaults`; this happens before the
  assistant message is ledgered. `authorize` then validates against the
  advertised tool schema, and only an authorized call reaches `execute`, the
  shape builder, and `SkillRegistry::validate`. A repair inside `execute` is
  too late to fix a value rejected by schema admission. Any compatibility
  reading of arguments must therefore run before `authorize`, in the reader
  when the schema alone decides it and in the tool's `fill_defaults` when
  only that tool can read the shape, never in the loop or a builder; its
  test runs the reader, `fill_defaults`, `validate_input` and then the real
  builder path.
  After validation, the call and its result are ledgered and a
  `PresentationRecord` is appended, built from the block the call was made
  to (`presentation_info`; a stray `semantic_type` in a call's arguments
  never sends it to another block, and only a fence resolves its block
  from `semantic_type`); a card the host cannot record is a tool fault,
  never acknowledged as shown; `vak-server` projects that record by
  `tool_use_id` (and an answer's by its block), and the client resolves `semantic_type` through the single
  renderer registry. Diagnose a missing or wrong card at each boundary in
  that order; a successful tool acknowledgement proves ledger admission, not
  UI projection. Keep this path and turn state consistent with
  `docs/design/30-render-architecture.md` and the ledger/context rules in
  `docs/design/68-context-engine.md`.
  **Turn context is planner-selected.** Do not add the full transcript to each
  request or create a second session to test cards. The ledger remains complete;
  `vak-context` chooses whole turns, TurnCards, and recallable evidence for the
  current model and directive. A new direct user request resets turn-local
  card duplicate/retry state, including image-only requests; typed runtime
  controls do not count as new turns. Same-turn distinct cards must retain
  their emit order, while identical retries are suppressed or superseded only
  under the documented repair rules.
  When prompt guidance, context, schema, tool call, ledger, projection and UI
  disagree, record the exact stage and evidence before changing the model
  prompt. These boundaries and the conformance gaps are documented in
  `docs/design/30-render-architecture.md` §30.1 and
  `docs/design/68-context-engine.md` §10.
  The standard request planner carries only the compact card of the turn
  the request follows, whatever its words. A request gets no older turn just
  because topic words match or prompt space remains; the model uses indexed
  `recall` when an older result is needed, even if it is many turns back. Full
  session history remains in the ledger. Inspect the recorded context plan and
  recall calls before attributing a cross-turn card to UI projection. Channel
  narration must inspect cards in the latest turn only; an older card must not
  suppress a current prose answer.
  **Presentation schema access is independent of request vocabulary.** The
  shared tool surface loads every admitted `presents_cards` schema from the
  first call on every provider, except a card that shows only what a call
  produced (`Tool::grounded_in`: a command ran, a file was written, a source
  was retrieved, an ability answered). That one loads once the conversation
  holds such a call, read from the ledger's own records
  (`vak_tools::grounds_held`), and is promoted in place at the step one
  appears, the way a tool called by name is; a card required by the plan
  check is promoted too. Before then it could only be refused, so it waits
  under "More tools". Schemas carry no historical card content and
  do not make ordinary conversation require structured output. Recipes and
  request predictions inform output demand and composition, not access to
  alternative shapes. **The model plans, the runtime checks**
  (docs/design/86-model-plans-runtime-checks.md): the model declares its
  deliverables with the loop-owned `plan` tool (each a name and a form:
  card, text, file, change or check) and names the one a card delivers in
  its `deliverable` field; when the turn ends, `vak_judge::plan` compares the plan
  with receipts (cards recorded, final text, files delivered, commands and
  changes) and a `[plan-check]` control names only what is missing, once,
  without discarding anything already produced; a planned card still
  missing after that is required of one step as a call of the card tool
  owning the kind the plan named for it (`Deliverable::kind`,
  `ChatRequest::must_call`), once, and what is left then stands. No reading of the request
  or the answer sends a turn back: the request-vocabulary presentation check
  and the card-completeness self-audit are removed, and the stop gate holds
  a turn only on observed facts (an unclosed fence, an unreported tool
  failure, code changed after its check). Never add a check that guesses
  from words or a threshold. The registry and live-run coverage checkpoint
  is `docs/audits/presentation-registry-and-live-coverage-2026-10-10.md`.
  Runtime-authored traffic is **typed, never sniffed**: every user-role
  message the agent loop appends (repair nudges, stop guards) is created with
  `MessageRecord::control(kind, body)`, which sets `MessageMeta::control` to a
  `vak_intent::control::ControlKind`, and that tag is the only way any layer
  recognises one. `vak_intent::control` is the single vocabulary (the kinds,
  the inline hints, the derived context-block tags) and the single
  implementation of `clean_scaffolding`, shared by `vak-server`, `vak-delivery`
  and every channel. The tag drives the projection, the transcript API, session
  search, exports, compaction packets, and the per-turn state
  the runtime resets on a user message; none of them count a nudge as a user
  turn. Clients receive only what a person can see (`/transcript` omits nudges,
  context blocks and the frozen contract), each message with its ledger
  `entry_id`, and the chat pairs a turn with the projection by that id, never
  by position. An answer the runtime sent back for a redo is an internal draft
  and is not projected. A channel, webhook, inbox or routine summary gets the
  turn's cards (their deterministic text form) ahead of the narration, via
  `projection::text_with_run_cards`, because a card emitted through a tool is
  not in the model's final text. The two lists the desktop client keeps
  (`INLINE_HINT_MARKERS`, `CONTEXT_BLOCK_TAGS` in `structured.ts`) are checked
  for exact equality with the Rust vocabulary by
  `vak-server/tests/it/control_vocabulary_sync.rs`.
  Identifiers that name a lockable resource are unique by construction, not by
  clock: a child worker's session id carries a process-wide sequence number
  (`task.rs::next_child_session_id`), because ids that were the clock's
  nanoseconds alone collided when tasks launched in the same wave. The chat
  pairs turns through `vak-client-ui/src/turnPairing.ts`. What remains
  judgement (which card the model picks, inline tool-result hints) is
  listed in
  `docs/design/30-output-engineering.md`. A tool declares that it
  presents cards with `Tool::presents_cards`. A tool names the file a
  successful call delivers for review with `Tool::delivered_file`
  (`office_apply`'s draft): the file is a receipt for the plan check,
  an identical call gets the first call's result instead of a second draft,
  and a card whose `artifact_path` previews that file is not shown.
- The shipped prompt seed stays under 1800 tokens and carries its
  `<!-- block: -->` markers; changes require a diff note in
  `docs/design/07-prompt.md`. Layer composition, trust, and the editing
  surfaces are `docs/design/45-prompt-layers.md`.
- A committed or shipped frontend bundle must match its source. `npm run
  build` writes `dist/.src-manifest`; `vak-server`'s and `vak-desktop`'s build
  scripts re-verify it and fail the build naming the stale file. Never
  weaken that check to get a build through — regenerate the bundle.
- Config keys unknown to this version are ignored with a warning, never fatal.
- Telemetry is content-free and never read to decide anything: a log line
  or span is for a person diagnosing, never an input to behaviour.

## Outcome-directed runtime contract

The outcome-directed runtime is the canonical interpretation-to-delivery path
for every surface, not a coding-only feature. A request is recorded as an
`OutcomeSpec` during admission and carried through capability discovery,
permission-bounded execution, child runs, evidence evaluation, and delivery.
The specification is an interpretation and completion contract; it never
grants authority. Permission, sandbox, broker, route, and approval contracts
remain authoritative and may only narrow the admitted outcome.

The durable chain is:

`request → IntentRecord/OutcomeSpec → admitted limits → results/evidence → evaluation → OutputTimeline`

`IntentRecord` is model-visible only through its recorded `model_visible`
projection. Audit-only records never enter `derive_messages()`. Sessions are
append-only: `GoalUpdate` records collaborative changes, `IntentRecord` records
outcome revisions, and `ActivityRecord`/receipts record lifecycle and evidence.
Replaying the ledger must reconstruct the model-visible input exactly.

Each meaningful result is independent. A result may be complete, partial,
failed, cancelled, or unknown; a typed renderer cannot upgrade an unsupported
claim into observed evidence. Evidence is scoped to the result and requirement,
and its strength, freshness, identity, and applicability are evaluated
separately. Missing or stale evidence must remain visible as partial/unknown.
A card shows only what its validated payload holds; otherwise deterministic
Markdown fallback is used. No surface may independently classify a result.

A collaborative message adds to the active goal unless it is an explicit
command: `/goal fix …` corrects (keeping the additions), `/goal replace …`
replaces, `/pause` / `/resume` / `/stop` / `/status` change control state.
There is no text classifier for this — one read "replace the deprecated API
call" as a goal replacement and wiped the additions. The `GoalState`
projection exposes the current objective, additions, superseded revisions,
and control state. Replan/reprioritise/add/remove commands use the
intervention path under the `ControlSource` matrix (humans: queued as a new
outcome revision; agents: only their own children, scope changes require a
human; systems: observe only) and queue work at a safe boundary. Human
approval is required whenever the change would exceed the current authority
or affect an irreversible action.

`OutputTimeline` is the cross-surface contract for web, desktop, and channel
delivery. It carries result outcome metadata, evidence references, diagnostics,
and the projected goal state. Clients render the closed semantic AST or the
exact fallback; they do not mount model-authored HTML or invent status. The
desktop/web goal panel is observational, while the existing run-control bar is
the single action surface for pause/resume/cancel and plan changes.

New capabilities (renderer, skill, evidence adapter, hook, tool, evaluator, or
environment provision) are workspace-scoped proposals until validated and
promoted. Presentation/skill/evidence/evaluator/environment contributions may
be auto-admitted only inside an existing envelope; hooks/tools and anything
networked, secret-bearing, or externally effectful require human review.
The one tool an Agent may admit itself is a skill's program
(docs/design/93-self-evolving-capabilities.md §5), and only for itself
(its own `agents/<id>/skills`): one whose tests the runtime ran and passed,
that declares no host, needs no install and was not written in a turn that
held someone else's words, while the owner leaves `[owner]
abilities_go_live` on (`skill_programs::review_reason`). It runs only
under the sandbox its declaration gives it, and anything else it proposes
waits for the owner; reaching every Agent is always the owner's promotion.
Promotion never silently activates a proposal, and revocation takes effect
immediately without a restart.

The evaluator must measure both usefulness and cost: pass rate, completion
verdict, evidence state, human-review rate, latency, token use, and baseline vs
outcome-directed deltas. Repository suites prove deterministic behavior;
representative live workloads are operational validation and must not be
presented as completed from fixture results alone.

## Layout

Every path in the left column is verified by
`python3 scripts/check_doc_paths.py`, so a crate rename that misses this map
fails CI rather than leaving the contract doc quietly wrong. Keep the column
format: the checker parses it (path, spaces, prose), which is why these are
not backticked.

```
crates/vak-llm       unified provider API (anthropic / openai-responses /
                     openai-completions / google / native ollama), SSE,
                     delta+snapshot events, live model discovery
                     (models.rs, incl. quantisation from Ollama's
                     `/api/show`), work receipts + dispatch ceiling
                     (work.rs), frozen-ladder ordering: demand-scored
                     objectives, belief demotion, cross-model fallbacks
                     (route.rs) -- docs/design/42-managed-work-contracts.md
                     Phases A+B+R. Which ids at different services name
                     one model (model_identity.rs): confirmed
                     `[route] same_model` groups, and suggestions read
                     from id spelling alone, used only once confirmed. `ChatRequest.cache` (session key +
                     per-message breakpoints) render per provider:
                     Anthropic `cache_control` (<=4 breakpoints) with
                     opaque `ContentBlock::Provider` round-tripping for
                     server tools, OpenAI `prompt_cache_key` +
                     `previous_response_id` chaining, OpenRouter
                     `session_id`; `turn.rs::current_turn_boundary` decides
                     what thinking gets replayed. Native Ollama
                     (`ollama.rs`) speaks `/api/chat` directly with
                     `keep_alive`/`num_ctx`, fills prefill/load timing into
                     `Usage`, never replays thinking, and honours
                     reasoning off (`ChatRequest.settings`, asked by the
                     structured side-dispatches: classify, compaction,
                     handoff, plan --
                     measured live, a thinking model spent its whole budget
                     deliberating and returned no JSON; the completion judge
                     keeps the default because thinking made its verdicts
                     parse 6/6 instead of 3/6) (docs/design/
                     68-context-engine.md §8, §10, §11)
                     + Gemini Live voice synthesis
                     (google_live.rs): BidiGenerateContent WebSocket
                     session, wall-clock timeout + input-length cap since
                     no upstream deadline exists otherwise, WAV wrapping,
                     WorkPurpose::VoiceSynthesis receipts (docs/design/
                     38-voice-personality.md)
crates/vak-voice     the voice vocabulary: the closed `VoiceProvider` set and
                     its catalogue, the direction-typed `/voice/session`
                     protocol (a client can never author a transcript),
                     server-side speech evidence measured against each
                     utterance's own quiet floor, WAV framing, and the
                     offline engine executables. The HTTP/socket surface is
                     crates/vak-server/src/voice.rs
                     (docs/design/49-live-voice.md)
crates/vak-session   append-only ledger trees on record segments, each
                     frame sealed under its conversation's key (keys.rs),
                     tenant objects (objects.rs), fencing (fence.rs), run records
                     and trigger claims (runs.rs), effect records and
                     their dispatch refs (effects.rs, invariant 41),
                     stream cursors and their gap rows (cursors.rs),
                     frozen contract, projection,
                     receipt, presentation and turn-card entries (audit-only,
                     projection-neutral), TurnIndex and fidelity projections
                     (docs/design/68-context-engine.md), the tail reader
                     a derived index catches up with (tail.rs), and the
                     text matching in-session recall ranks by (text.rs)
crates/vak-telemetry content-free structured telemetry (plan M5): the
                     process subscriber (`init(<service>)`), JSON lines with
                     allowlisted field names and the run's trace id, span
                     closes with durations, size-rotated
                     `<logs>/vak-<service>.jsonl`, the worker's captured
                     lines forwarded under their caller's span (`forward`),
                     and the source tests that keep `eprintln!` out of
                     library code and messages literal
crates/vak-control   the control plane's decisions (docs/design/
                     88-control-plane.md), with NO vak dependencies: the
                     closed set of decision points, the step contract, call
                     and answer verdicts, corrections and endings, each a
                     typed value. Every decision the loop makes is written
                     as an audit-only `EntryPayload::Decision`, never
                     model-visible; a turn's decisions in order are its
                     explanation
crates/vak-judge     receipts to verdicts (docs/design/88-control-plane.md
                     §7), pure, over vak-control and vak-intent only: the
                     verdict on a proposed call from gathered facts and
                     where a refusal's fault lies (call.rs), a turn's
                     unrecovered failures, the plan check (plan.rs) and the
                     outcome, requirement and completion evaluation
                     (outcome.rs), and what real turns measure by model
                     and the patterns `Promote` offers for review
                     (measure.rs). The loop gathers facts and carries out
                     what these return
crates/vak-lifecycle the lifecycle reconciler's decisions (plan M7a,
                     docs/design/74 §3.1, §5), with NO vak dependencies:
                     retention labels and the deterministic plan over what
                     was observed (due actions with idempotent keys, what
                     is kept back and why, the classes nobody observed).
                     Observing and committing are `vak_core::lifecycle`
crates/vak-catalog   the data catalog (plan M6, docs/design/73 §9): one
                     SQLite file per tenant of nodes (sessions, turns, calls,
                     files, runs, effects, triggers, memory, commitments),
                     lineage edges and FTS5 over doc 73's text projections,
                     fed by a tailer with one cursor per source
                     (`vak_session::tail`), rebuilt from the records alone;
                     `search` filters by audience before ranking, `lineage`
                     walks from anything to its run and cause; it also
                     holds each session's entry locations, one skew-binary
                     jump pointer per entry for branch membership, and
                     turn records for the turn recall (history.rs). Every
                     search (`/search`, admin, recall)
                     and every "where is session X" goes through it
crates/vak-mail-calendar
                     provider-neutral mail and calendar contracts
                     (docs/design/80-mail-and-calendar.md): account
                     linking and OAuth (oauth.rs), the encrypted
                     credential vault (vault.rs), the provider adapters
                     for Google, Microsoft and Apple (provider.rs,
                     endpoints.rs), the connection ledger and the
                     reviewed send and change effects (effect.rs). Host
                     code admits accounts and grants; an adapter receives
                     one validated typed operation
crates/vak-intake    intake connectors (plan M6.5, docs/design/76): the
                     closed set of source kinds (RSS/Atom, YouTube, Hacker
                     News, Reddit, Lobsters, HTTP), the one URL each
                     fetches, bounded parsers run only in the broker worker
                     (`IntakeParse`), and detection that labels an item
                     accepted, quarantined or blocked and never drops one;
                     and the HTML reader (html.rs) that turns a page
                     webfetch or browse returns into readable text, run in
                     the worker (`HtmlRead`);
                     NO network and NO vak dependencies but vak-ooxml's
                     bounded XML walk
crates/vak-storage   the storage substrate, NO vak dependencies (docs/design/
                     73-data-architecture-and-lifecycle.md §5-§7.3): keys
                     (KeyAuthority, fail-closed wrap/unwrap/rotate/revoke),
                     refs (CAS with generation and writer epoch, memory and
                     SQLite), objects (HMAC-keyed ids, zstd then AEAD under a
                     per-object key, grants per scope) and records (per-entry
                     AEAD frames, hash chain over the stored bytes, so it
                     verifies without keys and after a shred), segments
                     (verified seal: copy, verify, atomic swap, seal entry;
                     crash-safe), scopes (persistent wrapped scope keys with
                     grant/revoke/shred/hold), documents (versions as objects,
                     current as a CAS ref) and the Store trait (LocalStore,
                     MemoryStore, commit generation, pre-ack hook, restore
                     epoch; Remote declared only), the single-writer
                     flock a segment writer requires (`SegmentSet::lock`),
                     and streaming blobs (blobs.rs: chunks plus a manifest
                     object); fuzz targets with a committed corpus and a
                     stable mutation gate (tests/fuzz_corpus.rs)
crates/vak-delivery  schema-v2 channel-neutral semantic output contract,
                     closed AST/compiler, capability projection, safe templates,
                     exact Markdown fallback, ordered chunks, Telegram HTML,
                     and isolated renderer worker; durable delivery is an
                     effect (vak-session effects.rs, invariant 41)
                     (docs/design/30-output-engineering.md); the page an
                     answer is kept in out of its conversation, its own
                     items plus a text form, the one form a saved answer
                     and a living page's versions take (page.rs,
                     docs/design/98-answers-beyond-one-card.md §12)
crates/vak-presentation  the CLOSED primitive vocabulary (Primitive enum in
                     lib.rs) plus the bounded declarative spec, JSON-path
                     binding validator, deterministic compiler, content
                     digest, coverage accounting, and explicit fallback.
                     PACKS (StoredPresentation/PresentationSpec, PresentationOrigin,
                     LibraryScope) are runtime-pluggable and compose existing
                     primitives with ZERO code change; packages/answer-layouts
                     is the worked example. The primitives are the card forms
                     plus layout (docs/design/92-card-forms.md); adding a
                     primitive is the only part that needs a code change
                     (docs/design/57-adaptive-presentation-runtime.md,
                     docs/design/67-presentation-renderer-guide.md)
crates/vak-ooxml     the Open XML package engine, with NO vak dependencies so
                     it is testable and fuzzable alone: L0 bounded OPC reader
                     (content types, relationship graph, detection by main-
                     part content type, security inspection) and raw-copy
                     writer; L1 bounded XML walk that refuses DOCTYPE and a
                     span-aware splice editor; L2 anchored read projections
                     for Word, Excel, PowerPoint and Visio with hidden-content
                     labels; L3 the typed op engine (`edit`) behind
                     `office_apply`; and the built-in blank packages a
                     file created from scratch starts from (`blank`, parts
                     as plain XML in `blank/`). Every vak call
                     site runs it in the broker worker (invariants 14, 39;
                     docs/design/72-openxml-documents.md)
crates/vak-pdf       the PDF engine, written from ISO 32000-2 with NO PDF
                     library beneath it and NO vak dependencies: L0 lexer,
                     objects and bounded stream filters, the cross-
                     reference chain (tables, streams, hybrids), object
                     streams and a rebuild by scanning; L1 encodings,
                     ToUnicode CMaps, fonts and a content interpreter that
                     places and labels text (invisible, white, tiny,
                     off-page); L2 the anchored read projection
                     (`page:<n>/line:<m>`), the security inspection and the
                     client projection; L3 the typed op engine (`edit`)
                     behind `office_apply`, a clean whole-file writer and
                     a Helvetica layout engine for new content; L4/L5 the
                     semantic diff and review choices. Every vak call site
                     runs it in the broker worker (invariant 14;
                     docs/design/77-pdf-documents.md)
crates/vak-sandbox   the isolated-execution contract, deliberately ignorant
                     of models, prompts, sessions, approvals and
                     presentation: environment lifecycle + state machine,
                     the Seatbelt/Landlock/Docker backends, and the
                     reviewable boundary between a task candidate and its
                     destination (docs/design/25-docker-sandbox.md,
                     54-task-environments-and-promotion.md)
crates/vak-tools     read/write/edit/bash/glob/grep/webfetch (with a
                     local-browser `render` mode, webbrowse.rs)/
                     recall behind Tool trait, the one reader
                     of every call's arguments (arguments.rs), a skill's
                     program run as a tool under its own declared sandbox
                     (capability.rs, docs/design/93-self-evolving-
                     capabilities.md), the loopback egress proxy every
                     networked process reaches its declared hosts through
                     (egress.rs), versioned
                     broker-worker protocol, bounded subprocess
                     environment, resource claims, sandbox backends
                     (Seatbelt/Landlock). `recall` is the one way to reach
                     anything from before this turn: `query` searches this
                     conversation's turns and, through Core, other
                     conversations, notes, the profile, sources and
                     deliverables; `open` reopens `#n`, `pres:`, `ev:` or a
                     hit's `turn:`. The agent loop answers it (never a
                     worker) (docs/design/68-context-engine.md §3, §5)
crates/vak-permission rule engine: modes × rules -> Allow/Ask/Deny, over
                     what each tool declares (reach.rs: its reach and the
                     argument a rule matches; no tool names). Deny/Ask
                     match existentially (one bad effect gates the call);
                     Allow is UNIVERSAL — every segment of a compound shell
                     command must be covered, or the call falls through to
                     the mode default. Quote-aware splitting; redirection to
                     a path and command substitution make coverage
                     unprovable. tests/escapes.rs is the standing
                     adversarial corpus and every fix lands a case there.
crates/vak-plugin    plugin packages: manifest parsing, capability
                     declaration (skills/commands/hooks/MCP manifests),
                     workspace- and user-scoped stores, enable/disable/
                     rollback, invocation records, Ed25519 catalog-signature
                     verification with key revocation
                     (docs/design/39-plugin-ecosystem.md)
crates/vak-intent    the decision layer (docs/design/47-commitment-kernel.md):
                     seven behavioural axes, deterministic signal extraction,
                     clause segmentation into strands (strand.rs), the
                     fixed-reader resolution, the autonomy/envelope model, the control
                     plane (`ControlSource`, `parse_command`,
                     `evaluate_intervention` in outcome.rs) and the
                     narrowing lattice. NO vak dependencies -- a pure
                     decision layer, unit-testable without a network, a model,
                     or a config file. `Limits` is a meet semilattice whose top
                     element reproduces pre-kernel behaviour; `meet` is the only
                     composition operator and there is deliberately no `join`.
                     Temporal deixis ("current", "right now", "today",
                     "latest", "recent") is a signal that sets the `live-data` domain
                     without raising the evidence standard -- the vote alone,
                     tried and measured, stalled a small local model into
                     silence (docs/design/68-context-engine.md §7).
                     `control.rs` is the single `ControlKind` vocabulary for
                     every runtime-authored nudge, including
                     `EmptyStep`/`SteeringDrift`; a kind no longer written
                     stays as recorded history.
crates/vak-commit    durable commitments (docs/design/47): lifecycle, the
                     satisfaction lattice, the append-only ledger + projection,
                     and the deterministic portfolio scheduler. Reuses
                     vak-session's CriterionKind/EvidenceRef/WorkOwner rather
                     than defining a second vocabulary for the same idea.
crates/vak-hooks     lifecycle hooks: pre/post-tool-use, stop, session-start
crates/vak-mcp       MCP stdio client behind a lazy meta-tool; the on-demand
                     connection pool (no warm-up, idle eviction, failure
                     backoff) and its observations; each networked server
                     reaches its declared hosts through an egress proxy of
                     its own (`vak_tools::egress`,
                     docs/design/09-extensibility.md)
crates/vak-bus       distributed event + message fabric for agent swarms:
                     CloudEvents envelopes, W3C trace context + Merkle
                     causal lineage, AES-256-GCM payload encryption,
                     subject algebra with role-based ACLs, dead-letter
                     queues, and NATS Core + JetStream alongside an
                     in-memory engine (docs/design/53-distributed-bus.md)
crates/vak-agent     loop, steering queues (full user messages: text +
                     image blocks), parallel tool execution w/
                     resource-claim waves, retries + watchdog + circuit
                     breaker + stop gate (premature-completion guard,
                     which also holds a parent back from finishing with
                     background workers running), worker questions
                     (`questions.rs`), the `workers` tool
                     (`workers_tool.rs`) and the write leases of background
                     writers (`write_lease.rs`), docs/design/84-worker-questions-and-control.md,
                     spend-gate seam (docs/design/15-reliability.md), frozen-ladder
                     leg walk (docs/design/15-reliability.md), goal mode + audited
                     completion + regression obligations + handoff reset
                     (docs/design/42-managed-work-contracts.md), workers (task tool) +
                     parent-scoped WorkerRegistry. The loop orchestrates
                     the context engine (`vak-context`) per step -- plan,
                     assemble, dispatch, write the receipt and capacity
                     feedback back -- and owns the summariser call behind an
                     incremental compaction. Turn-close hook builds and
                     appends each turn's `TurnCard`; the empty-step and
                     steering-drift gates live in the loop itself. Card
                     calls are the loop's own (`show_card`, `cards.rs`):
                     each checked by its tool and recorded with the plan
                     deliverable it answers
                     (docs/design/03-agent-loop.md,
                     docs/design/68-context-engine.md,
                     docs/design/87-composed-answers.md)
crates/vak-context   the context engine, pure functions over the ledger
                     with no I/O (docs/design/68-context-engine.md).
                     `capacity.rs`: the `CapacityProfile` type, the
                     bind-time probe ladder (turn-shaped filler,
                     majority-of-three samples per rung) and usage/latency
                     feedback -- Core drives the probe and records it, this
                     crate owns the math. `planner.rs`: the pure
                     `WorkingSetPlan` -- the open turn verbatim plus the
                     compact card of the turn it follows, within the
                     measured budget; older turns are reached through
                     `recall`, never preloaded; `plan_for_session`
                     is the one entry point the loop, `/compact` and the
                     eval gate all plan through. `assemble.rs`: the
                     byte-stable prefix and its digest, the per-turn tail
                     on the last user message, cache breakpoints, the char
                     accounting that feeds the profile, and the summariser
                     request. The ledger it reads (`TurnIndex`, cards,
                     range-keyed packets, `derive_with_plan`, `recall`) stays
                     in `vak-session`.
crates/vak-flow      static flow DAGs + dynamic planner (bounded replan)
crates/vak-eval      deterministic eval suite + live-model mode +
                     context-engine gate and banned-token gate
                     (docs/design/68-context-engine.md)
crates/vak-config    layered TOML config + atomic persisted workspace
                     preferences + credential-store secret loading (OS
                     keychain or encrypted-file fallback) + canonical filesystem
                     paths (paths.rs: data_home, cache_home,
                     logs_dir) + [finops]
                     caps/pricing + [goal] policy + [route] ladder
                     preferences (docs/design/42-managed-work-contracts.md Phases D+H+R) +
                     [automation]/[update]/[tools] (docs/design/29) +
                     [server] bind/trusted_hosts/public_url/
                     session_ttl_hours/workspace_roots/[server.web] --
                     network exposure, PRIVILEGED in full
                     (docs/design/48-web-client.md) +
                     [providers.ollama] keep_alive/num_ctx (Go-duration
                     validated) threaded to the native Ollama provider +
                     [probe] hosted = "none"|"full" gating whether a paid
                     provider's model gets the full horizon-ladder probe
                     (docs/design/68-context-engine.md §1)
crates/vak-core      SDK facade, system prompt, checkpoints, worktrees,
                     capability/ (the registry for all five extension kinds:
                     domain.rs is the vocabulary capabilities classify
                     themselves against so the harness never enumerates
                     instances, resolution.rs is usability -- available or
                     retired, with nothing probed,
                     snapshot.rs publishes immutable epochs a *turn* binds,
                     registry.rs is the level-triggered reconcile loop plus
                     the immediate revocation channel, provider.rs turns
                     Core's five discovery paths into one declaration set,
                     report.rs is the single projection the model, doctor and
                     the console all render),
                     capability/turn.rs (TurnCapabilities: the single admission
                     pipeline for all capability kinds -- channel → reach,
                     policy only; the reading never removes a capability),
                     capability/surface.rs (ToolSurface: which admitted tools
                     are loaded vs deferred, from each tool's own
                     `Tool::always_loaded`/`serves`/`presents_cards`, plus the
                     reading-independent "More tools" catalogue -- a
                     low-confidence reading loads NO domain tools rather than
                     all of them; a deferred tool is called by its name and
                     loaded from then on, on a wire whose adapter declares
                     `unlisted_calls`; any other wire is sent every admitted
                     tool; docs/design/68-context-engine.md §5),
                     capacity_profile_for (the per-request bind: returns the
                     newer of the ledger-recorded profile and the in-process
                     cache by probed-at timestamp, synchronously, from
                     metadata alone when neither exists yet -- never blocks a
                     turn on a probe; `maybe_start_capacity_probe` runs the
                     horizon-ladder probe as a background task once the turn
                     is done, only when eligible (local, or hosted with
                     `[probe] hosted = "full"`), the cache is missing/stale,
                     and no probe for that key is already in flight),
                     discovery.rs (the warm model discovery behind the turn
                     ladder's fallback legs: one catalogue store per
                     credential, shared across the gateway pool, kept fresh
                     by a background loop that asks providers only while the
                     Core is planning turns, never on the turn path;
                     docs/design/15-reliability.md),
                     prompts.rs (`Resolution{text, tail}` -- the stable
                     prefix and the per-turn tail are two separate strings
                     since 3.5.0; see docs/design/07-prompt.md),
                     mcp_config_section (server names + tool names only, no
                     descriptions or schemas -- MCP is reached only through
                     the `mcp` broker),
                     intent.rs (the seam: gathers facts, runs the cascade,
                     projects the engagement onto runtime knobs -- every
                     function takes a baseline and returns something no
                     wider; `open_threads` for strand lineage,
                     `DeferringApprover` for the `Defer` HIL mode),
                     commitments.rs (opens or
                     continues one commitment per durable thread,
                     brackets an episode per strand, classifies what it achieved,
                     evaluates workspace criteria, and runs the zero-token
                     upkeep pass: schedule wakes, predicate wakes, escalation
                     policies, explicit expiry), tools_commitments.rs (the
                     read-only `commitments` capability),
                     elsewhere.rs (what `recall` reaches beyond its own
                     conversation, by Agent and audience), memory/
                     skill-proposal tools + duplicate screening
                     (docs/design/26-learning.md, 29 P5), sandbox
                     selection incl. Docker exec backend + runtime backend
                     override (docs/design/25-docker-sandbox.md), manual
                     compaction (compact_session_now), runtime MCP table
                     hot-apply, cost ledger + budget admission gate +
                     alert rows (docs/design/15-reliability.md), trigger
                     store + cron engine (triggers.rs), health report, backup export/import,
                     digest, shared transcript_md renderer
                     (docs/design/29-personal-os.md) + AgentDefinition store,
                     Agent/ConversationKey admission and lifecycle resolution,
                     scoped ownership/provenance for sessions, schedules,
                     child runs, and delivery (docs/design/64-agent-owned-platform.md)
crates/vak-server/site
                     The public website at `/`, `/surfaces`, `/security`,
                     `/install` — src/ built by build.py into a COMMITTED
                     dist/ embedded with include_dir!. motion.dev vendored,
                     not CDN-loaded: the box may have no public route
                     (crates/vak-server/site/README.md,
                     docs/design/48-web-client.md)
crates/vak-server    HTTP+SSE wrapper (sessions/runs/approvals/transcripts/
                     worker steer-stop/MCP management/memory
                     CRUD/search-all/transcript.md/doctor/backup/
                     digest/inbox-ack endpoints) + always-on gateway:
                     chat-surface routing, persisted bindings,
                     cron+script-watchdog scheduler w/ catch-up and model
                     pinning, heartbeat proactive check-ins, budget-alert
                     delivery-to-surface, approval forwarding with
                     addressed-gate resolution (Telegram forwards render as
                     inline-keyboard buttons via TelegramAdapter, resolved
                     through the same verdict-text path a typed yes/no
                     uses), semantic adapter registry, Telegram document
                     attachments (small text inlined, everything else
                     saved to the workspace inbox and named; image
                     attachments stay vision content), Telegram/webhook
                     transports, effect dispatch and replay
                     (delivery.rs, effects.rs: /effects, resend, reconcile)
                     semantic presentation snapshots/SSE alongside legacy
                     AgentEvent and transcript endpoints
                     (docs/design/22-gateway.md, 28-operations.md,
                     29-personal-os.md, 30-output-engineering.md) +
                     Agent-owned endpoint routing, request receipts, audience
                     isolation, lifecycle-gated admission, and bot-scoped
                     delivery provenance (docs/design/64-agent-owned-platform.md) +
                     multi-bot-per-channel: `Bot` identities independent of
                     surface, bot->chat->workspace policy/permission/route
                     resolution chain, bots.json store, /gateway/bots CRUD
                     (docs/design/34 Phase 5) + per-bot/per-chat voice
                     override riding the same inheritance chain and
                     `inherit_bot_policy` switch as route/permission_mode,
                     POST /voice/speak (docs/design/38-voice-personality.md) +
                     admin console: global event hub + SSE,
                     /admin/api/* data plane, embedded SolidJS SPA at
                     /admin (docs/design/33-admin-console.md), including
                     memory CRUD/cleanup, live parent-scoped worker
                     controls, and the evidence-backed Operations Center
                     (`/ops/center`, durable incidents, action receipts, and
                     bookmarkable resource drill-downs) +
                     the BROWSER SURFACE (web.rs, docs/design/48-web-client.md):
                     ONE login for every browser client
                     (/auth/login|logout|session, HttpOnly SameSite=Strict,
                     Secure only behind real TLS -- a Secure cookie over
                     plain http is silently discarded and the session then
                     never persists); /host + /stream standing in for
                     the desktop shell's backend_info; /workspaces{,/open}
                     over CorePool; /fs/dirs (folder names only, rooted at
                     [server] workspace_roots); WS /pty, off by default and
                     loopback-pinned when on. Host-header pinning is
                     `host_is_trusted` (loopback + exact [server]
                     trusted_hosts; wildcards refused), cross-origin
                     mutations are rejected before routing, and ?token= is
                     loopback-only. events.rs adds EventBus: every session
                     event carries a seq, the last 1024 are retained, and a
                     Last-Event-ID reconnect gets exactly the gap -- or an
                     explicit `resync` frame when the gap is wider than the
                     ring. embedded_ui.rs holds the ONE asset-serving path
                     shared by /admin and /app (recursive registration; the
                     non-recursive version shipped broken three times)
crates/vak-client-ui SolidJS + Vite WORKSPACE client, shared by the Tauri
                     shell and the browser (docs/design/48-web-client.md).
                     src/host/ is the seam: a `Host` port with a Tauri and a
                     web implementation, selected by a BUILD-TIME alias
                     (#host-impl, vite.config.ts) so the web bundle never
                     carries the Tauri IPC layer. No component imports
                     @tauri-apps or sniffs for window.__TAURI__; they ask
                     host.can(...) and render DIFFERENTLY, not brokenly -- an
                     absent capability is a design decision, not an error
                     state. Builds twice from one source: dist/ (base "./",
                     shipped by vak-desktop, gitignored) and dist-web/ (base
                     "/app/", embedded by vak-server, COMMITTED so a headless
                     box builds the server without node). The primary navigation
                     is Agent-first: one durable conversation per Agent, with
                     channels, history, and internal work behind settings/details
                     (docs/design/64-agent-owned-platform.md)
crates/vak-admin-ui  SolidJS + Vite admin console source; the commitment
                     portfolio (#/commitments) with the evidence meter --
                     the satisfaction lattice drawn, achieved as fill and
                     required as a rule beneath, shortfall in the accent
                     (docs/design/47-commitment-kernel.md, DESIGN.md); built dist is
                     committed so cargo builds need no node — observation,
                     operation, and interaction views per docs/design/
                     33-admin-console.md, including the six-area navigation,
                     persistent Operations context bar, and raw-evidence
                     drill-downs, plus a VoiceConfigEditor
                     (inherit-toggle + live Preview button) on the per-bot
                     and per-chat panels (docs/design/38-voice-personality.md)
crates/vak-terminal  the rich terminal surface behind `vak term`: a ratatui
                     app that is a CLIENT of a real vak server over HTTP/SSE,
                     never a second runtime -- health, sessions, routes, MCP
                     inventory, approvals and telemetry are fetched live and
                     kept fresh by background watchers
                     (docs/design/55-rich-terminal-surface.md)
crates/vak-desktop   Tauri 2 SHELL over an embedded secured_router. The UI
                     itself lives in crates/vak-client-ui (above); this crate
                     is the native half -- window/tray lifecycle,
                     single-instance handover, workspace trust gate, PTY
                     commands (spawn/write/resize/CLOSE -- the close is what
                     scopes a shell's life to its pane), and the native save
                     dialog. Renders the schema-v2 outcome-first semantic
                     timeline/AST registry: sessions, split view, approvals, diff
                     review, workers tab, MCP manager, image attachments,
                     best-of-N, tasks (cron/script/pin), side chats,
                     memory tier editor, global search, diagnostics,
                     backup/digest cards, budget banner
                     (29-personal-os.md) +
                     voice narration of turn completions and approval
                     prompts via a shared `<audio>` element and
                     `/voice/speak` (no native audio crate — the webview
                     plays the returned WAV Blob), Settings toggle + voice
                     picker (docs/design/38-voice-personality.md)
crates/vak-ops       service-control layer over launchd/systemd — status,
                     start/stop/restart, install/uninstall shared by tray,
                     CLI and desktop, and each service's structured log
                     readers (docs/design/28-operations.md)
crates/vak           binary. Run: exec / plan / flow / eval /
                     serve [--host] [--gateway] /
                     telegram|discord|slack [--bot-id <id>] /
                     term (the rich terminal surface, crates/vak-terminal) /
                     open [app|admin]. Inspect and steer: sessions /
                     export / checkpoints / memory / entities / agents /
                     triggers / runs / effects / inbox / intent (explain,
                     show) /
                     commit (list, show, close, supersede, attest) /
                     grant / revoke / office (read, apply, diff, verify,
                     for Office files and PDFs: JSON over the same worker
                     tasks as the Agent's tools, docs/design/72-openxml-
                     documents.md, 77-pdf-documents.md). Configure: config /
                     prompts / skills (validate, verify, and the learned
                     abilities: list, show, use, approve, decline,
                     promote, turn-off) / plugins / setup / doctor / backup /
                     digest / self (state, install, reinstall, verify,
                     update, uninstall, status, services-sync), plus the
                     first-run wizard and the opt-in update check.
                     `user` and `project` are SCOPE arguments on config,
                     prompts and plugins, never subcommands. There is no
                     `tui` subcommand: the inline TUI shipped in v0.1.x and
                     was withdrawn; `term` is its replacement and is a
                     client of a running server, not a second runtime.
docs/design/         architecture decisions — update with behavior changes;
                     security boundaries and roadmap in 24-agent-security.md
scripts/             dev utilities (mock servers, PTY/HTTP smoke drivers)
```

Two same-named module sets that are not duplication: `vak-server/src/surfaces/`
holds the chat-surface TRANSPORT adapters (long-poll and webhook bridges into
`POST /gateway/inbound`), while `vak-delivery`'s modules of the same names own
the MARKUP projection for each surface. Both once sat at `src/telegram.rs` in
their own crate and read as a copy until you opened both.

## Acceptance-workspace contract

Real acceptance runs must use a disposable workspace outside the vak source
checkout (for example, a fresh directory under `/tmp`). The checkout is the
implementation workspace and must not become the destination for generated
dashboards, documents, package caches, local servers, or scratch artifacts.

For generated rich output, an acceptance run is incomplete until it has all of
the following evidence:

1. An explicit output scope (for example `--write-path dashboard.html`) was
   admitted before the tool call.
2. The runtime successfully wrote the output and read the exact path back.
3. Any local server or renderer actually started; a command that prints a
   success message while its child exits is a failed verification.
4. Browser-facing output was inspected in the real browser when the result is
   interactive or visual. A syntax check or model assertion cannot substitute
   for this.
5. The candidate remains quarantined until a user reviews and promotes it.

Tool errors and failed verification are authoritative. Model prose must never
turn a rejected write, missing file, failed server, or failed browser check
into a successful outcome. See
`docs/audits/acceptance-dashboard-2026-09-08.md` for the reproducible dashboard
scenario and residual limitations.

## Investigating a failure

A failure is never closed by naming the model. "A small model", "model
quality" or "the model got confused" is not a cause, and a session that
writes it in a commit, a doc or a report has stopped investigating too
early. When a model's call fails, read the call it made and what the
runtime answered, then find what in Vak's own contract made a reasonable
attempt fail: a tool schema that refuses an unambiguous shape, an
envelope that repeats what the tool's name already says, a prompt or
block that points at the wrong tool, a result format the model copies
back. Fix that and record the cause. If a failure still remains after
the contract is right, say exactly what the model did and what Vak told
it, with the evidence, and leave it as an open question rather than a
verdict about the model. (Recorded 2026-10-07: a "small model" verdict
hid that `emit_table_card` refused a correct flat `{columns, rows}`
table; its envelope was the cause.)

## Tool arguments are written, then read

What the measurements on 2026-10-11 and 2026-10-12 established
(docs/audits/cards-in-answers-2026-10-11.md, docs/design/89-constrained-tool-arguments.md),
for every provider and model:

- A tool call's arguments are text the model writes. Most failures are
  syntax (a stray `}`, a raw newline in a string, a list written as text),
  not a misunderstanding, and grow with how long and how deeply nested one
  argument is: one nested call of every card block was broken JSON in 7 of
  16 captured steps, separate card calls with precise schemas in at most 1,
  and one union schema shared by every block was worse than either.
- Telling the model and showing the schema lowers that rate and cannot
  remove it. What removes it is a schema the wire enforces (constrained
  decoding): strict tools on OpenAI, Anthropic and Bedrock, a schema-bound
  output on Gemini, Ollama, OpenAI-compatible servers and the rest; it was
  valid in 16 of 16 on the dev route. A grammar guarantees structure only:
  content stays the builders' and the runtime's checks.
- Whether a wire and model enforce a schema is a capability declared by the
  adapter (`REACH`: `strict_tools`, `schema_output`) and learned from
  refusal, like `tool_choice`, never inferred from a provider's name. Tools
  go strict where the wire enforces them (`vak_llm::strict`, card tools
  first within a wire's limits), and a step whose output is data sets
  `ChatRequest::output_schema` rather than parsing JSON out of prose.
- A call whose arguments cannot be read (`vak_tools::unreadable`) or that
  its tool's schema refuses is written again under the tool's own schema (`Tool::call_schema`, which an
  integration broker narrows to the discovered tool) before it is ledgered;
  the ledger keeps what the model first wrote. A card call whose kind
  another card tool owns is not rewritten: it is told that tool. Telling
  the model first left the rewrite to its calling the tool again, which it
  may not do (a steps card with no `steps`, 2026-10-14). Never add a repair
  that guesses at broken JSON: the regeneration is the repair.
- Judge a change to how arguments are asked for by replaying Vak's own
  captured requests on each route (`scripts/measure_tool_form.py`), never
  by a toy schema: the first measurement used toy schemas and missed the
  fault entirely.
- Keep one canonical schema per tool, precise and flat (doc 30 §30.1), and
  one name for one shape across tools (`one_argument_name_is_one_shape_on_every_block`).
  Never merge several tools' schemas into one, and never nest a whole
  answer in one argument.

## No word lists, no tuned limits

Behaviour is never decided by a list of words or a number picked to make a
case pass. A failure is not fixed by adding a phrase to a list, a topic rule
to a builder, or a threshold or cap to a loop; when one is found it is
removed, and the change is verified with real turns.

- Whether a request refers back, needs a tool or asks for a card is not read
  from its vocabulary. The planner carries the card of the turn a request
  follows and nothing older; the model plans its own deliverables and the
  runtime checks receipts (docs/design/86-model-plans-runtime-checks.md).
- A loop that must end does so by a sequence of observed states, never a
  count: a failing tool is hinted, then shown its schema, then withdrawn for
  the turn; a tool the person declined is withdrawn for the turn at once;
  a repeated answer or a repeated card is told once, and the next
  one stands as the model's answer. The operator's configured turn limit is
  the only number that bounds a run.
- No runtime ending replaces the model's answer with words it did not write,
  and nothing already produced is discarded to end a turn.
- No domain, provider or tool is first-class: nothing names one to make it
  work.

A tool's permission is what it declares, never its name: its reach and
the argument a rule matches (`Tool::permission`, `vak_permission::Declared`,
docs/design/08-permissions.md "Declared reach"). The engine, rules, hook
matchers, remembered approvals and `auto_approve` read the declaration; a
new tool declares one, and a check that is not a tool call (a provider
effect) states its reach where it checks. Add no table of tool names to
`vak-permission` or its callers, and decide nothing else by a tool's name
either: a tool declares what it is (`permission`, `provenance`,
`leaves_machine`, `prompt_contract`,
`file_access`, `delivered_file`, `presents_cards`, `output_of_command`,
`fill_defaults` for a
name it accepts for an argument), and a wrapper forwards every
declaration. Only the loop's own tools (`plan`, `recall`, `task`, `work`,
the card calls) are recognised by name, because the loop defines them.

A tool error says where its fault lies as a typed value at its source
(`ToolOutput::error(Fault::…, …)`, `vak_control::Fault`): add no reader of
an error's words, and give a new failure the fault its author knows.

Every correction goes through the one ladder (`vak_control::Ladder`): a
failed check is told once with a typed `Finding`, then the frame changes or
the answer stands. Add no once-flag, block budget or repeat count of your
own, and write a correction's wording only in `vak_control::render`.

Nothing is ended because a number of seconds picked in code went by: a wait
ends when the work finishes, someone stops it, it goes silent, or the
operator's configured limit is reached (docs/design/88-control-plane.md
§5a, which lists what is still to be brought under this). The operator's
limits are `[control.limits]` (`vak_control::ControlLimits`, read through
`Core::control_limits`): add a wait's limit there, shown in Settings beside
its shipped default, never as a constant. A tool whose call may be bounded
names the argument that bounds it (`Tool::time_limit_argument`), and the
loop fills it; nothing fills a limit by a tool's name.

## Verification before every commit

```
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
scripts/check-version.sh && python3 scripts/check_doc_paths.py && python3 scripts/check_records.py
scripts/check-advisories.sh
```

`check-advisories.sh` runs `cargo audit` (install it once with
`cargo install cargo-audit --locked`). A known vulnerability in
`Cargo.lock` fails it. An advisory may be ignored in `.cargo/audit.toml`
only with the reason Vak cannot reach the vulnerable code written beside it,
and an open item that removes the exception.

`scripts/gate.sh` runs exactly that and times each step. While working,
`scripts/gate.sh --changed` checks only the crates changed against
`origin/main` and every crate that depends on them; it never stands in for
the full gate before a commit. Batch several finished steps under one full
gate, then commit each step on its own.

**One integration-test binary per crate.** A crate's integration tests are
modules of `tests/it/main.rs`, so the crate graph links once rather than
once per file (220 test binaries became 73 on 2026-10-12). A test file
stays in `tests/` as its own binary only when it needs its own process:
it acts on the whole data home (a restore, a sync, key rotation, a
lifecycle pass, erasure, holds, an integrity check, a catalog rebuild) or
it changes process-global state (a `set_override`, the telemetry
subscriber, the global config file, a count of syncs or fences). Tests in
one binary share one isolated home, so a test there reads only what it
made, never a whole list it did not filter. A test that fails only beside
others is a test that reads more than it made: fix that, or give it its own
binary if what it acts on really is the whole home. Never split a test out
just to make the failure go away.

**Touched a frontend or the public site? Rebuild it, or the binary ships
the previous one.**

```
cd crates/vak-client-ui && npm run build   # BOTH bundles: dist/ + dist-web/
cd crates/vak-admin-ui  && npm run build
python3 crates/vak-server/site/build.py    # the public site at /
```

`vak-client-ui` builds twice from one source — `dist/` for the Tauri shell
and `dist-web/` for the copy `vak-server` embeds at `/app` — and only
`dist-web/` and the admin bundle are committed, because a headless box must
build the server without node. The public site is the third embedded bundle
and the only one needing no npm; its `dist/` is committed for the same
reason, so a clone needs no Python either. All three refuse to compile
against a bundle whose `.src-manifest` no longer matches `src/`, so this is
a build error rather than a silently stale UI; `scripts/release.sh` and CI
additionally fail on a committed-bundle diff, which a local build cannot
see. `.gitignore` has to keep negating each of them out of the blanket
`dist/` rule, or a whole bundle silently stops being committed.

**A test never reaches the operator's real homes.** A cargo test binary
(always run from a `deps` directory) isolates its home on its first path
resolution unless it pinned one with `set_home_override`, and keeps its
secrets in an encrypted file, never the OS keychain
(`vak_config::paths::isolate_home_for_tests`, `home_is_isolated_for_tests`).
Calling `isolate_home_for_tests()` first still names the home a test works
in. A test that seeds space-keyed state (ledgers, tasks) for a folder binds
the folder first (`vak_config::spaces::bind`), as opening it would. Before
this was automatic, a test that forgot to isolate read the operator's real
Shared layer (`~/vak-home/.vak/config.toml` and the real Shared secret
scope), so the suite exercised whatever that machine had configured, and a
test that opened a temp folder bound it in the real space registry.
Twenty-one test files did the first: a real MCP server was advertised inside tests, and a
personal provider key could make an "unconfigured" case pass on one machine
and fail in CI. Neither
`std::env::set_var` nor `unsafe` is needed for this; the override map sits
above the real environment in `get_var`'s precedence. That home is shared by
every test in the binary, and every `Core::new` reads its Shared config, so a
test that writes `vak_config::global_path()` goes in a test binary of its own,
on a private home per test held under one lock, as
`crates/vak-server/tests/shared_config_layer.rs` does. Two such tests once
sat beside three hundred readers, and removing the file between another
test's existence check and its read failed `Core::new` under full-suite load.

`check-version.sh` covers the stamps a user reads (README badge, CHANGELOG,
git tags), not just the ones the build system reads, and fails on a version
that moves BACKWARDS unless the abandoned line is declared in
`scripts/.version-reset` — a decreasing version makes every install on the
higher line permanently un-updatable, and walks into tag space the earlier
line already used. Both happened; see the release-discipline rules in
`docs/design/32-release-engineering.md` before cutting a release, and never
move a version line backwards. `check_doc_paths.py` fails on a design
doc citing a path that no longer exists; docs whose `Status:` line says
"proposal" are skipped, because their paths are targets rather than
citations.

Live checks (needs a provider key set via the Settings UI, `PUT /config/key`,
or an environment variable):

```
target/debug/vak eval                    # deterministic suite, ~100ms
target/debug/vak eval --live             # real model benchmark
```

## Live development runs

`docs/development.md` is the step-by-step guide to building, component
harnesses, and running a dev build live against a real model in the
browser. Read it before your first live run; the rules below are the ones
that cost a session time to rediscover.

- Build both binaries: `cargo build -p vak -p vak-server --bins`. The
  worker binary is what runs tools, verifiers and Office reviews.
- After a client change, rebuild the bundle, then the server binary, then
  restart the server; it serves the bundle it was compiled with.
- Run live in a disposable `/tmp` workspace on a port other than the
  installed service's (`8901`), against the default data home. A copy of
  the data home does not carry the provider connection. Never pass
  `--gateway` for a dev run.
- Sign in with a throwaway `VAK_GATEWAY_TOKEN` pinned for that server
  process, and get the link from `vak open app --print --port <port>`.
- Never open credential files or read ledgers in the real data home;
  inspect through the app and its HTTP API with the bearer token.
- Keep one browser tab per server origin: several tabs exhaust the
  browser's per-host connections and requests hang silently.
- Stop the server and delete any data-home copy when done.

## Parallel agents

Only touch files you changed in this session. Sessions are partitioned by space id;
never edit another session's files under the data home.

Two homes, easily confused, and `crates/vak-config/src/paths.rs` is the only
place that decides either:

- **The data home** — application-managed state (sessions, index, install
  manifest). `~/Library/Application Support/vak` on macOS,
  `~/.local/share/vak` on Linux, overridden wholesale by `VAK_HOME`. Never a
  dotdir; a test asserts it.
- **The default workspace** — `~/vak-home`, a plain directory a person can
  `cd` into. It is what durable services run from (invariant 18) and what the
  Shared config layer and Agent workspaces live under (invariants 27 and 37).
  It must never collide with the data home.
