# Turn relations, context and answer sizing — 2026-10-21

Status: historical. Review of source at `292d717e6`, the last two installed
runs in the owner's default workspace, their recorded interface, and the
installed desktop. Includes a proposal, not an implemented policy or an
approved replacement of invariant 36. Consulted Astra independently.

## Finding

The current system can retain N turns and recall distant records, but that
does not establish reliable N-turn reference resolution or bounded planning
cost. It has no single binary related/unrelated decision. Lexical lineage,
one-card context admission, model-directed recall, persistent-work overlays,
the model's deliverable plan and client layout each decide different things.
Their boundaries are partly sound and partly misleading.

The two samples show progress: both requests opened their own threads,
looked up current sources and produced structured answers without reopening
older turns. They also show unnecessary carried context and substantial
input/output volume. They do not establish a universal failure rate, prove
that old context caused verbosity, or justify subject-specific changes.

## Scope and method

Read the documentation guide, agent contract, blueprint attention section,
context engine, turn graph, model-plan/runtime-check contract, control plane,
composed answers, card forms, large-answer design, prompt layers and seed,
and the lifetime context-selection repair plan. The authoritative current
policy is invariant 36 and doc 68 §4; old graph promotion examples and the
repair plan's older immediate-anaphora policy do not describe today's
one-card planner.

Read source for intent/history facts, graph extraction, ordinary/selected
planning, capacity, recall search/open, request assembly and tail sections,
card composition and responsive/Canvas sizing. Read the shipped layout
skills as product data; they were not invoked as instructions for this
review. Astra independently reviewed the design and source, including the
visual composition path.

Installed inspection used `vak sessions`, `vak runs list/show`, `vak data
cat` and authenticated read-only session `context`, `receipts`, `snapshot`
and `interface` APIs. The latter returns the latest turn's ledger-recorded
prompt and admitted schemas, not merely today's source seed. Authentication
remained in process memory. No ledger bytes, credential files or data home
were copied or directly decoded. Opened the installed desktop through its
normal UI and inspected the weather answer. No new model run, data reset,
install, release or runtime setting change was made.

## The two installed runs

Both belong to conversation `01a13fe9-002e-7f43-a4e3-80a88b4a0312`, on
`openrouter-responses`, `minimax/minimax-m3.1-flash:free`. The installed
service's health reported build parity, workspace `~/vak-home`, and an
unknown context window. Times below are IST on 21 October.

| | World-news request | Current-weather request |
|---|---|---|
| Run | `run_01a15c2a-8a0d-7892-9185-1fbd2b812d06` | `run_01a15c76-f5c1-7af0-80ef-34e3ee2f1028` |
| Turn | 40 | 41 |
| Started | 07:24:14 | 08:47:42 |
| Elapsed to settlement | 219.9 s | 28.0 s |
| Execute dispatches / attempts | 17 / 17 | 3 / 3 |
| Provider latency, sum | 205.7 s | 25.0 s |
| Prompt tokens, first → last dispatch | 20,754 → 129,088 | 20,423 → 31,945 |
| Prompt tokens, sum across dispatches | 1,676,027 | 83,860 |
| Automatically carried history | Prior AI-news card, 489 estimated tokens | Prior world-news card, 458 estimated tokens |
| Recorded relation | Own thread | Own thread |
| Recall / skill calls | None / none | None / none |
| Accepted cards | 9: summary, 7 details, sources | 6: summary, details, stats, chart, callout, sources |
| Card fallback text | 23,411 characters | 3,524 characters |

Prompt counts include cache-read input. Their cumulative sums count the
growing prompt repeatedly; they are not unique retained history. Counts were
correlated by the last two turns' transcript boundaries, their 17/3 context
plan records, run decisions, and the corresponding last 20 execute receipts.
Neither run's growth came from recalling the whole old conversation: most
growth was current-turn source material and output. The weather turn's first
receipt estimated 19,907 prefix tokens against 20,423 reported prompt tokens;
schema/prefix cost matters alongside history selection.

The weather model independently expanded a direct current-condition question
into an outlook, an hourly chart and extra source comparison. All its cards
set `beside_previous: false`, and the recorded outline has no beside groups.
The installed desktop draws a long summary paragraph and bullets before the
details card; this is observable duplication/volume, not a demonstrated
50/50 pairing defect in these particular runs. Both answers display Partial
result/verification cautions despite completed run records. Execution ending,
delivery, evidence support and confidence must remain separate facts.

