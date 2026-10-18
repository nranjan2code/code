# Effects projection measurement — 2026-10-19

Status: historical — local synthetic measurements of the implementation
based on `9dd55158e` plus this continuation. These are microbenchmarks, not
production latency guarantees or browser acceptance.

The previous implementation decoded one whole EffectIndex Document per read.
The same growth fixture now seeds the real RecordChain and queries the
production encrypted row projection. Large batches rotate at the normal
segment threshold. Each read opens the projection, verifies source history
since its stored position, and selects its requested rows. No provider calls,
personal data, installed app changes or network requests are involved.

Reproduce with:

```sh
cargo test -p vak-session effects_index_growth_measurement -- --ignored --nocapture
```

| Effects | Cold page ms | Warm get ms | Warm 25-row page ms |
|---:|---:|---:|---:|
| 100 | 124.904 | 1.812 | 2.016 |
| 1,000 | 234.426 | 3.408 | 3.836 |
| 10,000 | 1511.215 | 3.096 | 3.814 |

The prior 2026-10-18 run measured 307.920 ms warm get and 308.245 ms warm
page at 10,000 rows. The new warm page is about 81 times faster in these
single samples. Cold rebuild is slower than the prior 502.670 ms cold page:
it builds several row indexes and must replay history. An intermediate
measurement exposed an oversized batch segment and repeated payload opening;
fixing both was necessary to reach the final warm results.

Detail, cursor pages, run filtering, dispatchable work, prepare ordinals and
idempotency lookups now avoid decoding unrelated history. Cold replay and
seal-manifest verification still grow with history; a full-list API still
materializes all requested records. The benchmark is small and synthetic,
uses wall-clock single samples, and does not establish concurrent throughput,
very large-history scaling or a capacity SLO.

Regression evidence covers first-open concurrency, mismatched source refusal,
empty versus deleted source, transactional rollback after an invalid suffix,
filtered pagination, ciphertext checks, live-handle key erasure, batch
rotation, strict tail integrity and scoped payload access. The source remains
`crates/vak-session/src/effects.rs`, its `effects/projection.rs` module,
`crates/vak-session/src/tail.rs`, `crates/vak-session/src/chain.rs` and
`crates/vak-storage/src/records.rs`.
