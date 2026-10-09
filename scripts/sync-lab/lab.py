#!/usr/bin/env python3
"""Two machines in Docker ("desk" and "away") sharing a remote folder.

Runs the sync scenarios against real Linux binaries with real model turns
(the host's Ollama). Each scenario prints PASS or FAIL with what it saw.
Build with build.sh and bring the containers up with up.sh first; see
README.md.
"""
import json, subprocess, sys, tempfile, time, random

PORT = 8900
RESULTS = []


def sh(machine, *cmd, timeout=180, stdin=None):
    p = subprocess.run(["docker", "exec", "-i", "-w", "/work", f"lab-{machine}", *cmd],
                       capture_output=True, text=True, timeout=timeout, input=stdin)
    return p.returncode, (p.stdout + p.stderr).strip()


def vak(machine, *args, timeout=180):
    return sh(machine, "vak", *args, timeout=timeout)


def api(machine, method, path, body=None, timeout=60):
    cmd = ["curl", "-s", "-m", str(timeout), "-o", "/tmp/out", "-w", "%{http_code}",
           "-X", method, "-H", f"Authorization: Bearer lab-token-{machine}",
           "-H", "content-type: application/json", f"http://127.0.0.1:{PORT}{path}"]
    if body is not None:
        cmd += ["-d", json.dumps(body)]
    rc, code = sh(machine, *cmd, timeout=timeout + 10)
    _, raw = sh(machine, "cat", "/tmp/out")
    try:
        data = json.loads(raw)
    except Exception:
        data = {"raw": raw[:300]}
    return (int(code) if code.isdigit() else 0), data


def serve(machine):
    subprocess.run(["docker", "exec", "-d", "-w", "/work", f"lab-{machine}", "sh", "-c",
                    f"vak serve -C /work --port {PORT} --trust >> /data/serve.log 2>&1"])
    for _ in range(60):
        code, _ = api(machine, "GET", "/health", timeout=3)
        if code == 200:
            return True
        time.sleep(1)
    return False


def kill_server(machine, sig="-9"):
    sh(machine, "pkill", sig, "-f", "vak serve")
    for _ in range(20):
        rc, _ = sh(machine, "pgrep", "-f", "vak serve")
        if rc != 0:
            return
        time.sleep(0.5)


def running(machine):
    code, data = api(machine, "GET", "/sessions")
    return any(s.get("running") for s in data.get("sessions", []))


def turn(machine, prompt, wait=150):
    code, made = api(machine, "POST", "/sessions", {})
    sid = made.get("session_id")
    code, started = api(machine, "POST", f"/sessions/{sid}/run", {"prompt": prompt})
    if code >= 300:
        return sid, code, started
    deadline = time.time() + wait
    while time.time() < deadline:
        code, t = api(machine, "GET", f"/sessions/{sid}/transcript")
        if t.get("count", 0) >= 2 and not running(machine):
            return sid, 200, t
        time.sleep(3)
    return sid, 0, {"error": "the turn did not finish"}


def text(machine, sid):
    code, t = api(machine, "GET", f"/sessions/{sid}/transcript")
    return code, json.dumps(t)


def cat(machine, sid):
    return vak(machine, "data", "cat", sid)[1]


def carry_key(src, dst, name):
    vak(src, "sync", "key", "export", f"/data/{name}")
    # Carried by hand, as the owner would: never through the remote.
    with tempfile.TemporaryDirectory() as carried:
        subprocess.run(["docker", "cp", "-q", f"lab-{src}:/data/{name}", f"{carried}/{name}"], check=True)
        subprocess.run(["docker", "cp", "-q", f"{carried}/{name}", f"lab-{dst}:/data/{name}"], check=True)
    return vak(dst, "sync", "key", "import", f"/data/{name}")


def check(name, ok, detail):
    RESULTS.append((name, bool(ok), detail))
    print(("PASS  " if ok else "FAIL  ") + name + "  |  " + detail, flush=True)


def sync_status(machine):
    return api(machine, "GET", "/sync")[1]


def wait_for(pred, seconds, every=3):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if pred():
            return True
        time.sleep(every)
    return False


