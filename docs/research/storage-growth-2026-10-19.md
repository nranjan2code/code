# Storage growth and long-running conversations

Status: **historical** — code-led study on 2026-10-19, with a synthetic
Blueprint Storage simulator. Forecasts are not install measurements or
benchmarks. No runtime storage, retention, security or data-format change.

## Conclusion

Keep the authoritative append-only record, bound the work needed to use it,
and reclaim only what the existing lifecycle rules permit. A growing data
home is expected; a growing amount of history decoded on every turn is a
separate performance problem. Model context, physical disk, memory, request
traffic and recovery time need separate budgets.

The Blueprint's new **Storage** tab is a decision aid. It projects an empty
home over 1–120 thirty-day months. Every workload and physical-size value is
an editable assumption. Repository facts and shipped default retention days
are read at build time, linked to source lines, and produce `needs review`
if their anchors disappear. Nothing inventories the real data home or
applies the install's effective label.

## What the code establishes

| Layer | Growth and consequence | Source |
|---|---|---|
| Conversation ledger | Immutable record; compaction adds a summary, not deletion. Cold scan and loaded entries can still grow even when the request working set is bounded. | `crates/vak-session/src/log.rs` (`scan`, `derive_messages`), `crates/vak-session/src/types.rs` (`CompactionEntry`), [context engine](../design/68-context-engine.md) |
| Objects | Large content is referenced through tenant objects. Logical references are not additional physical copies; reachable content remains retained. Shared references and scopes constrain collection. | `crates/vak-session/src/objects.rs`, `crates/vak-session/src/documents.rs` |
| Sealed segments | Side chains rotate at 256 KiB plus a crossing frame. Encrypted frames are copied, not compressed. Seal manifests and verification work still grow. | `crates/vak-session/src/chain.rs`, `crates/vak-storage/src/segments.rs` |
| Prompts | Repeated prefix and carried history consume request traffic per model step. They are not proof of a new stored copy of every request body. Model-visible contributions must remain reconstructable. | [prompt layers](../design/45-prompt-layers.md), [context engine](../design/68-context-engine.md), invariant 1 |
| Cards | Text, arguments, presentation/state updates and any saved page versions can contribute bytes. Replacing a displayed card does not authorize deleting its historical records. A card may appear in more than one record representation. | `crates/vak-session/src/types.rs`, [file state](../design/90-file-state.md), [Library](../design/82-library.md) |
| Artifacts | Each version is immutable. Many large versions may outweigh conversation text. Count new physical bytes, not the full logical size of each reused object. | `crates/vak-core/src/artifacts.rs`, [Library](../design/82-library.md) |
| Lifecycle | Conversation records, their objects and run records have no default expiry. Defaults include environments/executions 7 days, telemetry 14 days plus 200 MiB cap, drafts 60 days to trash, trash 30 days, Document history 90 days. Guards and references matter. | `crates/vak-lifecycle/src/lib.rs`, `crates/vak-core/src/lifecycle.rs` |
| Projections | Effects and FinOps now use derived SQLCipher indexes. Bounded row decoding does not eliminate cold replay, seal-manifest or source-verification costs. Other consumers need measurement. | [data architecture qualifications](../design/73-data-architecture-and-lifecycle.md), [effects measurements](../audits/effects-projection-measurement-2026-10-19.md) |
| Backup/sync | Backup scope comes from the state registry. Derived databases are excluded; recovery rebuilds them. A folder remote is implemented; a cloud remote is not. Do not assume replicated bytes disappear immediately after local erasure or expiry. | `crates/vak-core/src/backup.rs`, `crates/vak-core/src/sync.rs`, [data architecture](../design/73-data-architecture-and-lifecycle.md) |

Crypto-shredding removes readability, not necessarily ciphertext bytes.
Logical expiry, physical collection and backup retention are different
clocks. A kept draft, live execution or hold can invalidate a naive
“daily bytes × retention days” plateau.

## Model and units

The pure forecast lives in `blueprint/harness/storage.js`; Python supplies
repository evidence from `blueprint/harness/storage.py`. The same function
runs in the browser and in deterministic scenario checks. JSON export
includes assumptions, repository facts and monthly results.

For day d in month m, turns = rounded(initial turns/day × (1 + growth)^m).
Permanent bytes = total turns × (ledger metadata + new text/tool objects +
cards + auxiliary records + artifact-producing fraction × versions × mean
new bytes/version). Fields are disjoint incremental **physical** byte
budgets, including their own framing/encryption overhead; they are not
serialization measurements. If card arguments already count in text/tool
objects, subtract them from that field before entering the card budget.

Temporary and telemetry bytes are daily cohorts. Past each assumed
retention age, the protected fraction stays. Index bytes are a chosen
percentage of ledger/card/auxiliary metadata. Backup equivalents multiply
permanent bytes plus modeled telemetry, excluding derived indexes and
scratch. This is capacity planning across locations, not the actual backup
manifest or an incremental backup algorithm. Primary headroom excludes
backup locations and starts with the entered disk allocation and free-space
reserve. Put copies on the same disk and their allocation must be budgeted
there separately.

