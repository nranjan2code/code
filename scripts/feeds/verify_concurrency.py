#!/usr/bin/env python3
"""Regression check for the DuckDB lock-contention bug in the feeds pipeline.

Background: every read endpoint the admin UI's Feeds page calls on load
(`/feeds/stats`, `/feeds/sources/configured`, `/feeds/alerts`) shells out
to a fresh `python3` subprocess (feed_ingest.py --stats, feed_mcp.py). Each
subprocess used to open feeds.duckdb with a plain read-write connection via
feed_utils.get_db(), and DuckDB allows only one such connection (or many
read-only ones, never both) at a time -- so three parallel page-load
requests would race for the exclusive lock and one would 500. This script
reproduces that scenario directly against the real pipeline scripts (no
mocking) and fails loudly if any request errors.

Usage:
    python3 scripts/feeds/verify_concurrency.py

Exits 0 and prints "PASS" if every concurrent request succeeds, prints the
captured stderr of each failure and exits 1 otherwise. Safe to run against
a real, already-populated feeds.duckdb -- it only performs reads.
"""

from __future__ import annotations

import concurrent.futures as cf
import json
import subprocess
import sys
from pathlib import Path

SCRIPTS_DIR = Path(__file__).parent
ROUNDS = 5
READS_PER_ROUND = 3  # stats, sources, alerts -- matches FeedsSection's page load


def _run_ingest_stats() -> subprocess.CompletedProcess:
    return subprocess.run(
        ["python3", str(SCRIPTS_DIR / "feed_ingest.py"), "--stats"],
        cwd=str(SCRIPTS_DIR),
        capture_output=True,
        text=True,
        timeout=30,
    )


def _run_mcp(tool_name: str) -> subprocess.CompletedProcess:
    request = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": tool_name, "arguments": {}},
    }
    return subprocess.run(
        ["python3", str(SCRIPTS_DIR / "feed_mcp.py")],
        cwd=str(SCRIPTS_DIR),
        input=json.dumps(request),
        capture_output=True,
        text=True,
        timeout=30,
    )


def main() -> int:
    calls = [
        ("feed_stats (GET /feeds/stats)", _run_ingest_stats),
        ("feed_sources (GET /feeds/sources/configured)", lambda: _run_mcp("feed_sources")),
        ("feed_alerts (GET /feeds/alerts)", lambda: _run_mcp("feed_alerts")),
    ]

    failures: list[tuple[str, str]] = []
    total = 0
    with cf.ThreadPoolExecutor(max_workers=READS_PER_ROUND * 2) as pool:
        futures = []
        for _ in range(ROUNDS):
            for label, fn in calls:
                futures.append((label, pool.submit(fn)))
                total += 1
        for label, fut in futures:
            result = fut.result()
            if result.returncode != 0:
                failures.append((label, result.stderr.strip()[-500:]))

    if failures:
        print(f"FAIL: {len(failures)} / {total} concurrent requests errored\n", file=sys.stderr)
        for label, stderr in failures:
            print(f"--- {label} ---", file=sys.stderr)
            print(stderr, file=sys.stderr)
            print(file=sys.stderr)
        return 1

    print(f"PASS: {total} / {total} concurrent requests succeeded across {ROUNDS} rounds")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