def main():
    S = {}
    # ---- 1. First copy: a real turn on the desk, push, key file, pull ----
    assert serve("desk"), "desk server did not start"
    S["a"], code, _ = turn("desk", "Reply in one short sentence: the quince jam is ready on Friday.")
    check("1a a real turn on the desk", code == 200 and "quince" in text("desk", S["a"])[1], f"session {S['a'][:13]}")
    code, r = api("desk", "POST", "/sync/setup", {"folder": "/remote"})
    code2, pushed = api("desk", "POST", "/sync/now")
    check("1b set up and first push over HTTP", code == 200 and code2 == 200, f"{pushed.get('files')} files, sync {pushed.get('generation')}")
    rc, out = carry_key("desk", "away", "key1.txt")
    vak("away", "sync", "setup", "/remote")
    rc2, pulled = vak("away", "sync", "pull")
    same = cat("desk", S["a"]) == cat("away", S["a"]) and "quince" in cat("away", S["a"])
    check("1c the away machine reads the same conversation", rc == 0 and rc2 == 0 and same, pulled.splitlines()[0] if pulled else "")
    rc, ver = vak("away", "data", "verify")
    check("1d the away copy verifies", rc == 0 and "0 destroyed" in ver, ver.splitlines()[0])

    # ---- 2. Standing by: the away machine begins nothing ----
    assert serve("away")
    sid, code, refused = turn("away", "Reply with one word: hello", wait=30)
    said = json.dumps(refused)
    check("2a a turn on the standing-by machine is refused", code != 200 and "standing by" in said, f"{code} {said[:110]}")
    code, r = api("away", "POST", "/sync/now")
    check("2b it cannot push", code == 409 and r.get("reason") == "standing_by", str(r)[:100])
    code, r = api("away", "POST", "/sync/takeover", {})
    check("2c it cannot take over before a hand-over", code == 409 and r.get("reason") == "held_elsewhere", str(r)[:100])

    # ---- 3. Automatic push ----
    gen = sync_status("desk").get("generation")
    S["b"], code, _ = turn("desk", "Reply in one short sentence: the saffron ledger closes Thursday.")
    ok = wait_for(lambda: sync_status("desk").get("generation", 0) > gen and sync_status("desk").get("unpushed") == 0, 90)
    st = sync_status("desk")
    check("3 a finished turn is pushed with nobody asking", ok, f"sync {gen} -> {st.get('generation')}, unpushed {st.get('unpushed')}")

    # ---- 4. Hand-over refused mid-turn, then fire and forget ----
    code, made = api("desk", "POST", "/sessions", {})
    busy = made["session_id"]
    api("desk", "POST", f"/sessions/{busy}/run", {"prompt": "Write three sentences about lighthouses."})
    code, r = api("desk", "POST", "/sync/handover")
    check("4a hand-over is refused while a turn runs", code == 409 and r.get("reason") == "in_use", str(r)[:90])
    wait_for(lambda: not running("desk"), 150)
    body = {"name": "morning-note", "kind": {"kind": "schedule", "schedule": {"kind": "interval", "every_secs": 60,
            "anchor": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}},
            "action": {"kind": "prompt", "text": "Reply in one short sentence: the heliotrope bulletin is out."}}
    code, trig = api("desk", "POST", "/triggers", body)
    tid = trig.get("id")
    check("4b an automation is made on the desk", code == 201 and tid, str(trig)[:90])
    wait_for(lambda: not running("desk"), 150)
    code, r = api("desk", "POST", "/sync/handover")
    check("4c the desk hands over", code == 200, str(r)[:100])
    sid, code, refused = turn("desk", "Reply with one word: hello", wait=30)
    check("4d the desk now refuses a turn", code != 200 and "standing by" in json.dumps(refused), str(code))
    # The desk is switched off entirely.
    subprocess.run(["docker", "stop", "-t", "2", "lab-desk"], capture_output=True)
    code, r = api("away", "POST", "/sync/takeover", {})
    check("4e the away machine takes over with the desk off", code == 200 and r.get("restart_required"), str(r)[:120])
    kill_server("away", "-TERM")
    assert serve("away")
    check("4f it holds the work after a restart", sync_status("away").get("role") == "holder", str(sync_status("away").get("role")))
    same = "quince" in cat("away", S["a"]) and "saffron" in cat("away", S["b"])
    check("4g everything the desk did is there", same, "both conversations read")

    def fired():
        code, runs = api("away", "GET", "/runs?limit=40")
        rows = runs.get("runs", runs if isinstance(runs, list) else [])
        return [r for r in rows if tid and tid in json.dumps(r)]
    ok = wait_for(lambda: any("complete" in json.dumps(r) or "succeed" in json.dumps(r) or "ended" in json.dumps(r) for r in fired()), 240, every=10)
    rows = fired()
    check("4h the automation runs on the away machine, unattended", ok and rows, f"{len(rows)} run records name it")
    sid2, code, _ = turn("away", "Reply in one short sentence: the orchard walk is on Sunday.")
    S["c"] = sid2
    check("4i the away machine does new work", code == 200 and "orchard" in text("away", sid2)[1], sid2[:13])
    api("away", "DELETE", f"/triggers/{tid}")
    wait_for(lambda: not running("away"), 200)
    code, r = api("away", "POST", "/sync/handover")
    check("4j the away machine hands back", code == 200, str(r)[:100])
    subprocess.run(["docker", "start", "lab-desk"], capture_output=True)
    time.sleep(2)
    rc, out = vak("desk", "sync", "takeover")
    assert serve("desk")
    got = cat("desk", S["c"])
    code, runs = api("desk", "GET", "/runs?limit=60")
    seen = tid in json.dumps(runs)
    check("4k back on the desk: the away machine's work and the automation's runs are here", rc == 0 and "orchard" in got and seen, out.splitlines()[0] if out else "")

    # ---- 5. The server is killed in the middle of a turn ----
    code, made = api("desk", "POST", "/sessions", {})
    crash = made["session_id"]
    api("desk", "POST", f"/sessions/{crash}/run", {"prompt": "Write five sentences about tide pools."})
    time.sleep(2.5)
    kill_server("desk")
    rc, ver = vak("desk", "data", "verify")
    check("5a after kill -9 mid-turn nothing stored is damaged", rc == 0 and "damaged" not in ver.lower().replace("nothing is damaged", ""), ver.splitlines()[0])
    assert serve("desk")
    ok = wait_for(lambda: "abandon" in json.dumps(api("desk", "GET", "/runs?limit=30")[1]).lower(), 150, every=10)
    check("5b the interrupted run is recorded as abandoned", ok, "runs list names it")
    S["d"], code, _ = turn("desk", "Reply in one short sentence: the pear trees are pruned.")
    check("5c the desk works again after the crash", code == 200 and "pear" in text("desk", S["d"])[1], S["d"][:13])
    ok = wait_for(lambda: sync_status("desk").get("unpushed") == 0, 90)
    check("5d and its push catches up", ok, f"unpushed {sync_status('desk').get('unpushed')}")
    rc, out = vak("away", "sync", "pull")
    kill_server("away", "-TERM")
    check("5e the away machine pulls the post-crash state", rc == 0 and "pear" in cat("away", S["d"]), out.splitlines()[0] if out else "")

    # ---- 6. The push is killed part-way, many times ----
    kill_server("desk", "-TERM")
    killed = good = 0
    for i, ms in enumerate([0.01, 0.02, 0.04, 0.06, 0.09, 0.13, 0.18, 0.25, 0.35, 0.5]):
        vak("desk", "data", "keys", "--rotate")
        carry_key("desk", "away", f"k6-{i}.txt")
        rc, out = sh("desk", "sh", "-c", f"vak sync now >/tmp/p 2>&1 & p=$!; sleep {ms}; kill -9 $p 2>/dev/null && echo KILLED; wait $p 2>/dev/null; true")
        killed += "KILLED" in out
        rc, pull = vak("away", "sync", "pull")
        reads = "quince" in cat("away", S["a"]) and "pear" in cat("away", S["d"])
        rc2, ver = vak("away", "data", "verify")
        good += (rc == 0 and reads and rc2 == 0)
        if not (rc == 0 and reads):
            print("   push-kill", ms, "->", pull[:160], flush=True)
    check("6 a push killed part-way never leaves the remote unusable", good == 10, f"{killed} of 10 pushes were killed; the away machine pulled a whole, readable copy {good} of 10 times")
    vak("desk", "sync", "now")

    # ---- 7. The pull is killed part-way, many times ----
    good = killed = 0
    for i, ms in enumerate([0.02, 0.05, 0.08, 0.12, 0.2, 0.3, 0.45, 0.7]):
        vak("desk", "data", "keys", "--rotate")
        carry_key("desk", "away", f"k7-{i}.txt")
        vak("desk", "sync", "now")
        rc, out = sh("away", "sh", "-c", f"vak sync pull >/tmp/p 2>&1 & p=$!; sleep {ms}; kill -9 $p 2>/dev/null && echo KILLED; wait $p 2>/dev/null; true")
        killed += "KILLED" in out
        rc, pull = vak("away", "sync", "pull")
        reads = "quince" in cat("away", S["a"]) and "pear" in cat("away", S["d"])
        rc2, ver = vak("away", "data", "verify")
        good += (rc == 0 and reads and rc2 == 0)
        if not (rc == 0 and reads and rc2 == 0):
            print("   pull-kill", ms, "->", pull[:200], "|", ver[:120], flush=True)
    check("7 a pull killed part-way is finished by the next pull", good == 8, f"{killed} of 8 pulls were killed; the next pull left a whole, readable, verified copy {good} of 8 times")

    # ---- 8. Keys changed and the key file was not carried ----
    vak("desk", "data", "keys", "--rotate")
    vak("desk", "sync", "now")
    before = cat("away", S["a"])
    rc, out = vak("away", "sync", "pull")
    check("8a a pull is refused when the keys changed and no new key file came", rc != 0 and "key file" in out, out[:110])
    check("8b and nothing on the away machine changed", cat("away", S["a"]) == before and "quince" in before, "still reads its last copy")
    carry_key("desk", "away", "k8.txt")
    rc, out = vak("away", "sync", "pull")
    check("8c with the new key file it pulls", rc == 0 and "quince" in cat("away", S["a"]), out.splitlines()[0] if out else "")

    # ---- 9. The remote folder goes away and comes back ----
    assert serve("desk")
    sh("away", "mv", "/remote/index.json", "/remote/index.gone")
    sh("away", "sh", "-c", "mkdir -p /remote-off")
    # The whole folder becomes unreachable for the desk: swap it for a file.
    sh("desk", "sh", "-c", "true")
    sh("away", "mv", "/remote/index.gone", "/remote/index.json")
    code, _ = api("desk", "POST", "/sync/forget")
    sh("desk", "mkdir", "-p", "/remote/box")
    # Use a sub-folder that can be moved away and back.
    sh("desk", "sh", "-c", "cp -a /remote/blobs /remote/index.json /remote/lease.json /remote/box/")
    api("desk", "POST", "/sync/setup", {"folder": "/remote/box"})
    code, r = api("desk", "POST", "/sync/now")
    sh("away", "mv", "/remote/box", "/remote/unplugged")
    S["e"], code, _ = turn("desk", "Reply in one short sentence: the lanterns are lit at dusk.")
    worked = code == 200 and "lantern" in text("desk", S["e"])[1]
    code, r = api("desk", "POST", "/sync/now")
    ok = wait_for(lambda: sync_status("desk").get("last_error"), 60)
    st = sync_status("desk")
    check("9a with the remote unreachable the desk still works and says the push is owed", worked and code == 409 and r.get("reason") == "unreachable" and st.get("unpushed", 0) > 0,
          f"reachable {st.get('reachable')}, unpushed {st.get('unpushed')}, said: {str(st.get('last_error'))[:50]}")
    sh("away", "mv", "/remote/unplugged", "/remote/box")
    code, r = api("desk", "POST", "/sync/now")
    st = sync_status("desk")
    check("9b when it is back the owed push goes through", code == 200 and st.get("unpushed") == 0, f"sync {r.get('generation')}, {r.get('copied')} files copied")
    # The away machine is pointed at the folder's new place: a remote it
    # has not synced with, so it is told to replace what it holds.
    vak("away", "sync", "setup", "/remote/box")
    rc0, out0 = vak("away", "sync", "pull")
    check("9c0 a machine pointed at a remote it never synced with is not overwritten unasked", rc0 != 0 and "never pushed" in out0, out0[:90])
    rc, out = vak("away", "sync", "pull", "--discard")
    check("9c and the away machine gets what was written meanwhile", rc == 0 and "lantern" in cat("away", S["e"]), out.splitlines()[0] if out else "")

    # ---- 10. A lost machine: force take-over, then it comes back ----
    S["f"], code, _ = turn("desk", "Reply in one short sentence: the kiln is cooling overnight.")
    wait_for(lambda: sync_status("desk").get("unpushed") == 0, 90)
    # Work the desk never gets to push: its remote is cut first.
    sh("away", "mv", "/remote/box", "/remote/unplugged")
    S["g"], code, _ = turn("desk", "Reply in one short sentence: the weathervane points north.")
    kill_server("desk")
    sh("away", "mv", "/remote/unplugged", "/remote/box")
    rc, out = vak("away", "sync", "takeover")
    check("10a an unreleased lease refuses a plain take-over", rc != 0 and "has not handed over" in out, out[:80])
    rc, out = vak("away", "sync", "takeover", "--force")
    has = "kiln" in cat("away", S["f"])
    lacks = "weathervane" not in cat("away", S["g"])
    check("10b a forced take-over brings what was pushed and not what never was", rc == 0 and has and lacks, out.splitlines()[0] if out else "")
    assert serve("away")
    S["h"], code, _ = turn("away", "Reply in one short sentence: the ferry leaves at nine.")
    wait_for(lambda: sync_status("away").get("unpushed") == 0, 90)
    check("10c the away machine works and pushes as the holder", code == 200 and sync_status("away").get("role") == "holder", "ferry turn done")
    # The desk comes back, still believing it holds the work.
    assert serve("desk")
    code, r = api("desk", "POST", "/sync/now")
    check("10d the returning desk is refused and told what it never pushed", code == 409 and r.get("reason") == "lost" and "never pushed" in r.get("error", ""), r.get("error", "")[:110])
    sid, code, refused = turn("desk", "Reply with one word: hello", wait=30)
    check("10e and it begins no turn", code != 200 and "standing by" in json.dumps(refused), str(code))
    code_r, remote_gen = 0, sync_status("away").get("remote_generation")
    check("10f the remote is what the away machine wrote", sync_status("away").get("generation") == remote_gen, f"sync {remote_gen}")
    kill_server("desk", "-TERM")
    rc, out = vak("desk", "sync", "pull")
    check("10g the desk will not drop its unpushed work unasked", rc != 0 and "never pushed" in out, out[:100])
    rc, out = vak("desk", "sync", "pull", "--discard")
    check("10h told to, it pulls and stands by with the away machine's work", rc == 0 and "ferry" in cat("desk", S["h"]), out.splitlines()[0] if out else "")

    # ---- 11. An erasure on one machine reaches the other ----
    code, _ = api("away", "POST", f"/sessions/{S['f']}/archive", {"archived": True})
    code1, _ = api("away", "DELETE", f"/sessions/{S['f']}")
    code, looked = api("away", "GET", f"/conversations/{S['f']}/erasure")
    code, done = api("away", "POST", f"/conversations/{S['f']}/erasure",
                     {"digest": looked.get("preview", {}).get("digest", ""), "confirm": looked.get("confirm", "")})
    check("11a a conversation is erased on the away machine", code == 200 and done.get("receipt", {}).get("scope") == "conversation", f"trash {code1}, erase {code}")
    wait_for(lambda: sync_status("away").get("unpushed") == 0, 90)
    had = "kiln" in cat("desk", S["f"])
    rc, out = vak("desk", "sync", "pull")
    gone = "kiln" not in cat("desk", S["f"])
    rc2, receipts = vak("desk", "data", "receipts")
    check("11b the desk had it, pulls, and no longer has it; the receipt came too", had and rc == 0 and gone and "signature ok" in receipts, f"had {had}, gone {gone}")
    rc, raw = sh("away", "sh", "-c", "grep -rl kiln /remote/box 2>/dev/null | wc -l")
    check("11c nothing readable of it is in the remote", raw.strip() == "0", f"{raw.strip()} files mention it")

    # ---- 12. Both machines try to take over at once ----
    kill_server("away", "-TERM")
    vak("away", "sync", "handover")
    procs = [subprocess.Popen(["docker", "exec", "-w", "/work", f"lab-{m}", "vak", "sync", "takeover"],
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True) for m in ("desk", "away")]
    outs = [p.communicate()[0] for p in procs]
    vak("desk", "data", "rules", "--set", "trash=61")
    vak("away", "data", "rules", "--set", "trash=62")
    pushes = [vak(m, "sync", "now") for m in ("desk", "away")]
    winners = [m for m, (rc, _) in zip(("desk", "away"), pushes) if rc == 0]
    losers = [o for rc, o in pushes if rc != 0]
    check("12 when both take over at once, exactly one may push", len(winners) == 1 and all("took the work over" in o or "standing by" in o for o in losers),
          f"winner: {winners}; the other was told: {(losers[0][:70] if losers else '-')}")

    print()
    bad = [r for r in RESULTS if not r[1]]
    print(f"{len(RESULTS) - len(bad)} of {len(RESULTS)} checks passed")
    for name, _, detail in bad:
        print("  FAILED:", name, "|", detail)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    random.seed(7)
    main()
