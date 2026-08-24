#!/usr/bin/env python3
"""Drive vakcoder serve over HTTP+SSE end-to-end against the mock provider.

Usage: server_smoke.py <vakcoder-binary> <mock-port>
"""
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
REPO = os.path.dirname(os.path.abspath(__file__))

env = {
    **os.environ,
    "ANTHROPIC_API_KEY": "test",
    # Pin the lane so user-level .env (opencode-zen etc.) can't hijack
    # this offline run into a live provider.
    "VAKCODER_PROVIDER": "anthropic",
    "VAKCODER_MODEL": "claude-sonnet-4-5",
    "VAKCODER_ANTHROPIC_BASE_URL": f"http://127.0.0.1:{MOCK_PORT}",
    "VAKCODER_HOME": "/tmp/vak-smoke/home",
}

mock = subprocess.Popen(
    ["python3", f"{REPO}/mock_anthropic.py", MOCK_PORT],
)
# serve prints its bearer token to stderr; capture it for auth.
server = subprocess.Popen(
    [BIN, "serve", "--port", "8903"],
    env=env,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.PIPE,
)
token = None
for _ in range(50):
    line = server.stderr.readline().decode(errors="replace")
    if "auth token:" in line:
        token = line.split("auth token:")[1].strip()
        break
if not token:
    print("FAIL: no auth token from serve")
    server.terminate()
    mock.terminate()
    sys.exit(1)

HDR = {"Authorization": f"Bearer {token}"}
time.sleep(0.5)

events = []
finish = threading.Event()
opened = threading.Event()


def sse_reader(sid):
    req = urllib.request.Request(f"{BASE}/sessions/{sid}/events", headers=HDR)
    with urllib.request.urlopen(req, timeout=15) as resp:
        for raw in resp:
            line = raw.decode().strip()
            if line.startswith("data:"):
                data = line[5:].strip()
                events.append(data)
                try:
                    v = json.loads(data)
                    if isinstance(v, str) and "StreamOpened" in v:
                        opened.set()
                    if isinstance(v, dict) and "RunFinished" in v:
                        finish.set()
                        return
                    if isinstance(v, dict) and "ApprovalRequested" in v:
                        req_id = v["ApprovalRequested"]["id"]
                        urllib.request.urlopen(
                            urllib.request.Request(
                                f"{BASE}/sessions/{sid}/approvals/{req_id}",
                                data=json.dumps({"approve": True}).encode(),
                                headers={**HDR, "content-type": "application/json"},
                                method="POST",
                            ),
                            timeout=5,
                        ).read()
                except json.JSONDecodeError:
                    pass


try:
    # health stays open
    with urllib.request.urlopen(f"{BASE}/health", timeout=5) as r:
        health = json.load(r)
        assert health.get("status") == "ok", health

    # unauthenticated request must be rejected
    try:
        urllib.request.urlopen(urllib.request.Request(
            f"{BASE}/sessions", method="POST"), timeout=5)
        raise AssertionError("unauthenticated create succeeded")
    except urllib.error.HTTPError as e:
        assert e.code == 401, e.code
    print("PASS: unauthenticated rejected (401)")

    with urllib.request.urlopen(urllib.request.Request(
            f"{BASE}/sessions", method="POST", headers=HDR), timeout=5) as r:
        sid = json.load(r)["session_id"]
    print(f"PASS: session created ({sid[:16]}…)")

    reader = threading.Thread(target=sse_reader, args=(sid,), daemon=True)
    reader.start()
    assert opened.wait(timeout=5), "stream never opened"
    print("PASS: event stream opened")

    req = urllib.request.Request(
        f"{BASE}/sessions/{sid}/run",
        data=json.dumps({"prompt": "run the smoke test"}).encode(),
        headers={**HDR, "content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=5) as r:
        assert r.status == 202
    print("PASS: run accepted (202)")

    if not finish.wait(timeout=10):
        print("FAIL: RunFinished never arrived")
        sys.exit(1)
    print("PASS: RunFinished received")

    kinds = set()
    for e in events:
        try:
            v = json.loads(e)
            if isinstance(v, dict):
                kinds.update(v.keys())
        except json.JSONDecodeError:
            pass
    checks = {
        "TurnStart streamed": "TurnStart" in kinds,
        "tool call streamed": "ToolCallStart" in kinds,
        "terminal marker": "RunFinished" in kinds,
    }
    for name, ok in checks.items():
        print(f"{'PASS' if ok else 'FAIL'}: {name}")

    req = urllib.request.Request(f"{BASE}/sessions/{sid}/transcript", headers=HDR)
    with urllib.request.urlopen(req, timeout=5) as r:
        t = json.load(r)
    ok = t.get("count") == 4 and "smoke-ok" in json.dumps(t)
    print(f"{'PASS' if ok else 'FAIL'}: transcript has full loop (count={t.get('count')})")

    all_ok = all(checks.values()) and ok
    print(f"exit: {0 if all_ok else 1}")
    sys.exit(0 if all_ok else 1)
finally:
    server.terminate()
    mock.terminate()