All permanent classes stay in both curves. There is no forecast credit for
hypothetical compression, cross-scope deduplication, artifact expiry,
conversation erasure or future cold tiering. Temporary expiry assumes timely
collection; real segment boundaries, the newest sealed segment, trash,
shared references and active work can retain bytes longer. Telemetry's
shipped size cap is omitted conservatively. Checkpoint stores, pre-existing
data, WAL peaks, file allocation overhead and unrelated workspaces are not
modeled. A deployment estimate must add those reserves.

For T turns in one conversation adding h history tokens each, full-history
input is approximately prefix × T + h × T(T−1)/2, multiplied by requests per
turn. A bounded working set caps the history term per request, making it
approximately linear once full. The simulator repeats this calculation
within the chosen conversation length; disk retention is unchanged by that
choice. Prefix/schema tokens are included each time. It excludes intra-turn
history changes, output/reasoning tokens and provider cache billing, so it
is a traffic comparison rather than a bill. A prefix exceeding the context
budget is flagged as infeasible.

Cold reopen is sequential session bytes / chosen read rate + entries /
chosen decode rate. Projection scan and restore copy use separate assumed
throughputs. None predicts end-to-end latency: verification, key recovery,
object hydration, index construction, fsyncs, competing I/O and UI work add
cost. Use measured recovery drills before claiming a recovery objective.

## Illustrative scenarios, not measured usage

With 24 KiB ledger metadata, 100 KiB text/tool objects, 12 KiB cards and
8 KiB auxiliary bytes per turn; 10% of turns producing three 2 MiB versions;
1 MiB temporary bytes per turn kept seven days; 8 KiB telemetry per turn
kept fourteen days; 25% metadata index overhead; no protected temporary
bytes, no growth, and two full backup equivalents:

| Turns/day | Turns / 360 days | Primary | Across primary + two backup equivalents |
|---|---:|---:|---:|
| 50 | 18,000 | 13.55 GiB | 39.60 GiB |
| 500 | 180,000 | 135.55 GiB | 396.03 GiB |
| 5,000 | 1,800,000 | 1,355.48 GiB | 3,960.30 GiB |

Artifact versions dominate these chosen assumptions. Another workload may
be dominated by tool payloads, screenshots, PDFs, intake or checkpoints.
The table establishes arithmetic, not a recommended disk purchase.

## Recommended decisions, in order

1. **Measure before changing the substrate.** Add content-free physical byte,
   entry/object/reference/segment counts and growth deltas per registered
   class through existing administration/telemetry boundaries. Include
   saved/held/live bytes, catalog/index/WAL size, backlog and reclaimable
   bytes from the effective lifecycle preview. Do not scan private files
   through a second reader or log filenames, prompts or titles. An install
   measurement adapter is future work, not part of this simulator.
2. **Budget the active path separately.** Benchmark 1k/10k/100k-turn sessions
   and 1k/10k/100k/1M-row chains in isolated homes. Measure reopen, first
   response preparation, append, verification, projection catch-up, catalog
   search and peak RSS. Use p50/p95/p99 under concurrent writers/readers;
   capacity forecasts cannot prove these timings. Avoid replay on turn paths;
   use the existing verified projection/rollup contracts. Paginate history
   at the surface rather than rendering every card at once.
3. **Use the existing lifecycle first.** Preview temporary cleanup under the
   actual install label, account for holds and kept versions, and observe
   actual released bytes. Never rewrite a conversation or trim rows to make
   a size graph flat. Saved versions need an explicit product/policy decision
   before any different expiry behavior can be designed.
4. **Investigate amplification from evidence.** Count unique stored object
   bytes versus logical version sizes, duplicated card/argument payloads,
   Document revisions, projection indexes, WAL and manifests. Prefer an
   incremental reader/projection fix when decoding dominates. A format change
   for compression before encryption needs a separate security and migration
   design; encrypting then compressing offers no useful ledger discount.
5. **Plan recovery and copies.** Size primary and backup locations separately,
   reserve temporary publication/index-rebuild space, and run a supported
   backup/restore drill in a disposable home with the test binary's fencing
   boundary. Verify recorded erasures are reapplied and projections rebuilt.
   Include backup age and maximum acceptable data loss as an explicit
   operational decision; this forecast models capacity, not backup scheduling.
6. **Consider cold storage only after measured pressure.** A read-through
   archive would need catalog lookup, scope-key access, grants, erasure,
   holds, fencing, sync and offline semantics. It is not equivalent to moving
   old ledger files to a folder. The current folder remote and single-writer
   model remain authoritative; no cloud backend or retention change is chosen.

SQLite documents why WAL checkpoint frequency trades write throughput for
read/checkpoint cost and why long readers can prevent progress
([official WAL guide](https://www.sqlite.org/wal.html)). FTS5 documents
`optimize` and `rebuild` as index operations
([official FTS5 guide](https://www.sqlite.org/fts5.html)). These support
measuring derived index and WAL behavior; neither permits rewriting Vak's
authoritative ledger, and no automatic maintenance policy is added here.

## Validation and remaining work

Run `node blueprint/harness/test_storage.cjs` for independent arithmetic
checks, retention windows/blocked expiry, version multiplication, copy
budgets, context summation against a direct turn-by-turn oracle, zero work,
invalid rates and long horizons. Run the Blueprint harness and record/doc
checks after changing inputs or source evidence. Browser acceptance and
remaining measurement work are recorded in the one tracker and handover.
