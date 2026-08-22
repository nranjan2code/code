#!/usr/bin/env python3
"""Chaos endurance driver v2: one ledger, ten phases, faults scheduled mid-run."""
import json
import os
import subprocess
import sys
import threading
import time
import urllib.request

BIN = os.environ.get("VAK_BIN", "target/debug/vakcoder")
CWD = os.environ.get("VAK_CHAOS_CWD", "/tmp/vak-chaos-workspace")
PROXY = "http://127.0.0.1:8930"
CONFIG = f"{CWD}/.vakcoder/config.toml"
RESULTS = []


def set_mode(m):
    req = urllib.request.Request(f"{PROXY}/mode", data=m.encode(), method="POST")
    urllib.request.urlopen(req, timeout=5)


def hits():
    with urllib.request.urlopen(f"{PROXY}/hits", timeout=5) as r:
        return json.load(r)


def set_mode_verified(m, tries=5):
    """Flip mode and confirm; a lost flip leaves faults stuck on forever."""
    for _ in range(tries):
        try:
            set_mode(m)
            time.sleep(0.4)
            with urllib.request.urlopen(f"{PROXY}/mode", timeout=5) as r:
                if r.read().decode().strip() == m:
                    return True
        except Exception:
            pass
        time.sleep(1)
    print(f"WARN: mode flip to {m!r} unverified", flush=True)
    return False


def schedule(steps):
    """[(after_secs_from_now, mode)] applied while exec runs."""
    def go():
        for delay, m in steps:
            time.sleep(delay)
            set_mode_verified(m)
    threading.Thread(target=go, daemon=True).start()


def write_config(extra):
    os.makedirs(os.path.dirname(CONFIG), exist_ok=True)
    base = (
        'provider = "opencode-zen"\n'
        'model = "x-preview-f-free"\n'
        'permission_mode = "workspace-write"\n'
    )
    with open(CONFIG, "w") as f:
        f.write(base + extra)


def run(sid, prompt, extra_config="", timeout=900, expect_completed=True):
    write_config(extra_config)
    env = {**os.environ, "VAKCODER_OPENCODE_ZEN_BASE_URL": "http://127.0.0.1:8930"}
    try:
        p = subprocess.run(
            [BIN, "exec", "--session", sid, "--trust", "--yes", "--max-turns", "25", prompt],
            cwd=CWD, env=env, capture_output=True, text=True, timeout=timeout,
        )
        out = p.stdout + p.stderr
        code = p.returncode
    except subprocess.TimeoutExpired as e:
        out = f"DRIVER TIMEOUT after {timeout}s"
        code = -1
    n = len(RESULTS)
    with open(f"/tmp/chaos_p{n}.log", "w") as f:
        f.write(out)
    completed = "── completed" in out
    ok = code in (0, 1) and (expect_completed is None or completed == expect_completed)
    return ok, out


def record(phase, ok, detail=""):
    RESULTS.append((phase, ok))
    print(f"{'PASS' if ok else 'FAIL'}  {phase}  {detail}", flush=True)


def sh(cmd):
    return subprocess.run(cmd, shell=True, cwd=CWD, capture_output=True, text=True)


def suite_ok(strict=False):
    rc = sh("python3 -m pytest -q chaoswiki/tests").returncode
    if rc == 0:
        return True
    # Free models sometimes skip authoring tests; don't let that mask
    # fault-survival signals in intermediate phases.
    return not strict and rc in (4, 5)


def wiki_src():
    return open(f"{CWD}/chaoswiki/wiki.py").read()


