# Context selection and lifelong conversations

Status: in progress. Immediate defects repaired in the working branch;
live verification and the lifetime-scale design remain open.

## Evidence

On 2026-09-30, the same Noida weather request took 131 seconds in the
installed desktop conversation and 18 seconds in the development preview.
Both dispatched gpt-6-luna. Desktop steps carried approximately 152,000
input tokens and repeatedly received token-per-minute rate limits. Preview
steps carried roughly 16,000–18,000 tokens and did not retry. Receipts were
read through Core's trash-aware read-only session API.

After the repair was installed through `vak self install --force`, verified
with `vak self verify`, and the desktop restarted, the same conversation and
same prompt completed in 15.7 seconds (13:18:04–13:18:19 UTC). Its four
execute receipts succeeded on their first attempts, carrying approximately
8,121 / 8,594 / 8,986 / 9,114 prompt tokens including cache reads. The live
desktop displayed the result and no Offline state. This is one observed run,
not a general latency guarantee. The remote AWS process has not been inspected
or updated; identical build code alone does not prove identical live config,
history, provider account limits, or runtime behavior.

The desktop listener also omitted ConnectInfo, so query-token EventSource
authentication failed its loopback check. Ordinary bearer requests continued
to work while the app showed Offline. A regression exercises the actual
desktop boot path, including stream authentication and local health detail.

A 12-prompt desktop spot-check on the existing long conversation covered
simple arithmetic, a new science topic, two semantic follow-ups, a topic switch,
older-turn recall, an underspecified question, Hindi translation/pronunciation,
and an exact repeat. The direct answers and the chlorophyll/Mars follow-ups
were coherent; the repeat returned the same correct number. Two defects stood
out: “What is the capital?” guessed New Delhi instead of checking whether India
was the intended country, and “What temperature did you report in the latest
weather answer?” was treated as a request for live weather. Its initial prompt
carried 124,754 input tokens, then it searched and returned a *new* reading.
The latter run used a desktop process that had remained loaded across the
install, so it does not verify the resolver changes packaged in that build.
Several later test messages also arrived while that slow run was active and
were queued into its turn; their answers are useful behavior signals, not
independent latency samples. After explicitly terminating and relaunching the
desktop process, its PID changed.

After the final install, `target/debug/vak self verify` passed and the installed
desktop executable hash matched `target/debug/vak-desktop` exactly. The
restarted desktop then answered the same historical-weather prompt in the long
conversation in about 13 seconds (16:25:29–16:25:42 UTC). The persisted turn
card records `context: recall`; it returned the latest prior reading (25.5°C at
9:30 p.m. IST), with no live weather fetch. Three `openai-responses` /
`gpt-6-luna` dispatches settled on their first attempts (about 3.5s, 6.1s and
2.5s). The first two used 17,807 and 18,199 input tokens; the final targeted
recall used 109 new input tokens plus 18,196 cache-read tokens. There were no
rate-limit retries. This verifies the installed binary is active and the
historical-vs-current classification regression is repaired for this case.
It also shows the cap applies to automatically admitted full history, while
explicitly needed context can still be recalled on demand. This is one
successful regression run, not a broad latency benchmark or proof of million-
turn scaling.

A live `/health` comparison of the running preview server (8936) and installed
desktop server (64826) showed the same effective `openai-responses` provider,
`gpt-6-luna` model, FullAccess mode, 128,000-token context window, retry policy,
and healthy capability state with no warnings. Their value sources differed:
the preview inherited the shared global route, while the desktop workspace
(`~/vak-home`) selected its project route. The runtime route/model/mode therefore
do not explain the reproduced oversized-context slowdown. `vak self verify`
reported build/install parity and no config warnings. This check does not
establish what a remote AWS process is running; AWS was explicitly left out of
scope.

Finally, the installed desktop repeated the original fresh-weather request in
the same long conversation. It completed in 11.4 seconds (16:28:42–16:28:54
UTC), returning a current Noida reading. Four provider dispatches settled on
their first attempts; there were no TPM retries. The largest input was 7,506
new tokens plus 4,676 cache-read tokens. The card retained 394 Full tokens and
34 Card tokens. The prior 131-second run had sent 152,000 input tokens and
repeatedly hit TPM limits; this installed run is now close to the earlier
18-second preview observation. The controlled conclusion is that context
over-admission and rate-limit retries caused the large slowdown, not FullAccess
or a different provider/model. The two live latency samples are not a
statistical benchmark, and the preview process itself was not rebuilt or
restarted as part of this check.

## Required behavior