The recorded latest interface contains a 12,564-character system prompt and
43 admitted tool definitions: about 90,768 compact-JSON characters, about
42,837 for `emit_*` tools. These are the bound interface inventory, not an
exact reconstruction of each provider request's stubbed/deferred wire schemas.

## How relatedness actually works

1. `vak_core::intent::open_threads` scans intent records, retaining twelve
   distinct recent threads. Each descriptor uses the latest strand's words,
   not the accumulated subject of the thread. This is a candidate horizon,
   not a deletion of older history.
2. `vak_intent::resolve::lineage_for` filters by act or deictic signal,
   requires two shared topic words covering at least half the older keyword
   set, or accepts a deictic follow-up of at most three content words with
   the same act. A presentation-word blacklist suppresses generic overlap.
   Ties prefer the latest matching candidate. Natural act changes,
   paraphrases, longer references, languages and older work are fragile.
3. A strand has one primary lineage and can name merged threads. Session
   extraction preserves those as `ContinuesThread` links. Explicit correction
   and replacement commands have special handling; ambiguous commands can
   fall back to the most recent open thread.
4. `vak_context::planner::plan` finds the newest visible, nonempty stopped
   exception-adjusted closed turn matching any continued thread, otherwise
   the latest visible closed turn. It admits at most that one compact card
   if it fits. Thus lexical inference indirectly chooses model-visible
   context even though the planner itself does not classify the request.
   Multiple continued threads still yield one card.
5. The two-hop/eight-turn-per-node graph walk computes diagnostics; it
   does not expand admission. File/page/commitment/recall links do not
   automatically load their historical records. A recorded recall link is
   evidence that something was consulted, not proof it supports this request.
6. The model can search/reopen through always-loaded `recall`. This is the
   route to work 1,000 turns back, including a stable turn id when known.
   Ordinary same-conversation search uses catalog FTS/BM25 over OR-combined
   subject terms, at most 64 candidates before branch/aside filtering, and
   returns at most 20. These are bounded candidate results, not a semantic
   classifier or a guarantee that the relevant old turn was found.
7. Persistent work/goal tail sections also consult inferred thread
   continuity (`tail_work_contract`, `tail_conversation_thread`). Reviewing
   only planner admission misses another path through which an incorrect
   lineage can alter the request.

Primary source locations: `crates/vak-core/src/intent.rs:100`, `:189`;
`crates/vak-intent/src/resolve.rs:440`; `crates/vak-intent/src/strand.rs:54`;
`crates/vak-session/src/turns.rs:318`; `crates/vak-context/src/planner.rs:106`,
`:203`, `:240`; `crates/vak-tools/src/recall.rs:36`, `:260`;
`crates/vak-catalog/src/history.rs:638`;
`crates/vak-session/src/log.rs:3482`, `:3523`.

## Sizing and scalability findings

- **Bounded prompt history does not mean bounded planning.**
  `plan_for_session` reconstructs the full TurnIndex and estimates every
  closed turn's Full and Card projection before admitting one card
  (`crates/vak-context/src/planner.rs:368`). The graph also builds holders
  from all closed turns. UI history paging does not fix this runtime path.
- **Unknown capacity is encoded as effectively unbounded.**
  `CapacityProfile::from_metadata_only` substitutes `u64::MAX` when no window
  is known (`crates/vak-context/src/capacity.rs:226`, `:315`). The installed
  plans consequently expose budgets near 18 quintillion tokens. This is a
  sentinel, not a measured model capacity, and disables a meaningful
  preventive ceiling until metadata, feedback or a rejection bounds it.
  It did not flood these requests with old turns because admission is one
  card; current-turn growth remains material.
- **The selected planner is not the production solution yet.**
  `plan_selected` exists but ordinary dispatch calls `plan_for_session`
  (`crates/vak-agent/src/lib.rs:1567`). Its sequential Full-before-Card
  allocation also lacks coverage-first fairness across multiple dependencies.
  The lifetime plan explicitly says the indexed selection/ContextSelection
  activation is unfinished. Reconcile its obsolete byte-cache/storage
  references with today's catalog before implementation.
- **Some tests/doc examples name behavior they no longer prove.**
  `linked_turn_promoted_without_shared_words` now asserts only the preceding
  card, no retrieved turns and empty link diagnostics (`planner.rs:655`).
  Doc 85's old Full-promotion examples are not evidence of today's behavior.

## Prompt and tool help, and sources of confusion