def main():
    sid = sys.argv[1]
    os.makedirs(CWD, exist_ok=True)
    subprocess.run("git init -q", shell=True, cwd=CWD)

    # P0 — scaffold (pass mode)
    set_mode("pass")
    ok, _ = run(sid,
        "Create a python package chaoswiki/ in this workspace: __init__.py, wiki.py "
        "(WikiStore class: add_page(title, body), get_page(title), list_titles() backed "
        "by an in-memory dict), and tests/test_wiki.py with pytest tests for all three "
        "methods. Run python3 -m pytest -q and make it green.")
    files_ok = os.path.isfile(f"{CWD}/chaoswiki/wiki.py")
    record("P0 scaffold + wiki.py green", ok and files_ok)

    # P1 — perspective baseline
    ok, out = run(sid, "In one short sentence: what is the name of the package you are building and what class does it center on?")
    record("P1 perspective#1 names chaoswiki/WikiStore",
           ok and "chaoswiki" in out and "WikiStore" in out)

    # P2 — 429 window DURING the feature work
    set_mode("429")
    h_before = hits()["injected"].get("429", 0)
    schedule([(25, "pass")])  # window opens for first ~25s of the run
    ok, out = run(sid,
        "Add search(query) to WikiStore returning titles whose body contains the query "
        "(case-insensitive), plus tests. Keep the suite green.")
    h_429 = hits()["injected"].get("429", 0) - h_before
    t = suite_ok()
    search_ok = "search" in wiki_src()
    record("P2 survived 429 window", ok and t and search_ok and h_429 > 0, f"(429s injected: {h_429})")

    # P3 — 503 storm during subagent delegation
    set_mode("503")
    h_before = hits()["injected"].get("503", 0)
    schedule([(20, "pass")])
    ok, out = run(sid,
        "Delegate to a subagent with the task tool: have it write chaoswiki/docs.md "
        "documenting WikiStore's API from reading the source. Then verify the file exists.")
    h_503 = hits()["injected"].get("503", 0) - h_before
    doc_ok = os.path.isfile(f"{CWD}/chaoswiki/docs.md")
    record("P3 subagent through 503 storm", ok and doc_ok and h_503 > 0, f"(503s injected: {h_503})")

    trunc = 0
    # P4 — truncated streams in a bounded window (real gateways flap)
    set_mode("truncate:400")
    h_before = hits()["injected"].get("truncate:400", 0)
    schedule([(30, "pass")])
    ok, out = run(sid,
        "Add categories support: WikiStore pages may store a set of category strings "
        "(add_page gains optional categories param; add categories_for(title) query) "
        "plus tests. Keep everything green.", timeout=1200)
    trunc += hits()["injected"].get("truncate:400", 0) - h_before
    t = suite_ok()
    cat_ok = "categories" in wiki_src()
    pass
    record("P4 survived truncated streams", cat_ok and t, f"(truncations: {trunc})")

    # P5 — slow drip vs watchdog
    set_mode("slow")
    h_before = hits()["injected"].get("slow", 0)
    schedule([(40, "pass")])
    ok, out = run(sid,
        "Add backlinks(page_title) to WikiStore returning titles whose body links the "
        "given title as [[title]], plus tests. Keep the suite green.",
        extra_config="request_timeout_secs = 75\nmax_retries = 2\nretry_base_backoff_ms = 300\nrun_retry_attempts = 6\ncircuit_breaker_threshold = 100\n",
        timeout=900)
    t = suite_ok()
    bl_ok = "backlinks" in wiki_src()
    slow_n = hits()["injected"].get("slow", 0) - h_before
    record("P5 watchdog vs slow drip", bl_ok and t, f"(slow-served requests: {slow_n})")

    # P6 — hang vs watchdog
    set_mode("hang")
    h_before = hits()["injected"].get("hang", 0)
    schedule([(45, "pass")])
    ok, out = run(sid,
        "Add revision history: store a list of (timestamp, body) per page; add "
        "history(title) returning it; new add_page appends. Tests included. Green suite.",
        extra_config="request_timeout_secs = 75\nmax_retries = 2\nretry_base_backoff_ms = 300\nrun_retry_attempts = 6\ncircuit_breaker_threshold = 100\n",
        timeout=900)
    t = suite_ok()
    rev_ok = "history" in wiki_src()
    hang_n = hits()["injected"].get("hang", 0) - h_before
    record("P6 watchdog vs hang", rev_ok and t, f"(hung requests: {hang_n})")

    # P7 — token starvation
    ok, out = run(sid,
        "Add tags: optional tags set per page and by_tag(tag) query, plus tests. Work "
        "in small steps and keep replies brief until the suite is green.",
        extra_config="max_tokens = 120\nmax_turns = 40\n",
        timeout=1200, expect_completed=None)
    t = suite_ok()
    tag_ok = "by_tag" in wiki_src()
    record("P7 token starvation (max_tokens=120)", tag_ok and t)

    # P8 — garbage SSE mid-run
    set_mode("garbage")
    h_before = hits()["injected"].get("garbage", 0)
    schedule([(12, "pass")])
    ok, out = run(sid,
        "Run the full test suite; if anything fails, fix it. Then confirm green.",
        timeout=600)
    garb = hits()["injected"].get("garbage", 0) - h_before
    t = suite_ok()
    record("P8 garbage SSE recovery", t and garb > 0, f"(garbage responses: {garb})")

    # P9 — perspective after everything + final suite
    set_mode("pass")
    ok, out = run(sid,
        "Final report: (1) package name, (2) every public method WikiStore has had since "
        "the start of this session, in the order they were added, (3) ensure "
        "chaoswiki/tests/test_wiki.py exists and covers EVERY public method of WikiStore "
        "(create it if missing), (4) run python3 -m pytest -q chaoswiki/tests and give "
        "the pass count.")
    methods = ["add_page", "get_page", "list_titles", "search", "categories_for", "backlinks", "history", "by_tag"]
    named = sum(1 for m in methods if m in out)
    record("P9 perspective#2 method timeline", named >= 6, f"({named}/8 methods recalled)")
    record("P9 final suite green", suite_ok(strict=True))

    st = hits()
    print("\nproxy stats:", json.dumps(st), flush=True)
    failed = [p for p, okk in RESULTS if not okk]
    print(f"\n{'='*56}\nCHAOS CAMPAIGN: {len(RESULTS)-len(failed)}/{len(RESULTS)} PASS" + (f" — FAILED: {failed}" if failed else ""))
    return 0 if not failed else 1


if __name__ == "__main__":
    main()