- A new topic must not inherit unrelated full evidence because space is free.
- Immediate follow-ups retain their referenced result and necessary evidence.
- A return to an older topic retrieves relevant work regardless of its age.
- Explicit turn, artifact, entity, and thread references outrank recency.
- Ambiguous references use compact candidate summaries and targeted recall;
  low confidence must not cause a wholesale history dump.
- Multi-topic requests union their independently selected dependencies.
- History remains append-only and recallable, subject to trash/access rules.
- Request size and request-path work must be independent of total history size.

## Immediate repair

Full-context admission requires relevance or a reference. Recency ranks
eligible context; it does not admit it. Generic intent labels, interrogatives,
temporal qualifiers, and substring coincidences are not topic evidence.
Unselected turns retain compact, recallable references. The planner measures
the actual rendered Full records, including tool arguments, and Card lines
with the current capacity profile instead of trusting old recorded costs.
Automatic Full history is now capped at 12,000 tokens per run even when the
model's remaining horizon is much larger; further detail is reopened on demand.
The intent kernel recognizes a prior-assistant-response reference as historical,
so “latest answer you reported” does not trigger current-world freshness.
The system prompt now tells the model to interpret continuity semantically,
use indexed matches as candidates, and ask a brief question when candidates
remain ambiguous. The resolver and working-set selection are still partly
lexical; semantic candidate reranking is not implemented yet.

Tests cover a fresh weather question after large market/document turns,
return to a matching topic 100 turns back, immediate anaphora, old-card cost
underestimates, no turn splitting, packet batching and reset boundaries.
This is deterministic lexical candidate selection with model-driven recall
available; it is not a proven semantic selector for every paraphrase or
ambiguous reference.
A short desktop comparison later found generic arithmetic/output words ("times",
"reply", "number") matching old report cards and promoting large tool histories.
They, generic instruction verbs, and response-shape words are now excluded as
topic evidence. The focused regression passes; the installed app still needs a
repeat run to verify the reduction in its actual provider request.

## Lifetime-scale algorithm and remaining work

The current ledger and TurnIndex paths still load/scan history. The cross-session
recall cache indexes only a trailing 4,000 message lines and invalidates on
append. Neither is sufficient proof of lifetime-scale operation.

The intended request path is:

1. Resolve request strands and reference hints using the existing intent kernel.
2. Retrieve bounded candidates from incremental derived indexes: explicit
   IDs/thread edges first, entity/topic postings next, then semantic candidates.
   Use a small hot working set; do not scan all ledger entries per request.
3. Rank candidates by reference strength, dependency, subject relevance and
   confidence. Use recency only as a tie-breaker among eligible candidates.
4. Admit only the evidence needed for each strand within the measured budget.
   Expand dependencies by typed links; keep a strict limit on expansion work.
5. When references remain ambiguous, supply compact candidate summaries for
   targeted recall or clarification. Do not guess the most recent topic.
6. Freeze the selected projection for the run, append receipts and new turns,
   and update derived indexes incrementally at settlement.

Indexes must be rebuildable accelerators over the canonical ledger, scoped to
the same agent/workspace and trash boundary. Do not introduce another durable
history or start a data-architecture milestone implicitly. A disk-backed index
or ledger access refactor needs an explicit design tied to the pending data
architecture, registry/path ownership, invalidation and recovery contracts.

Acceptance remains open for semantic paraphrases, ambiguous older references,
mixed strands, permission/trash invalidation, restart/index recovery and
1,000 / 100,000 / 1,000,000-turn scale measurements. Measure prompt tokens,
retrieval latency, peak memory and bytes read separately; a passing synthetic
ranking test is not proof that the live runtime handles a million-turn ledger.

## Permanent architecture: bounded context over addressable history

The user explicitly authorized this repair on 2026-09-30, including storage,
information architecture, text and voice references, and million-turn scale.
AWS operations are out of scope: validate the shared runtime on desktop.
No additional AWS inspection or deployment is required by this repair.
This is a context-engine replacement, not authorization to begin the pending
storage/encryption/lifecycle milestones. Reuse their locked ownership and
lifecycle contracts; do not create a competing catalog or schedule model.

### Units and ownership

A conversation is a durable history, not a prompt. A run is a bounded working
set assembled for one request. A turn may contain several strands with different
subjects, dependencies and freshness requirements. Turn numbers are display
positions; UUIDs are durable addresses. A reference identifies agent,
conversation/session, turn, optional strand and artifact version. It must survive
pagination, session rotation and a changed display order.

The canonical append-only records own facts, provenance, explicit references,
resolved intent, outputs and corrections. Derived indexes accelerate lookup.
The active run owns a small mutable working set. Global safety policy and live
permission checks remain outside historical retrieval; an old approval cannot
re-authorize a present effect.

### Turn metadata, not one loose tag

