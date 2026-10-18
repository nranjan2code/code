# FinOps growth measurement — 2026-10-19

Status: historical — local synthetic debug-build measurements based on
`8cf058457` plus this continuation. They establish a remaining growth cost,
not a production SLO or an executed provider transaction.

```sh
cargo test -p vak-core finops_growth_100_1000_10000 -- --ignored --nocapture
```

The fixture isolates its data home, seeds valid CostRows with distinct UUIDv7
reservation IDs in one RecordChain batch, prewarms the strict Rollup, then
averages 25 strict reads and five durable reserve/release pairs. Every row
uses the same budget owner, with distinct child-session attribution. A
synthetic model/provider and explicit fixture prices avoid discovery or
provider calls. Both estimation and actual USD are known; no unknown debt is
discarded. Seeds and reservations stay in the temporary test home.

| Rows | Seed ms | Cold strict read ms | Warm strict average ms | Durable reserve/release average ms |
|---:|---:|---:|---:|---:|
| 100 | 10.71 | 125.36 | 1.44 | 67.67 |
| 1,000 | 53.48 | 64.43 | 9.87 | 71.01 |
| 10,000 | 581.26 | 301.56 | 85.40 | 144.99 |

Cold samples are noisy; the 100-row case is slower than the 1,000-row case.
Warm strict reads show clear growth. `FinOpsLedger::strict_cost_totals_at`
loads a full Rollup Document whose `DailyCostTotals::reservations` retains
one settled identity per reservation. This protects duplicate detection and
verified corrections, but makes each admission decode unrelated history.
The probe enables a day cap; this Rollup cost applies to capped admission,
not every uncapped call. Reserve/release additionally performs durable
reservation Document writes;
its total is not a pure Rollup-read timing. These small synthetic averages
are not a CPU profile or a concurrency/very-large-history capacity claim.

The repair should keep day/session aggregates and individual reservation
identities in an encrypted indexed projection with the source cursor updated
in the same transaction. Preserve strict source validation, unknown-cost
liabilities, exact idempotence, ownership, owner-verified corrections and crash
recovery. Do not drop historical reservation identities or weaken verification
to obtain a lower number. Repeat this benchmark after the repair.
