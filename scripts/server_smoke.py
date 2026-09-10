#!/usr/bin/env python3
"""Drive vak serve over HTTP+SSE end-to-end against the mock provider.

Usage: server_smoke.py <vak-binary> <mock-port>
"""
import json
import os
import subprocess
import sys
import threading
import time
import urllib.request

SMOKE_PORT = os.environ.get("VAK_SMOKE_PORT", "8903")
BASE = f"http://127.0.0.1:{SMOKE_PORT}"
BIN = sys.argv[1]
MOCK_PORT = sys.argv[2]
REPO = os.path.dirname(os.path.abspath(__file__))

env = {
    **os.environ,
    "ANTHROPIC_API_KEY": "test",
    # Pin the lane so user-level .env (opencode-zen etc.) can't hijack
    # this offline run into a live provider.
    "VAK_PROVIDER": "anthropic",
    "VAK_MODEL": "claude-sonnet-4-5",
    "VAK_ANTHROPIC_BASE_URL": f"http://127.0.0.1:{MOCK_PORT}",
    "VAK_HOME": "/tmp/vak-smoke/home",
    # Non-interactive servers intentionally suppress generated bearer tokens
    # in stderr. Pin a disposable test token so this driver can authenticate
    # without weakening the production startup logging policy.
    "VAK_GATEWAY_TOKEN": "vak-server-smoke-token",
}

mock = subprocess.Popen(
    ["python3", f"{REPO}/mock_anthropic.py", MOCK_PORT],
)
# The disposable token is supplied explicitly for this non-interactive smoke
# run. The server intentionally does not print configured credentials.
server = subprocess.Popen(
    [BIN, "serve", "--port", SMOKE_PORT],
    env=env,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.PIPE,
)
HDR = {"Authorization": f"Bearer {env['VAK_GATEWAY_TOKEN']}"}
time.sleep(0.5)
if server.poll() is not None:
    print("FAIL: vak server exited before smoke test (port may be occupied)")
    mock.terminate()
    sys.exit(1)

events = []
finish = threading.Event()
opened = threading.Event()
first_cursor = None


def sse_reader(sid, resume=None, disconnect_after_open=False):
    global first_cursor
    suffix = f"?last_event_id={resume}" if resume else ""
    req = urllib.request.Request(f"{BASE}/sessions/{sid}/events{suffix}", headers=HDR)
    with urllib.request.urlopen(req, timeout=15) as resp:
        cursor = None
        for raw in resp:
            line = raw.decode().strip()
            if line.startswith("id:"):
                cursor = line[3:].strip()
            if line.startswith("data:"):
                data = line[5:].strip()
                events.append(data)
                try:
                    v = json.loads(data)
                    if "StreamOpened" in data:
                        opened.set()
                        if disconnect_after_open:
                            first_cursor = cursor
                            return
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
    # Optimized release binaries can take longer than the dev binary to bind.
    # Treat readiness as a bounded wait so a slow startup is not misreported
    # as an application failure.
    health = None
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if server.poll() is not None:
            raise RuntimeError("server exited before becoming ready")
        try:
            with urllib.request.urlopen(f"{BASE}/health", timeout=2) as r:
                health = json.load(r)
                break
        except (urllib.error.URLError, TimeoutError):
            time.sleep(0.25)
    if health is None:
        raise RuntimeError("server did not become ready within 15 seconds")
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

    # The presentation projection establishes its reconnect boundary on the
    # initial snapshot. Read only the first SSE frame and close deliberately.
    presentation_id = None
    with urllib.request.urlopen(
        urllib.request.Request(f"{BASE}/sessions/{sid}/presentation/events", headers=HDR),
        timeout=5,
    ) as response:
        for raw in response:
            line = raw.decode().strip()
            if line.startswith("id:"):
                presentation_id = line[3:].strip()
            if line == "" and presentation_id is not None:
                break
    assert presentation_id is not None, "presentation stream did not emit an initial cursor"
    print(f"PASS: presentation snapshot cursor ({presentation_id})")

    reader = threading.Thread(target=sse_reader, args=(sid,), kwargs={"disconnect_after_open": True}, daemon=True)
    reader.start()
    assert opened.wait(timeout=5), "stream never opened"
    print("PASS: event stream opened")

    # Deliberately run the turn while the first stream is closed. The second
    # connection must use the cursor from the first connection so the server
    # can replay the missed lifecycle/tool events.

    req = urllib.request.Request(
        f"{BASE}/sessions/{sid}/run",
        data=json.dumps({"prompt": "run the smoke test"}).encode(),
        headers={**HDR, "content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=5) as r:
        assert r.status == 202
    print("PASS: run accepted (202)")

    time.sleep(0.25)
    resumed = threading.Thread(target=sse_reader, args=(sid,), kwargs={"resume": first_cursor}, daemon=True)
    resumed.start()

    if not finish.wait(timeout=10):
        print("FAIL: RunFinished never arrived")
        sys.exit(1)
    print("PASS: RunFinished received")
    print(f"PASS: stream resumed from cursor {first_cursor}")

    # Reconnect the presentation projection after the run. Its contract is
    # snapshot-authoritative: the cursor may resume the live event lane, but
    # the first frame must always be a complete schema-v2 timeline that the
    # client can hydrate without reconstructing history itself.
    resumed_presentation_id = None
    resumed_presentation = None
    with urllib.request.urlopen(
        urllib.request.Request(
            f"{BASE}/sessions/{sid}/presentation/events?last_event_id={presentation_id}",
            headers=HDR,
        ),
        timeout=5,
    ) as response:
        for raw in response:
            line = raw.decode().strip()
            if line.startswith("id:"):
                resumed_presentation_id = line[3:].strip()
            if line.startswith("data:"):
                resumed_presentation = json.loads(line[5:].strip())
            if line == "" and resumed_presentation is not None:
                break
    snapshot = (resumed_presentation or {}).get("snapshot", {})
    assert resumed_presentation_id is not None, "presentation reconnect did not emit a cursor"
    assert snapshot.get("schema_version") == 2, "presentation reconnect snapshot has wrong schema"
    assert snapshot.get("session_id") == sid, "presentation reconnect snapshot has wrong session"
    assert isinstance(snapshot.get("items"), list), "presentation reconnect snapshot has no items"
    print(f"PASS: presentation stream reconnected with schema-v2 snapshot ({resumed_presentation_id})")

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