Reuse existing Intent strands, TurnCard and presentation provenance. Add only
missing typed fields to their canonical records, then project them into the
existing search store. A search descriptor contains:

- Stable scope/turn/strand identifiers, ledger sequence and bounded locator.
- Exact user subject, short outcome, entities with names/aliases, artifact and
  version IDs, topic terms, and language. Entities need scope and type; two
  people called Alex or two files named budget must remain distinguishable.
- Thread membership and typed edges: continues, refers-to, depends-on,
  corrects, supersedes, produced-artifact, and derived-from.
- Requested time interval, evidence observation time, source time, validity
  interval if supplied, and temporal intent: current, historical, comparison,
  refresh, or unspecified. Unknown validity stays unknown.
- Origin and trust: human directive, host observation, retrieved document,
  model inference. Extraction confidence and extractor version are separate
  from retrieval score. A model-created label is not a user instruction.
- Lifecycle and scope filter keys; content remains subject to canonical access,
  trash, sharing, and later erasure rules.

Tags are hints, not exclusive buckets. A turn can mention several entities and
belong to several strands. Summaries never replace the evidence locator. Store
embeddings only as optional, versioned derived projections; lexical/exact
lookup must still work when an embedding provider is absent or down. Never
send history to a new external embedding service without configured authority.

### Request algorithm

1. **Interpret each strand.** Extract explicit references first, then temporal
   intent and subject. Typed UI references bypass fuzzy identity resolution.
   Resolve voice transcripts using the same path; the speech layer does not
   guess hidden references or grant authority.
2. **Obtain bounded candidates.** Exact scoped ID/version lookup first; then
   thread and entity postings, text ranking, and optional semantic retrieval.
   Retrieve separate small candidate lists for each strand, with a global cap.
   No request-time walk over all turns, all files, or all agents. Search work
   has a deadline and expansion budget even when a term is very common.
3. **Assess the relationship.** Distinguish exact reference, strong continuation,
   useful background, weak candidate and unrelated. Raw lexical/vector scores
   are not probabilities; do not normalize the best of a bad set to certainty.
   Use coverage, entity agreement, conflicting entities/time, source quality,
   top-result margin and explicit edges. Calibrate confidence using labelled
   scenario tests before adopting numeric thresholds.
4. **Resolve ambiguity.** A new independent question proceeds without history.
   An ambiguous continuation gets compact candidates, targeted recall, or a
   short clarification. Multiple named subjects get independent selections.
   If the user names older work, recency cannot silently redirect to newer work.
5. **Check freshness separately from relevance.** A relevant old weather answer
   can supply the place or units, not today's weather. Historical questions
   retain historical evidence; refresh requests fetch fresh evidence; comparisons
   require both. Current news, prices, schedules, availability, software state,
   service health and mutable workspace contents use current evidence. Source
   timestamps and the request's locale/time zone determine the requested interval.
   An unsuccessful refresh reports failure/staleness rather than recycling an
   old result as current. Do not invent universal TTLs for arbitrary facts.
6. **Expand dependencies narrowly.** Follow necessary referenced artifact
   versions, decisions and evidence with visited-set cycle detection and bounded
   hop/count/byte limits. Thread membership alone cannot pull an entire thread.
   Current file state must be checked when an old artifact version is selected.
7. **Assemble within measured limits.** Stable policy/capability prefix, selected
   source-labelled evidence, current directive and current-turn steps. Include
   compact candidate references where necessary. Unselected history stays on
   disk and searchable; do not summarize unrelated history to fill the prompt.
   Enforce both request token limits and retrieval byte limits. Exact costs
   include tool arguments, tool results and provider blocks.
8. **Record the projection.** Model-visible selection is reproducible from
   canonical ledger entries and a logged selection manifest: source IDs, fidelity,
   policy/index/extractor versions, freshness decisions and cost. Log metadata,
   never conversation content, in operational telemetry. Record later recall tool
   results as current-turn messages as usual.
9. **Settle incrementally.** Append new canonical records, update affected index
   rows and outgoing edges, publish the new watermark. Avoid a model side-call
   solely because another unrelated turn was added. Resume/retry keeps the run's
   selected source identities unless an explicit resync is recorded.

### Storage and index execution

Reuse the declared cache search database; augment it with scoped turn/strand
rows, edges, term/entity postings and ledger locators. The existing full-file
import and trailing-4,000-line recall cache are replaced, not stacked beneath
another index. The session handle must stop materializing every historical tool
payload in RAM. Read the open run and selected bounded ranges lazily through the
same trash-aware session boundary. Append offsets, parent links, reset/branch
membership and hash-chain state must be indexed so a selected turn can be
reconstructed without traversing the whole chain.

