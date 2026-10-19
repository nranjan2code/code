# Indexed FinOps measurement — 2026-10-19

Status: historical — local synthetic debug-build measurements based on
`071d89696` plus this continuation. No production latency SLO, provider call,
manual app acceptance or installed-build claim is made.

```sh
cargo test -p vak-core --lib finops_growth_100_1000_10000 -- --ignored --nocapture
```

The original [growth record](finops-growth-measurement-2026-10-19.md) retained
all settled identities in one Rollup. The fixture still seeds the same valid
synthetic CostRows, uses one budget owner and distinct child sessions, and
enables a day cap with explicit fixture pricing. The read now selects the
day/session aggregates needed for admission and requested outstanding IDs,
rather than materializing every settled identity. Warm averages cover 25
reads; durable reserve/release averages cover five pairs. Source verification
and encryption use the production path. All homes are isolated test data.

| Rows | Seed ms | Cold selected read ms | Warm selected average ms | Durable reserve/release average ms |
|---:|---:|---:|---:|---:|
| 100 | 11.660 | 117.622 | 1.270 | 68.857 |
| 1,000 | 56.581 | 102.619 | 2.742 | 67.443 |
| 10,000 | 584.726 | 995.574 | 2.988 | 61.921 |

At 10k the prior warm read averaged 85.40 ms and reserve/release 144.99 ms.
The selected read is about 29 times faster in these samples. Cold rebuild
rises from 301.56 ms to 995.57 ms because it builds the row index, with no
claim that cold work is bounded. Cold timings are noisy; synthetic counts
are small and these averages do not prove concurrent throughput or an SLO.
Reserve/release includes durable reservation Document writes, not only reads.
Many current unresolved reservations and seal metadata still add work.

Verification includes reference-fold parity across days, owners and unknown
corrections; exact floating-point bit preservation; invalid-suffix rollback
of rows and cursor; independent-handle initialization; bound-source mismatch;
empty-versus-deleted source; encryption and live key erasure. Existing
concurrency, duplicate settlement, attribution, crash cleanup, UTC rollover,
unknown-charge and owner-recovery regressions remain required. Runtime readers
no longer consult old Rollup Documents. Full-map inspection exists only in
tests; production summaries deliberately read their requested source history.
