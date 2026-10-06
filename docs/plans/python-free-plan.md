# Python-free Vakyartha

Status: **plan, 2026-10-02.** The feed runtime part is done another way:
data-architecture M6.5 (2026-10-06) deleted `scripts/feeds/` and replaced it
with Rust connectors in `crates/vak-intake` feeding the catalog, not the
`crates/vak-feeds` and SQLite store sketched below. Nothing else below is
built. Decisions locked with the
maintainer: scope is runtime and dev tooling; the feed store becomes SQLite via
`vak-store`; there are no users, so no migration and no compatibility code
(invariants 29 and 30). Old DuckDB feed data is simply dropped.

Goal: the repository's stack is Rust and SolidJS. Shell scripts and the
sandbox's user-visible `python3` (agents running user code, project launch
detection in `vak-server/src/lib.rs`) are out of scope and stay.

## Inventory (taken at `989002e0b`)

Runtime, shipped in the install bundle and spawned as `python3` by
`crates/vak-server/src/feeds.rs`, found via `feed_ingest.py` in
`crates/vak/src/install/bundle.rs`: `scripts/feeds/` (3,793 lines). Third-party
Python it needs: `duckdb`, `feedparser`, `requests`, `nh3`.

Dev tooling (3,174 lines): `site/build.py`, `site/build_vercel.py`,
`check_doc_paths.py`, `integrity-manifest.py`, `merge-feeds.py`, mock providers
(`mock_anthropic/gemini/openai/openai_responses`, `fake_mcp_server`,
`fault_proxy`), harnesses (`chaos_driver`, `harness_500`, `harness_selftest`,
`compound_regression`, `prompt_scenarios`, `server_smoke`, `resource_watch`,
`test_harness_500`) and `docs/research/.../run-probes.py`. Shell callers:
`build.sh`, `release.sh`, `install.sh`, `upgrade-gate.sh`, `macos-check.sh`,
`linux-check.sh`, `build-amazonlinux-arm64.sh`, `docker/Dockerfile`,
`crates/vak-server/build.rs` (error text).

## Stages

1. **`crates/vak-feeds`** (replaces `scripts/feeds/`, 3.8k lines). Modules map
   one to one: security (URL/SSRF validation, HTML sanitising, injection
   detection), config + source registry, store (SQLite, FTS5 instead of the
   hand-rolled BM25 in `feed_search.py`), drivers (rss, youtube, aggregator,
   custom http), ingest with the file lock, alerts, search, and the MCP stdio
   server as `vak __feed_mcp`. `feeds.rs` calls it in-process (blocking work on a
   blocking worker); delete `run_feed_script`, the `python3` spawns, the PATH
   note and the bundle's script lookup in the same change. Exit tests: the
   contract tests in `test_feed_contract.py` and `verify_concurrency.py`
   ported as Rust tests; a live ingest of a real RSS feed.
   Dependencies need pinned exact versions and justification (`feed-rs`,
   `ammonia` for `nh3`; `reqwest` is already present).
2. **Build and check tools as `xtask`** (`cargo xtask site|doc-paths|integrity`):
   `build.py`, `build_vercel.py`, `check_doc_paths.py`, `integrity-manifest.py`,
   `merge-feeds.py`. Update `build.rs`, the shell scripts, CI, AGENTS.md
   ("python3 ... build.py" and the verification commands) and
   `site/README.md`.
3. **Test doubles and harnesses** into one `vak-dev` crate (bin): the four mock
   providers, `fake_mcp_server`, `fault_proxy`, then `chaos_driver`,
   `harness_*`, `compound_regression`, `prompt_scenarios`, `server_smoke`,
   `resource_watch`. Tests that spawn the mocks switch to the Rust binary.
4. **Close out**: no `.py` tracked outside fixtures a test writes at runtime;
   Dockerfile and install scripts stop installing Python; add a CI check that
   fails on a tracked `.py`; amend docs 51 (status: Rust pipeline), 76 §9 notes
   and AGENTS.md layout entries; CHANGELOG entry.

## Stage 1 is a redesign, not a transliteration

Maintainer, 2026-10-02: with no users, any design fault in the feed system, and
any correction needed to make it actually work, is in scope for Stage 1. Do not
port bugs faithfully and do not re-ask whether to fix one. Stage 1 therefore
starts with a short audit of the shipped pipeline (docs 51 and 76 §9, the
contract tests, a live ingest of real sources) and records each defect found
with its fix in this section before the port begins. Limit: doc 76's target
model (one intake path into the data catalog) depends on data-architecture M6.5
and is not started here; the Rust crate is built so that move is a swap of its
store, not a rewrite.

## Order and cost

Stage 1 is the only one that changes behaviour and is roughly four-fifths of
the work; do it first and alone. Stages 2 and 3 are mechanical. Each stage ships
whole (code, tests, docs, the removed path), per invariant 30.