Ingestion consumes only completed appended records after a durable watermark;
its transaction updates projections and watermark together. A partial final line
is retried when complete. Scope IDs and file identity prevent a replaced or
truncated ledger from borrowing an old cursor. Rebuild is background, resumable
and generation-based; queries see a consistent generation. Index loss must
never force a million-turn synchronous rebuild on the request path: report
warming/search-unavailable, keep the active run usable, and rebuild asynchronously.
A missing index is not permission to return unrelated history. A post-query
canonical access/lifecycle check protects against revocation/trash races.

Use bounded caches of descriptors/locators, not an unbounded cache of full turns.
Large tool results retain their canonical body and bounded evidence windows.
Concurrent desktop/server/CLI ingestion needs idempotent scoped keys, WAL/busy
handling and watermark arbitration. A file append and a database transaction
cannot be atomic together: ledger-first plus idempotent replay is the recovery
contract. Detect stale or corrupt index generations; verify requested canonical
records and chain links instead of trusting cached bytes as authority.

### Text, navigation and voice experience

- An older assistant turn has a small **Continue from here** action next to its
  existing actions. The left turn-rail preview has **Go to turn** and **Continue
  from here** actions. Keyboard and touch can reach these; hover is optional.
- Continuing adds a removable composer reference chip with a human-readable
  subject/date preview. Sending appends a new turn with a typed scoped reference;
  it does not rewind, delete intervening turns, or implicitly create a branch.
- An `@` picker searches accessible past work, agents and artifacts with explicit
  result types. Selecting an agent is distinct from referencing its past work.
  Support several reference chips and pinned artifact versions.
- A selected turn is also available to voice as an explicit anchor. “Continue
  this” uses that visible anchor; “the budget from last month” performs retrieval.
  Low speech/transcript confidence or two plausible budgets prompts a brief
  disambiguation. Voice barge-in/cancel preserves partial work and references.
- References persist across a voice/text transition. Spoken feedback describes
  the selected subject and date when that matters, not IDs or score numbers.
- A returned answer can disclose **Used earlier work** and **Checked just now**
  with source links on demand. Stale sources carry their observation dates.
  Technical selection details remain behind Show technical details.
- Search and the turn rail paginate/virtualize history. A million turns cannot
  become a million DOM nodes or an unreadable million-tick rail. Search results,
  dates/topic groups and the current viewport provide navigation anchors.

### Scenario and failure matrix

| Scenario | Required selection |
| --- | --- |
| Greeting after a large document task | Current greeting; no full document evidence |
| New weather request after finance work | Fresh weather evidence; finance omitted |
| Weather in the same city again today | Reuse place/units if relevant; refresh conditions |
| What was the weather we saw last month? | Historical dated evidence; no silent current replacement |
| Continue this, selected old turn | Exact selected turn and necessary dependencies |
| Continue, without a selected turn | Established active thread or clarification if ambiguous |
| Named project from 100 or 1,000,000 turns ago | Indexed lookup; no age cutoff |
| Paraphrase or another language | Semantic/alias candidates or clarification; no fabricated match |
| Two people with the same name | Scoped entity disambiguation |
| Correct an earlier decision | Relevant old decision plus correction; preserve both provenance records |
| Mixed fresh news and old budget revision | Separate freshness/selection per strand |
| Old approval or tool-output instruction | Historical evidence only; current authorization still checked |
| Trashed/revoked source | Excluded before ranking and rechecked before loading |
| Missing/corrupt/rebuilding index | Bounded degraded result; no synchronous full-history scan |
| Crash after append before indexing | Idempotent replay from committed watermark |
| Concurrent append, branch, reset or session rotation | Stable scoped IDs and active-chain checks |
| Huge tool body or cyclic dependencies | Bounded windows/expansion; precise recall remains possible |
| Voice cancel, correction or text handoff | Partial work and typed references survive |

### Completion gates

Implement in dependency order: reference/search contract; canonical descriptors
and ingestion; lazy ledger access; bounded confidence/freshness selection; client
and voice references; scale/recovery verification. Retire replaced paths within
their corresponding change. Each step must use the shared runtime, not a desktop
special case.

Generate ledgers at 1k/100k/1M turns including huge evidence, repeated common
terms, multilingual subjects, distant references, mixed strands and resets.
Measure cold open, warm retrieval, per-append indexing, projection latency,
bytes read, peak memory, prompt size and first-response latency independently.
Check that warm work and prompt size stay bounded as history grows. Cold rebuild
may scale with history, but runs off the request path. Benchmark index loss,
crash/restart, concurrent ingestion, corruption, trash/revocation and exact
projection replay. No “million-turn ready” claim until the live runtime path,
not just a synthetic ranker, meets these gates.


