# Conversation history implementation acceptance — 2026-10-21

Status: **historical** — measurements of the working-tree implementation on
2026-10-21, with the remaining qualification stated below.

The maintainer approved recent history plus older loading on scroll after
[the growth review](conversation-history-growth-2026-10-21.md). The implemented
contract is in [the web client design](../design/48-web-client.md#conversation-history-pages).
No commit, release, installation or push was performed.

## Checks

- Desktop and web bundles: `npm --prefix crates/vak-client-ui run build`.
- Node tests: `tests/history-pages.mjs` and
  `tests/presentation-hydration.mjs` under `crates/vak-client-ui`, run with
  `node --experimental-strip-types`. Ordering, deduplication, byte-budget
  eviction, adjacent-page refresh and live/settled authority passed.
- Real ChatPane fixture: `tests/history-scroll.html?turns=10000`, served by
  Vite and driven through the in-app browser. Eleven checks passed at
  1440 × 900 and 390 × 844, light and dark. They cover recent 40 turns,
  mounted turn bounds, one older fetch, prepend identities and reading
  anchor, settlement, addressed deltas, paging during an active answer,
  Latest reset and horizontal overflow. Browser error log was empty.
- `cargo test -p vak-storage -p vak-session -p vak-catalog --lib`:
  catalog 20 passed; session 96 passed / 2 ignored; storage 58 passed.
  The newly added catalog chat-scale test also passed separately.
- `cargo test -p vak-server --lib`: 441 passed / 3 ignored after building
  its required worker with `cargo build -p vak-server --bins`. The initial
  run had 18 missing-worker prerequisite failures; those were resolved.
  Tests include canonical turn/entry paging, complete traversal, late
  set-aside overlays, shorter retry branches, incompatible cursors,
  unrelated corrupt historical segments and real participant HTTP layers.
- Targeted session metadata test passed: enumerating a nonexistent ledger
  creates nothing, while strict readers reject missing sealed history.
- Clippy for storage, session, catalog, intent and server, all targets,
  with warnings denied passed. Repository formatting/record/path/version
  checks passed. The full workspace gate was not run; no commit was made.

## Scale measurements

The explicit ignored server test `history_scale_keeps_warm_pages_bounded`
was run separately against real sealed ledgers. The fixture includes
ledger creation and indexing; those are not request latency.

| Retained turns | Fixture + index | Recent read | Recent JSON bytes | Messages |
|---:|---:|---:|---:|---:|
| 100 | 808 ms | 61 ms | 80,538 | 80 |
| 1,000 | 6,379 ms | 110 ms | 80,866 | 80 |
| 10,000 | 62,596 ms | 2,364 ms | 81,188 | 80 |

Every older-page check also returned 80 messages. Unchanged responses were
under 300 bytes. This run overlapped compilation and the catalog's existing
million-node test; its latency is not an isolated capacity benchmark. The
10,000-turn read is still noticeably slower and must not be described as
constant-time. It establishes bounded page content and targeted decoding,
not a latency SLA or heap bound.

The independent catalog summary test seeded 100, 1,000 and 10,000 chats
with identical timestamps, fetched pages of 40 and traversed every id
exactly once. First-page measurements rounded below 1 ms in this run.
It measures the catalog query, not canonical-header/lifecycle checks or an
end-to-end server listing across multiple Agents and workspaces.

## Limits and remaining qualification

The byte target is soft at a whole-turn boundary. One exceptional turn can
exceed both the 2 MiB canonical target and the minimum-one-page cache budget.
Meaningful goal-state overlay chains, first discovery/index construction and
segment-metadata revision checks still grow with retained state. The browser
fixture synthesizes network responses; the separate ledger tests exercise
real storage, but do not replace a complete running-system soak.

Running native-shell acceptance and hostile multi-process paging/erasure
interleavings remain in [open items](../plans/open-items.md). Model-context
planning and append-only retention remain independent of UI paging.


## Commit-gate follow-up

The maintainer subsequently requested committing and pushing all changes.
The full workspace gate found a sync-erasure regression: catalog rebuilding
must omit a ledger with a recorded destroyed scope, while continuing to fail
on missing keys and corruption. Both indexed session and sandbox walks now
handle the typed `Erased` outcome. The real multi-machine `sync_erasure`
integration test passes with the fix. Backup restore also exposed an old
cursor whose indexed row had already been erased; ingestion now checks the
scope tombstone before looking up its byte boundary, and catalog erasure
purges the new history/sandbox address tables. Both real sync-erasure and
backup-restore integration tests pass.

The gate also exposed an unbounded test-fixture failure: the required-card
scripted provider returned a retryable parse error after exhausting its
finite responses. Exhaustion now returns a terminal invalid-request value;
this changes only the test provider, not runtime retry behavior.


The server trash integration test also caught a configured-data-home scope
mismatch in the new readers. Both history pages and summary pages now read
lifecycle through `vak_core::trash::state` with the serving Core's shared
scope. The HTTP test now checks the paged snapshot before and after trash,
in addition to lists/search/export, and passes.


The person-erasure HTTP fixture also supplied six model responses for seven
requests. It now supplies the final reply and treats script exhaustion as
terminal, avoiding a provider-endurance wait unrelated to erasure behavior.


The full workspace tests then passed. The final advisory fetch reported
RUSTSEC-2026-0381/0382 against existing TLS dependencies. The lockfile now
uses rustls-webpki 0.103.16 and webpki-roots 1.0.10. The remaining 0.26.11
webpki-roots package is a compatibility re-export of that patched root set,
verified from its registry source (`pub use parent::TLS_SERVER_ROOTS`) and
lockfile dependency. Its narrowly documented advisory exception is coupled
to a pre-audit guard that rejects vulnerable actual root sets or an
unpatched shim parent. Removing the shim is tracked in open items. The
advisory check passes; the full gate is rerun with the patched dependencies.


Final commit verification: `scripts/gate.sh` passed in full after the fixes
and TLS updates: formatting, workspace/all-target Clippy, workspace tests,
version stamps, documentation paths, records and advisories. No gate waiver
was used. The person-erasure fixture also passed separately in 5.25 seconds
with its complete finite script.