The recorded prompt correctly tells the model to distinguish fresh,
continuation, correction and historical questions, use recall at any age,
reopen candidates, preserve historical dates, and treat old approvals as
non-authoritative. The recall schema exposes exact open values and specific
query search; it distinguishes missing from unavailable history. These are
useful existing mechanisms to strengthen, not replace with mandatory model
classification before every request.

There are pressures in the other direction:

- The prompt says `<turns>` is “the turn this request follows,” even for the
  unrelated automatically carried card. This describes a semantic relation
  more confidently than runtime evidence supports. `TurnCard::line` carries
  the prior directive and unmet deliverables, not just a neutral reference.
- “Fresh request needs no recall” is useful, but there is no equally clear
  explanation that remembered entity facts, constraints or previous results
  can be needed without repeating the older request's exact words. The model
  should retrieve when an answer depends on missing history, including a
  question whose historical dependence is implicit; uncertainty about a
  referent is a reason to query, not to inherit the carried request.
- The presentation contract encourages a summary and as many cards as parts;
  the plan tool repeats the parts-to-cards rule. “Keep answers concise” is
  broad advice against that concrete instruction. There is no explicit
  separation between requested coverage and optional enrichment, or instruction
  to avoid repeating the same finding across several shapes. These are
  plausible volume pressures, not proven causes from two observations.
- Layout skills prescribe multi-card sequences. Neither inspected run loaded
  one, so their bodies cannot explain those runs. Their inventory descriptions
  are present. Evaluate skills only when they actually enter a request.
- The prompt and webfetch contract accurately say source discovery is
  unavailable. The news run nonetheless made 41 webfetch calls. Recall is
  historical discovery, not a substitute for public source search. Do not
  confuse missing capabilities with relation or context-selection failures.

Source: the latest installed `interface` response; source equivalents are
`crates/vak-core/src/system-prompt.md`, `crates/vak-tools/src/recall.rs`,
`crates/vak-tools/src/plan.rs`, and `packages/answer-layouts/skills/`.

## Visual layout findings

These are source-level findings; the two samples use no explicit pairs.

- `composeAnswer` moves the first summary and groups by plan sections before
  resolving `beside_previous`. Its new predecessor can differ from the
  original one, including across sections
  (`crates/vak-client-ui/src/components/PresentationRenderer.tsx:1249`).
- Paired blocks become equal half-width columns above 560px without form
  minimum widths (`crates/vak-client-ui/src/styles.css:3060`). A wide table
  and short note receive the same width policy.
- LiveAnswer bypasses the settled composer. Canvas uses CSS columns of
  24rem instead of that grid (`PresentationRenderer.tsx:1289`,
  `styles.css:6799`), so placement semantics differ across surfaces/states.

Relatedness must not decide pixel widths. A continuation can need a whole
table; a fresh answer can fit in a sentence. Card count also does not reveal
requested depth or semantic complexity.

## Proposed clean contract — not yet approved or built

Keep three distinct objects, reusing existing ledger/catalog primitives:

1. **Addressed historical facts and relations.** Turns, strands, answer
   blocks, artifacts/versions and evidence have stable addresses. Relations
   name their role and origin: human reference, observed production/version
   relation, or model-inferred dependency. A retrieval records “consulted”;
   it does not certify relevance, truth, approval or a merged thread.
2. **A per-request selection.** The current request declares/retrieves the
   missing dependencies it actually needs. Runtime resolves addresses,
   verifies branch/access/lifecycle, measures costs and records what entered
   context. Activate/consolidate the existing ContextSelection design rather
   than adding another index or selection log. Model choices use existing
   plan/recall calls; no mandatory classifier side-call or forced recall for
   an independent question.
3. **An answer document.** The model chooses current deliverables, blocks
   and explicit group/revision relations. The renderer chooses geometry
   using form needs and actual space. Live, settled and Canvas draw the same
   addressed document through the same composition rules.

“Related” is a set of relations, not one boolean for the entire turn:

| Relation | What may carry | What does not follow automatically |
|---|---|---|
| Continue work | Needed prior result, constraints and current state | Every earlier step or an old authorization |
| Correct/supersede | Exact target and the fact corrected | Prior claim stays current |
| Transform/reformat | Addressed content being transformed | Repeat the old research or unrelated blocks |
| Compare/combine | Several independently addressed sources | Collapse their identities into one best thread |
| Ask about history | Original dated evidence/result | Refresh replaces the historical answer |
| Refresh current state | Subject/units/preferences plus fresh evidence | Old measurements become current |
| Reuse a constraint or person fact | The applicable scoped fact | All instructions from its source turn |
| Background association | Candidate descriptor, if helpful | Admission of its full record |
| Ambiguous referent | Candidate discovery/open, then clarification if needed | Pick latest silently |
| Independent request | Its own task and genuinely applicable standing facts | Prior unfinished deliverables become new work |