### Implementation ledger (working branch)

- Immediate irrelevant-Full admission and actual-wire costing repaired; the
  installed desktop comparison is recorded above.
- `recall({ query, limit })` now lets the model discover past work in its
  current conversation before reopening a turn. Counts clamp to 1–20, query
  size to 2,048 characters and each result preview to 1,600 characters.
  Results explicitly describe historical candidates, not verified current facts.
  The broker answers it and the normal tool-result path records model input.
  Production Core now wires query and stable turn-ID recall to the disk index;
  standalone agent fixtures without a host retain their in-memory resolver.
  Indexed queries carry bounded candidates, timestamps and a limit-reached
  indicator, distinguishing a bounded pass from an exhaustive absence proof.
- TurnCard construction now measures actual text/call/result/provider blocks
  and the rendered card, avoiding the old text-only/full and search-text/card
  under/overcounts. Planner still remeasures old cards for the current profile.
- Admission Intent entries written before a new directive now tag that new
  turn, rather than overwriting the settled preceding turn's descriptor.
- The existing `store.db` importer now uses a transactional per-ledger cursor
  and bounded first/tail anchors. Warm imports read only the appended extent
  plus constant-size validation; no full-file string allocation. Partial final
  records wait for completion, errors roll back rows and cursor together,
  and replacement/truncation invalidates stale search rows. `ImportStats`
  exposes bytes read for verification. Cursor state stays in the existing
  rebuildable cache file; no second durable history was added.
- Settled TurnCards now contribute subject/outcome text to the existing FTS
  store. Tool argument bodies are not part of that compact subject projection.
- Tests verify query exclusivity/bounds, broker recall and ledger replay,
  intent association, restart/append cursor behavior, partial records,
  replacement and rollback. A large-record fixture verifies warm read work
  stays under 16 KiB despite a much larger committed prefix.

- Every indexed canonical entry now has a byte locator, including opaque
  evidence that is intentionally absent from full-text search. Core can load
  an exact record through canonical workspace, agent/audience and trash checks,
  verifying its ID and digest. Missing/stale locators fail explicitly, with no
  full-history fallback. The runtime planner still needs to use these reads.
- The code-owned prompt now explains selected context and model-directed
  recall, verification/reranking, fresh independent requests, historical dates,
  missing-versus-unavailable history and clarification. This remains under
  the existing seed budget and applies when operating rules are customized.
- Voice inspection confirmed transcription starts the server-side turn.
  Continuation anchors must therefore travel in the authenticated utterance
  protocol before dispatch; a composer-only chip is insufficient. This path
  is not implemented yet.

- Index ingestion also projects compact closing-record addresses and binary
  ancestor links. Candidate membership is checked against the selected leaf
  with logarithmic jumps, excluding sibling/abandoned branches without a
  full-chain traversal. Derived rows and the watermark commit together.
- Query/stable-ID recall is wired through Core to the agent broker. Search
  has a 250 ms SQLite progress deadline, cancellation, at most 64 candidate
  records and 20 returned hits. Reopening one turn follows only its canonical
  parent range, capped at 512 entries, 8 MiB and 250 ms. Its normal tool
  result remains logged in the current turn. Cold/stale/oversized indexing
  explicitly reports history unavailable; no request-path full rebuild.
- Core schedules background ingestion at run preparation and settlement.
  A warm lookup may import only a bounded append (2 MiB maximum), with
  anchor validation. Missing/replaced cursors are deferred to background work.
  Warm store opening no longer takes a schema write transaction every time.
- Tests cover distant ancestor and sibling paths, selected turn reconstruction,
  cancellation, trash, bounded warm/cold ingestion, cache-version replay and
  opaque evidence locators. These are retrieval-path proofs; SessionLog opening
  and ordinary planning still materialize all historical entries.

- Reopened turn results now preserve structured message blocks, including tool
  arguments/results; the earlier plain-text renderer silently lost those blocks.
  Results identify historical scope and retain the current-authority/freshness
  caveat. Search result prose/timestamps are regenerated from bounded,
  digest-verified canonical closing records, with an 8 MiB verification ceiling.
- Settlement builds only the active turn and inspects only the latest message,
  replacing two full TurnIndex passes and a cloned full message chain. Open-turn
  reserve sizing uses the same active-turn range and honors handoff resets.
  Newest-state queries can use a reverse active-chain iterator without allocating
  the whole chain. Tests compare these projections with the full historical
  implementation through open/closed/card/branch/reset states.

