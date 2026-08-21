#!/usr/bin/env python3
"""Drive vakcoder serve over HTTP+SSE end-to-end against the mock provider."""
import json
import os
import subprocess
import sys
import threading
import time
import urllib.request

BASE = "http://127.0.0.1:8903"
BIN = sys.argv[1]
MOCK_PORT = sys.argv[2]

env = {
    **os.environ,
    "ANTHROPIC_API_KEY": "test",
    "VAKCODER_ANTHROPIC_BASE_URL": f"http://127.0.0.1:{MOCK_PORT}",
    "VAKCODER_HOME": "/tmp/vak-smoke/home",
}

mock = subprocess.Popen(
    ["python3", "/Users/nisheethranjan/Projects/vakcoder/scripts/mock_anthropic.py", MOCK_PORT],
)
server = subprocess.Popen(
    [BIN, "serve", "--port", "8903"],
    env=env,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
)
time.sleep(1.5)

events = []
finish = threading.Event()
opened = threading.Event()


def sse_reader(sid):
    req = urllib.request.Request(f"{BASE}/sessions/{sid}/events")
    with urllib.request.urlopen(req, timeout=15) as resp:
        for raw in resp:
            line = raw.decode().strip()
            if line.startswith("data:"):
                data = line[5:].strip()
                events.append(data)
                try:
                    v = json.loads(data)
                    if "StreamOpened" in data:
                        opened.set()
                    if "RunFinished" in v:
                        finish.set()
                        return
                except json.JSONDecodeError:
                    pass


try:
    # health
    with urllib.request.urlopen(f"{BASE}/health", timeout=5) as r:
        assert r.read() == b"ok"

    # create session
    with urllib.request.urlopen(
        urllib.request.Request(f"{BASE}/sessions", method="POST"), timeout=5
    ) as r:
        sid = json.load(r)["session_id"]
    print(f"PASS: session created ({sid[:16]}…)")

    reader = threading.Thread(target=sse_reader, args=(sid,), daemon=True)
    reader.start()
    assert opened.wait(timeout=5), "stream never opened"
    print("PASS: event stream opened")

    # run a prompt
    req = urllib.request.Request(
        f"{BASE}/sessions/{sid}/run",
        data=json.dumps({"prompt": "run the smoke test"}).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=5) as r:
        assert r.status == 202
    print("PASS: run accepted (202)")

    if not finish.wait(timeout=10):
        print("FAIL: RunFinished never arrived")
        sys.exit(1)
    print("PASS: RunFinished received")

    kinds = []
    for e in events:
        try:
            v = json.loads(e)
            if isinstance(v, dict):
                kinds.extend(v.keys())
        except json.JSONDecodeError:
            pass
    checks = {
        "TurnStart streamed": "TurnStart" in kinds,
        "tool call streamed": "ToolCallStart" in kinds,
        "terminal marker": "RunFinished" in kinds,
    }
    for name, ok in checks.items():
        print(f"{'PASS' if ok else 'FAIL'}: {name}")

    # transcript
    with urllib.request.urlopen(f"{BASE}/sessions/{sid}/transcript", timeout=5) as r:
        t = json.load(r)
    ok = t.get("count") == 4 and "smoke-ok" in json.dumps(t)
    print(f"{'PASS' if ok else 'FAIL'}: transcript has full loop (count={t.get('count')})")

    all_ok = all(checks.values()) and ok
    print(f"exit: {0 if all_ok else 1}")
    sys.exit(0 if all_ok else 1)
finally:
    server.terminate()
    mock.terminate()