The same turn may have several of these relations to several records. This
table describes general roles, not domains, intent keywords or routing enums
that force behavior from the user's words.

### Discovery at arbitrary distance

Exact UI/voice references and stable ids resolve directly. Otherwise the main
model uses the available scoped query/open tools when missing history could
change the answer, with paraphrases/entities/artifact references as needed.
Bounded lexical candidates are allowed to be uncertain; an optional configured
semantic projection can improve discovery, never replace exact lookup or
silently introduce another external provider. Candidate limits bound work
per query, never the age of eligible history. Pagination/refined queries
must be able to reach candidates beyond the first bounded set.

A bounded recent referent descriptor can help bootstrap “make that shorter,”
but must be labelled as a recent candidate, not a declared dependency or
outstanding instruction. Specify and evaluate that bootstrap deliberately;
removing all recent referents also risks making ordinary follow-ups fail.
Do not retain lexical thread gates as a hidden second authority. Long-lived
work overlays need the same explicit dependency boundary.

### Context allocation

Cover each needed dependency compactly before enriching any one with a Full
record. Expand only the needed evidence/versions, not the entire thread.
If compact coverage does not fit, expose what could not be loaded rather
than silently answering from the easiest source. Preserve exact source ids,
dates, outcome provenance and replay. New recall results remain ordinary
logged current-turn evidence. Reuse the frozen selection unless a recorded
expansion/reset changes it; no invisible rewriting under cached reasoning.

Represent unknown capacity explicitly. Prefer discovered provider metadata;
otherwise the product needs an explicit configured provisional resource
envelope with honest provenance, or an admitted inability to establish one.
Do not print infinity as measured spare capacity or introduce model-id limits.
Measure complete wire costs, including schema stubs, current results and
output reserve. Request work should depend on selected records and bounded
index queries, not on N full projections.

### Answer extent and geometry

Choose the smallest complete answer to the current ask; add depth where the
request or evidence warrants it. Requested coverage and optional enrichment
are different. A summary, chart, details card and source panel should each
contribute information or interaction, rather than repeat one finding. This
belongs in one coherent prompt/tool contract, not contradictory advice at
several layers or a fixed maximum number of cards.

Resolve placement to a stable target/group id at acceptance. Forms expose
minimum/preferred reading width and overflow behavior; grouped blocks share
space when their minima fit, otherwise stack in semantic order. Unequal
widths are permitted. Tables/charts can use wider Canvas space through the
same rules; no independent masonry/column interpretation. Content extent,
form and viewport determine pixels; context tokens and history age do not.
Keep original answers immutable when a later request revises/reuses them.

This proposal changes invariant 36's automatic preceding-card policy and
several current interfaces. Update that invariant and docs 68/85/87/98
together if approved. With no user base, remove superseded runtime paths
rather than maintain two active policies or compatibility shims. Retain
append-only records and existing access/erasure boundaries.

## Acceptance before calling this solved

- Exact and paraphrased references at distances 1, 10, 1,000 and 10,000;
  an explicit old target must not be replaced by a more recent similar one.
- One request combines several distant sources; one source feeds several
  later requests; corrections and transforms preserve target identity.
- Long/short references, changing acts, mixed languages, similar subjects
  with different entities, topic switches and interruptions. Include ordinary
  fresh questions to measure unnecessary retrieval and stale-work carryover.
- Historical, current and comparison requests preserve their time scopes;
  failed/refused/aside/erased/branched material keeps its real status.
- Tool uncertainty: failed/empty/truncated/partial searches, unavailable
  indexes, bounded candidate overflow and genuinely ambiguous targets.
  Measure recall success, reformulation burden and clarification frequency.
- Grow N with unrelated turns while holding the required dependency set
  fixed. Measure planning time, bytes loaded, RSS, full wire tokens and
  retrieval correctness. Also grow a single dependency-rich turn.
- Measure answer completeness, duplication, reading burden and tool/schema
  overhead without enforcing a topic recipe or a universal card count.
- Live/settled/Canvas ordering and anchored groups at 1440 × 900 and
  390 × 844, both themes; unequal pairs, long tables, charts, prose boundaries
  and revisions. The desktop inspection here is not that qualification.

The two installed runs are baseline examples in this broader suite. No
weather/news branch, extra stop-word list, fixed history-distance threshold
or spot repair is a proposed solution.