- Background ingestion now commits replay in bounded chunks (4 MiB target,
  100 ms between-record transaction budget; one complete record may exceed the
  target but remains under the 64 MiB record cap). It publishes a completed
  byte watermark with each chunk. An incomplete final record stops replay until
  another trigger; it does not spin. Tests replay 1,000 chained entries with a
  handle restart after every chunk, read through a separate observer between
  transactions, and verify later corruption rolls back only its current chunk.
- Disk and in-memory topic lookup share subject-term filtering so question and
  temporal words cannot match every previous topic. Unicode word extraction
  preserves combining marks instead of silently dropping non-English subjects.
  Hindi, Chinese and accented Latin lookup have regression coverage. This is
  lexical support, not a claim of semantic paraphrase or language segmentation.

Still required: lazy indexed session/turn loading; replacing the trailing recall
cache; scoped descriptor/edge queries wired into production planning; calibrated
confidence and freshness selection; client/voice typed references; UI
scale/responsiveness verification; and live million-turn measurements. The
million-turn implementation is not complete or deployed.

A first versioned `ContextSelection` ledger record now captures the exact plan
and pre-request leaf, and checked replay rejects missing, mismatched, duplicate,
or packet-shaped source sets. Stable turn IDs are used in rendered cards, with
token costs measured from that actual rendering. Planner and ledger primitives
are covered, but the live agent request path does not yet append/replay these
records or use the indexed candidate list. Do not treat this as production
activation.

## Local desktop/Web lifecycle investigation — 2026-09-30

- An earlier comparison server at `127.0.0.1:8946` used a throwaway workspace
  and was stopped after discovering the mismatch; it is not a valid
  desktop-vs-installed-Web comparison.
- The installed local Web server at `127.0.0.1:8901` is the one managed
  gateway. The rebuilt Desktop attaches to it; Web and Desktop-created session
  summaries are served by that process and all report `~/vak-home`. The
  session ledger files are under the same agent home,
  `~/Library/Application Support/vak/agents/vak/sessions/`. Health/config
  checks show `openai-responses` / `gpt-6-luna` / `FullAccess` on the gateway.
- On one freshly-created local session, a Desktop prompt and exact response
  appeared in the already-open Web tab; a Web follow-up and its answer then
  appeared in Desktop. Closing and reopening Web restored both turns. Both
  clients were explicitly pointed at the same session ID for this test.
- A real Desktop quit/relaunch exposed a separate resume defect: the new local
  session's conversation id is `agent:vak:local:<uuid>`, while startup only
  recognized the older unsuffixed `agent:vak:local` id. Desktop therefore
  returned to the older `hi` session even though the newer session was at the
  top of the gateway's session list. The local-conversation predicate now
  recognizes the exact base id and its UUID-suffixed local sessions. The
  rebuilt installed app was cold-started and verified against the same session
  in Web.
- The exact-prompt test on the user's long historical session produced a
  completely unrelated equity-market report. That is a separate confirmed
  context-selection defect; shared runtime parity does not resolve it.
- The Desktop fallback still starts an embedded server if no managed gateway
  is running. In that mode durable fire-and-forget across app exit is not
  promised. The parity test here used the managed gateway, which stays alive
  after both clients close.

## Installed binary and workspace verification — 2026-09-30

- Process inventory shows one managed listener, PID 93392, at
  `127.0.0.1:8901`, launched from
  `/Applications/Vakyartha.app/Contents/MacOS/vak serve --gateway --trust
  --port 8901`, and the Desktop tray process PID 93456. No second `vak serve`
  listener was running for the Web comparison.
- Installed and workspace `vak` hashes match
  (`7fb57ad1be3ffed2e3a66cda25c13d1bbad5b2d32a31ce4078b4fb6ec7d1b056`);
  installed and workspace `vak-desktop` hashes match
  (`d16496a8519d3e5a8a68ef2ef88ef70e46b0f6201a84a83dbb088c051bb93f39`).
  Both report `5.3.0 (0ef131b5)`, and `vak self verify` passes.
- `/health` on the one gateway reports cwd `~/vak-home`, sessions home
  `~/Library/Application Support/vak/agents/vak`, provider/model sources as
  `project_config`, `openai-responses` / `gpt-6-luna`, FullAccess, healthy
  status, and no warnings. Running `doctor` from the actual workspace also
  passes; its untrusted-workspace note means local project permission and
  capability settings are ignored by that standalone invocation. It is not a
  second server or a different installed build.
- After rebuilding and restarting Desktop, the local conversation suffix fix
  cold-started the same session in Desktop and Web. Both surfaces showed the
  same two prompt/answer pairs. Closing/reopening Web retained them. The
  earlier live cross-surface send and receive checks also passed.
