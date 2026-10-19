# Run-history growth — 2026-10-19

Status: historical — synthetic local debug-build measurement on `45b13c8c5`
plus the ignored benchmark in this continuation. No production SLO or
installed-build claim is made.

```sh
cargo test -p vak-session --lib run_lookup_growth_100_1000_10000 -- --ignored --nocapture
```

The fixture appends distinct synthetic RunOpened events in one batch to an
isolated chain, warms `Runs::get` once, then averages five exact-ID lookups.
It uses the production strict reader, frame verification, event decoding and
run folding. There are no provider calls or conversation records. The target
is the newest run; finding it still folds all history.

| Run events | Warm exact lookup average ms |
|---:|---:|
| 100 | 1.592 |
| 1,000 | 15.484 |
| 10,000 | 154.564 |

The approximately linear curve confirms that repairing strictness did not
repair the history cost. `Runs::get`, `of_trigger`, `of_flow` and abandonment
sweep currently derive from `list`; this fixture measures only `get`, not
scheduler throughput, simultaneous writers or realistic multi-event runs.
These small local debug samples do not establish a latency SLO.

Next implementation should provide indexed/ref-backed run decisions over the
existing RunIds and append-only run chain. It must preserve strict source
verification, missing/truncated-source refusal, atomic cursor updates, trace
and actor fields, trigger claim authority, fencing, and open-run recovery.
Do not introduce another run or trigger model. No index is implemented by
this measurement. Cold replay, seal metadata and other consumers remain open.