- Targeted tests passed: 13 context planner tests, 10 working-set projection
  tests, 6 agent recall/card tests, and 4 work-receipt tests. `git diff
  --check` is clean. These prove bounded projection/recall cases, not
  million-turn request-path scaling.

## Installed same-session latency regression — 2026-09-30

The slow/absurd result reproduced on the installed build, then was retested
after the fix against the same managed gateway (`127.0.0.1:8901`), session
`01a0f344-38b0-7941-824d-f81f3b270031`, route, workspace, and settings. Desktop
and Web remained attached to that one gateway; process inventory showed one
gateway (`/Applications/Vakyartha.app/Contents/MacOS/vak serve --gateway
--trust --port 8901`) and one Desktop tray process. Current installed `vak`
and `vak-desktop` hashes match their `target/debug` build outputs byte for
byte; `/health` reports `~/vak-home`, `openai-responses`, `gpt-6-luna`,
FullAccess, and no warnings.

For the exact prompt `Reply with exactly latency-check and no other text.` the
pre-fix run took about 25 seconds, made eight provider dispatches and several
tool calls, and returned an unrelated HTTP-latency metric. The resolver split
the hyphenated literal, read `check` as a verification act, and then let the
previous turn's `verify` act bias a fresh request; the stop guard consequently
kept asking for tool work after the model had the exact answer. Preserving
hyphenated compounds alone did not remove the stale-history vote.

The permanent resolver fix preserves internal hyphens and only lets the
previous act resolve a missing current act when the new directive has an
explicit continuity marker. Resolver version is now 8. On the same installed
session and exact prompt after rebuild/service sync, the result settled in
1.66 seconds with exactly `latency-check`, one provider dispatch (1.60s), no
tools, and no stop-guard intervention. The visible Desktop and Web histories
both showed that exact result. `cargo test -p vak-intent --lib` passed all 138
tests; `cargo test -p vak-agent --lib stop_policy::tests` passed all 16; and
`vak self verify` passed for the installed app.

This is a confirmed runtime cause and a same-install before/after measurement,
not proof that every latency issue is resolved. The million-turn context
selection design remains partly unconnected to the live request path, and
million-turn performance has not been measured.

## Current-weather tool-discovery replay — 2026-09-30

On the same installed gateway, workspace, route, settings, and shared session,
`What is the current weather in Noida? Give temperature and condition only.`
resolved as a fresh live-data answer, received current Open-Meteo evidence, and
returned the correct current card. It nevertheless took about 28 seconds and
eight sequential model dispatches. Five early dispatches were spent on tool
discovery; three successive `find_tools` calls returned no new useful
capability, while the configured install had no web-search tool. This explains
why FullAccess did not make the request fast: permission to access a capability
does not install a missing search integration, and the system prompt made the
model repeatedly look for one instead of using a known direct API.

The tool-discovery result now reports how many matches are new within the
current turn and explicitly says when rewording did not expand the available
set. The system prompt instructs the model to search once, stop repeating
equivalent queries, use a known authoritative direct API when appropriate, and
state the limitation when no suitable source path exists. After rebuilding,
installing, bouncing the one managed gateway and Desktop, and confirming the
installed hashes and health values again, the same exact request was replayed
in the same session. It settled in about 17 seconds across four dispatches
(input-token counts 6,625 / 100 / 260 / 134), made no `find_tools` call, fetched
one current Open-Meteo URL, and returned `25.6 °C, Clear`. The first answer
reused the prior reading and was correctly rejected by the freshness gate;
the final answer followed a successful current fetch. This is close to the
earlier 18-second development-preview weather run and materially better than
the 28-second pre-fix installed replay, though it remains a single comparison,
not a latency distribution. The `find_tools` unit tests, all intent tests,
`vak self verify`, installed hash parity, and shared-session rendering pass.

The post-fix process inventory was exactly one gateway (PID 11867, installed
`/Applications/Vakyartha.app/Contents/MacOS/vak`) and one Desktop (PID 11910,
installed `vak-desktop`), with the browser still on that gateway's port 8901.
`/health` reported `/Users/nisheethranjan/vak-home`, `openai-responses`,
`gpt-6-luna`, FullAccess, and no warnings. Installed/workspace SHA-256 values
matched: `vak` `47ee1f19789a398ab36d4d2feba6b9f455743e853ab2adc748082307422a7df3`,
`vak-desktop`
`732b711b6a19b36272f39fe89b94a743ed5a11037a98239e2ba9c94946446d82`.
Desktop and Web both rendered the final result in session
`01a0f344-38b0-7941-824d-f81f3b270031`.

## Installed long-history request-path probe — 2026-09-30

Rechecked the actual live topology before testing: exactly one listener on
`127.0.0.1:8901` (PID 11867, installed app-bundle `vak`) and one Desktop
process (PID 11910, installed app-bundle `vak-desktop`); no preview server was
running. The installed executable hashes still match this checkout's
`target/debug` outputs byte-for-byte. `/health` reports the shared workspace
`/Users/nisheethranjan/vak-home`, sessions home
`/Users/nisheethranjan/Library/Application Support/vak/agents/vak`,
`openai-responses` / `gpt-6-luna`, FullAccess, healthy status and no warnings.

In the existing 38-turn conversation `01a0ee5d-5e56-73f3-bc64-63f9937be005`,
sent `Reply with exactly: parity-probe-731` through the installed Web UI. It
returned exactly `parity-probe-731`; from pressing Enter to the answer becoming
visible took 16.8 seconds. The installed Desktop rendered the same new prompt
and answer in that same session immediately afterward. This verifies a
long-history request on one shared installed runtime and that unrelated prior
turns did not change the literal response. It is a UI-observed end-to-end time;
provider receipt timings/token counts were not yet correlated when first
recorded, so the initial 16.8-second observation included UI-state polling and
is not a valid request latency. Do not use that number for comparison.

A second probe used a timer that began at Enter and polled both the rendered
answer and the authenticated `/sessions/{id}/receipts` endpoint. Prompt
`Reply with the single word: lattice-982` completed in 3.48 seconds end to
end; the single winning provider dispatch took 2.259 seconds and reported
6,491 input tokens, 18 output tokens, and 4,347 cache-read input tokens. It
used `openai-responses` / `gpt-6-luna`, made no retries, and returned exactly
`lattice-982`. The answer was confirmed in the Desktop view of the same
40-turn session. This is now a properly correlated long-history datapoint;
more repetitions and a matched development-preview sample are needed before
claiming a stable comparative latency distribution.

An immediate repeat in the same session used
`Reply with the single word: cobalt-743`. It completed in 3.12 seconds end to
end; one provider dispatch took 2.071 seconds and reported 3,137 uncached
input tokens, 4,347 cache-read input tokens, and 19 output tokens. It returned
the exact answer, and the Desktop view showed the same turn. The two
correlated probes are consistent: one dispatch, no retry/tool loop, and a
roughly 2-second provider response despite the long conversation.

## Research-to-document and live surface parity probe — 2026-09-30

Used the existing long conversation on the installed home, not a second
gateway. The first Web-submitted research-to-document turn remained active for
over five minutes while Desktop continued to show an older weather turn as
working. The Web UI and Desktop therefore disagreed during the run. Stopping
the active run changed its state to `Stopped` in both views; the original
research request did not produce a document. A transcript read during the run
returned HTTP 409 (`run in progress`), and the receipts route returned 404
while the session was active. Its handler returns 404 when the live session
log is temporarily absent; after the run settled, it returned 205 receipts.
Installed `vak` and `vak-desktop` hashes still matched `target/debug`
byte-for-byte, so this was an active-run observability gap, not evidence of a
different installed build.

Resubmitted once from Desktop after the session became idle. Web immediately
showed the same prompt and `Working` state for session
`01a0ee5d-5e56-73f3-bc64-63f9937be005`; both surfaces then showed the same
completed answer and the same `Long_Duration_Energy_Storage_Brief.docx` draft.
The result arrived within about 75 seconds. The Desktop document preview
contained 526 words, 17 paragraphs, six headings and five source links, with
separate sections for flow batteries, compressed-air storage, thermal storage,
comparison/uncertainty and sources. The document is a reviewable draft, not an
accepted file. Sandia's DOE Energy Storage Handbook page was confirmed to
cover flow batteries, CAES and thermal storage; the external browser tool could
not fetch the four OSTI records, so those links remain unverified here.

The second run confirms that a research/document task can finish quickly in
the installed long-history session and that Desktop/Web eventually converge.
It also exposes two unresolved defects: a stale in-progress activity remained
visible on Desktop while Web was working, and the completed document was
labelled `Partial result` with “The requested outcome is not verified” even
though both clients displayed a generated draft that opened in the document
preview. Diagnose event/state reconciliation and outcome evidence separately;
do not treat the artifact card alone as verification of source correctness.

The completed prompt produced ten model dispatches and ten new receipts: nine
settled directly and one had a failed attempt followed by a successful one.
Winning-attempt latency summed to about 76.8 seconds; the failed attempt adds
about 5.7 seconds. The first dispatch carried about 27.1k input plus cached
tokens, rising to about 59.0k on the last. Tool calls included two `recall`
searches with empty queries, two `find_tools` searches, six `webfetch` calls,
and `office_apply`. The two empty recall calls are wasted dispatches even
though the parser rejects empty search text.
