#!/usr/bin/env python3
"""The mail and calendar soak (README.md): one machine, a fake Google, three
routines, and faults for as long as `--hours` says.

  python3 soak.py --hours 24

It links an account through the real OAuth flow against the fake, makes a
watch, a digest and a calendar-event routine, feeds the fake new mail and
meetings, and every 10 to 20 minutes injects one fault: the server killed
or stopped, the machine paused (sleep and wake), the provider failing,
rate-limiting or hanging, the fake gone (network loss), access tokens
rejected, the refresh token revoked (then the account linked again), the
mail history expired, a second server on the same data home (duplicate
triggers), a routine paused mid-run, and a burst of meetings past the queue
ceiling. Every half hour it checks what must hold, and at the end it writes
`report.json` beside this file's output directory.
"""
import argparse, calendar, json, random, subprocess, sys, time
from urllib.parse import urlencode

C = "mail-soak"
PORT = 8900
TOKEN = "soak-token"
AGENT = "scout"
LOG = []
CHECKS = []
FAULTS = []
SCALE = 1.0


def minutes(low, high):
    """A random wait of low to high minutes, in seconds, times --scale."""
    return random.randint(low, high) * 60 * SCALE


def say(*parts):
    line = time.strftime("%H:%M:%S ") + " ".join(str(p) for p in parts)
    print(line, flush=True)
    LOG.append(line)


def sh(*cmd, timeout=120):
    try:
        p = subprocess.run(["docker", "exec", "-i", "-w", "/work", C, *cmd],
                           capture_output=True, text=True, timeout=timeout)
        return p.returncode, (p.stdout + p.stderr).strip()
    except subprocess.TimeoutExpired:
        return 124, "timed out"


def http(method, url, body=None, timeout=60, auth=True, port=PORT):
    cmd = ["curl", "-s", "-m", str(timeout), "-o", "/tmp/out", "-w", "%{http_code} %{redirect_url}",
           "-X", method, "-H", "content-type: application/json"]
    if auth:
        cmd += ["-H", f"Authorization: Bearer {TOKEN}"]
    if body is not None:
        cmd += ["-d", json.dumps(body)]
    target = url if url.startswith("http") else f"http://127.0.0.1:{port}{url}"
    rc, head = sh(*cmd, target, timeout=timeout + 10)
    _, raw = sh("cat", "/tmp/out")
    code, _, redirect = head.partition(" ")
    try:
        data = json.loads(raw) if raw else {}
    except Exception:
        data = {"raw": raw[:300]}
    return (int(code) if code.isdigit() else 0), data, redirect.strip()


def fake(path, body=None):
    code, data, _ = http("POST" if body is not None else "GET", f"http://127.0.0.1:9100/_soak/{path}",
                         body, auth=False)
    return data


def check(name, ok, detail):
    CHECKS.append({"at": time.time(), "name": name, "ok": bool(ok), "detail": detail})
    say(("PASS " if ok else "FAIL ") + name + " | " + str(detail))


def serve(port=PORT):
    subprocess.run(["docker", "exec", "-d", "-w", "/work", C, "sh", "-c",
                    f"vak serve -C /work --port {port} --trust >> /data/serve-{port}.log 2>&1"])
    for _ in range(90):
        code, _, _ = http("GET", "/health", timeout=3, auth=False, port=port)
        if code == 200:
            return True
        time.sleep(1)
    return False


def stop(sig="-TERM", port=PORT):
    sh("pkill", sig, "-f", f"vak serve -C /work --port {port}")
    for _ in range(40):
        if sh("pgrep", "-f", f"vak serve -C /work --port {port}")[0] != 0:
            return
        time.sleep(0.5)


def health():
    code, data, _ = http("GET", "/health", timeout=5, auth=False)
    return code, data.get("status")


def ensure_up():
    code, status = health()
    if code == 200 and status != "fenced":
        return
    stop("-9")
    serve()


# ---- setup ----

def write_agent():
    agent = {"id": AGENT, "revision": 1, "lifecycle": "active", "name": "Scout", "character": "vak",
             "personality": "Calm.", "behaviour": "Brief.", "animation": "off", "voice": "default"}
    payload = json.dumps([agent])
    sh("sh", "-c", f"cat > /work/.vak/agents.json <<'JSON'\n{payload}\nJSON")


def link_account():
    code, begun, _ = http("POST", f"/mail-calendar/accounts/{AGENT}/oauth",
                          {"provider": "google", "capabilities": ["mail_read", "calendar_read"]})
    url = begun.get("authorization_url") or begun.get("url") or begun.get("authorizationUrl")
    if code != 200 or not url:
        return False, f"begin {code} {str(begun)[:160]}"
    code, _, redirect = http("GET", url, auth=False)
    if code != 302 or not redirect:
        return False, f"authorize {code}"
    code, done, _ = http("GET", redirect, auth=False)
    if code not in (200, 302, 303):
        return False, f"callback {code} {str(done)[:160]}"
    return True, "linked"


def account_id():
    code, data, _ = http("GET", "/mail-calendar/accounts?" + urlencode({"agent_id": AGENT}))
    accounts = data.get("accounts", data if isinstance(data, list) else [])
    live = [a for a in accounts if a.get("status") == "connected" and not a.get("revoked_at")]
    return (live[-1].get("id") or live[-1].get("account_id")) if live else None


def make_trigger(name, every, scope, prompt):
    body = {"name": name, "enabled": True,
            "kind": {"kind": "schedule", "schedule": {"kind": "interval", "every_secs": every,
                     "anchor": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}},
            "action": {"kind": "prompt", "text": prompt}, "agent": AGENT, "agent_revision": 1,
            "scope": scope}
    code, made, _ = http("POST", "/triggers", body)
    if code != 201:
        return None, f"{code} {str(made)[:200]}"
    if not made.get("enabled"):
        code, made2, _ = http("PUT", f"/triggers/{made['id']}", body)
        if code != 200:
            return made["id"], f"enable {code} {str(made2)[:160]}"
    return made["id"], "made"


def routines(account):
    base = {"account_id": account, "max_items": 5, "read_commitments": False}
    triggers = {}
    triggers["watch"], why = make_trigger("Soak watch", 60, dict(base, operations=["recent_mail"], watch_new_mail=True),
                                         "Say in one sentence what the new mail is about.")
    say("watch", why)
    triggers["digest"], why = make_trigger("Soak digest", 900, dict(base, operations=["recent_mail"], watch_new_mail=False),
                                          "Summarize the recent mail in two sentences.")
    say("digest", why)
    triggers["meetings"], why = make_trigger("Soak meetings", 120, dict(base, operations=["calendar_events"], watch_new_mail=False,
                                            calendar_event_trigger={"boundary": "start", "offset_minutes": -10, "max_lateness_minutes": 30}),
                                            "Say in one sentence which meeting starts soon.")
    say("meetings", why)
    return triggers


# ---- faults ----

def fault_restart():
    stop(random.choice(["-9", "-TERM"]))
    return serve(), "server stopped and started again"


def fault_sleep():
    wait = minutes(5, 15)
    subprocess.run(["docker", "pause", C], capture_output=True)
    time.sleep(wait)
    subprocess.run(["docker", "unpause", C], capture_output=True)
    return True, f"machine paused {wait / 60:.1f} min"


def fault_provider(mode):
    wait = minutes(3, 8)
    fake("fault", {"mode": mode, "seconds": wait})
    time.sleep(wait)
    return True, f"provider {mode} {wait / 60:.1f} min"


def fault_network():
    wait = minutes(3, 10)
    sh("pkill", "-f", "fake.py")
    time.sleep(wait)
    subprocess.run(["docker", "exec", "-d", C, "sh", "-c", "python3 /soak/fake.py >> /data/fake.log 2>&1"])
    time.sleep(3)
    return True, f"provider unreachable {wait / 60:.1f} min"


def fault_tokens():
    fake("fault", {"mode": "reject_tokens"})
    return True, "access tokens rejected"


def account_status(account):
    code, data, _ = http("GET", "/mail-calendar/accounts?" + urlencode({"agent_id": AGENT}))
    for item in data.get("accounts", data if isinstance(data, list) else []):
        if (item.get("id") or item.get("account_id")) == account:
            return item.get("status")
    return None


def fault_revoke(state):
    fake("fault", {"mode": "revoke_refresh"})
    # The owner links the account again once Vakyartha says it needs sign-in.
    deadline = time.time() + 20 * 60
    while time.time() < deadline and account_status(state["account"]) == "connected":
        time.sleep(15)
    noticed = account_status(state["account"])
    if noticed == "connected":
        return False, "the revoked account still looked connected after 20 min"
    time.sleep(minutes(1, 3))
    ok, why = link_account()
    new = account_id()
    if ok and new and new != state["account"]:
        for key, trigger in state["triggers"].items():
            if trigger:
                http("DELETE", f"/triggers/{trigger}")
        state["account"] = new
        state["triggers"] = routines(new)
    return ok, f"refresh revoked, linked again: {why}"


def fault_history():
    fake("fault", {"mode": "history_reset"})
    return True, "mail history expired"


def fault_duplicate():
    wait = minutes(5, 10)
    started = serve(PORT + 1)
    time.sleep(wait)
    stop("-TERM", PORT + 1)
    return started, f"second server on the same data home {wait / 60:.1f} min"


def fault_pause_mid_run(state):
    watch = state["triggers"].get("watch")
    fake("mail", {"count": 3})
    time.sleep(20)
    code, current, _ = http("GET", f"/triggers/{watch}")
    if code != 200:
        return False, f"get {code}"
    current["enabled"] = False
    code, _, _ = http("PUT", f"/triggers/{watch}", current)
    time.sleep(minutes(3, 6))
    current["enabled"] = True
    code2, _, _ = http("PUT", f"/triggers/{watch}", current)
    return code == 200 and code2 == 200, "watch paused mid-run, then resumed"


def fault_burst():
    fake("events", {"count": 150, "start_in_minutes": 12, "spacing_minutes": 0.2})
    return True, "150 meetings at once"


FAULT_KINDS = ["restart", "sleep", "error500", "error429", "hang", "network", "tokens", "revoke",
               "history", "duplicate", "pause", "burst"]


def inject(kind, state):
    if kind == "restart":
        return fault_restart()
    if kind == "sleep":
        return fault_sleep()
    if kind in ("error500", "error429", "hang"):
        return fault_provider(kind)
    if kind == "network":
        return fault_network()
    if kind == "tokens":
        return fault_tokens()
    if kind == "revoke":
        return fault_revoke(state)
    if kind == "history":
        return fault_history()
    if kind == "duplicate":
        return fault_duplicate()
    if kind == "pause":
        return fault_pause_mid_run(state)
    return fault_burst()


# ---- checks ----

def runs_of(trigger):
    code, data, _ = http("GET", "/runs?" + urlencode({"trigger": trigger, "limit": 2000}))
    return data.get("runs", data if isinstance(data, list) else [])


def slot_key(run):
    slot = run.get("slot")
    return json.dumps(slot, sort_keys=True) if slot is not None else None


def checks(state, final=False):
    ensure_up()
    rc, out = sh("vak", "data", "verify", timeout=600)
    check("integrity", rc == 0, out.splitlines()[0] if out else "")
    code, status = health()
    check("server healthy", code == 200 and status != "fenced", f"{code} {status}")
    summary = {}
    for key, trigger in state["triggers"].items():
        if not trigger:
            continue
        runs = runs_of(trigger)
        seen, duplicates = {}, []
        for run in runs:
            if run.get("status") == "skipped" or run.get("attempt", 1) > 1:
                continue
            key_ = slot_key(run)
            if key_ is None:
                continue
            if key_ in seen:
                duplicates.append(key_)
            seen[key_] = run.get("id")
        statuses = {}
        for run in runs:
            statuses[run.get("status")] = statuses.get(run.get("status"), 0) + 1
        stuck = [r for r in runs if r.get("status") == "running"
                 and time.time() - calendar.timegm(time.strptime(r["opened_at"][:19], "%Y-%m-%dT%H:%M:%S")) > 1800]
        check(f"{key}: at most one run per slot", not duplicates, f"{len(runs)} runs {statuses}; duplicates {duplicates[:3]}")
        check(f"{key}: nothing left running", not stuck, f"{len(stuck)} open past 30 min")
        summary[key] = statuses
    stats = fake("stats")
    check("provider stand-in answering", "counts" in stats, json.dumps(stats.get("counts", {}))[:200])
    code, unread, _ = http("GET", f"http://127.0.0.1:9100/_soak/unread?older={int(900 * max(SCALE, 0.25))}", auth=False)
    check("every message older than 15 minutes was read", unread.get("unread") == 0,
          f"{unread.get('unread')} unread of {stats.get('messages')}; {unread.get('fetched')} read; sample {unread.get('sample')}")
    rc, du = sh("du", "-sm", "/data/vak")
    state.setdefault("disk", []).append({"at": time.time(), "mb": du.split()[0] if du else "?"})
    say("disk MB", du.split()[0] if du else "?", "runs", json.dumps(summary))
    return summary


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--hours", type=float, default=24)
    parser.add_argument("--report", default="report.json")
    parser.add_argument("--seed", type=int, default=11)
    parser.add_argument("--scale", type=float, default=1.0,
                        help="multiply every fault's spacing and length (0.25 for a quick shakedown)")
    args = parser.parse_args()
    global SCALE
    SCALE = args.scale
    random.seed(args.seed)
    started = time.time()
    end = started + args.hours * 3600
    write_agent()
    assert serve(), "server did not start"
    ok, why = link_account()
    check("account linked through OAuth against the stand-in", ok, why)
    account = account_id()
    check("the linked account is connected", account is not None, str(account))
    if not account:
        finish(args, started, {})
        return
    fake("mail", {"count": 5})
    fake("events", {"count": 4, "start_in_minutes": 20, "spacing_minutes": 30})
    state = {"account": account, "triggers": routines(account)}
    check("three routines made", all(state["triggers"].values()), json.dumps(state["triggers"]))
    next_fault = time.time() + minutes(10, 20)
    next_check = time.time() + 1800 * max(SCALE, 0.25)
    next_events = time.time() + 1800
    kinds = []
    while time.time() < end:
        if random.random() < 0.4:
            fake("mail", {"count": random.randint(1, 3)})
        if time.time() >= next_events:
            fake("events", {"count": 3, "start_in_minutes": 15, "spacing_minutes": 20})
            next_events = time.time() + 1800
        if time.time() >= next_fault:
            if not kinds:
                kinds = FAULT_KINDS[:]
                random.shuffle(kinds)
            kind = kinds.pop()
            at = time.time()
            say("FAULT", kind)
            try:
                ok, why = inject(kind, state)
            except Exception as error:
                ok, why = False, f"injector error {error!r}"
            ensure_up()
            FAULTS.append({"kind": kind, "at": at, "seconds": round(time.time() - at), "ok": ok, "detail": why})
            say("fault done", kind, ok, why)
            next_fault = time.time() + minutes(10, 20)
        if time.time() >= next_check:
            checks(state)
            next_check = time.time() + 1800 * max(SCALE, 0.25)
        time.sleep(60)
    summary = checks(state, final=True)
    finish(args, started, summary, state)


def finish(args, started, summary, state=None):
    report = {"hours": round((time.time() - started) / 3600, 2), "faults": FAULTS, "checks": CHECKS,
              "runs": summary, "disk": (state or {}).get("disk", []), "provider": fake("stats")}
    with open(args.report, "w") as handle:
        json.dump(report, handle, indent=1)
    bad = [c for c in CHECKS if not c["ok"]]
    say(f"{len(CHECKS) - len(bad)} of {len(CHECKS)} checks passed; {len(FAULTS)} faults")
    for c in bad[:30]:
        say("  FAILED", c["name"], "|", c["detail"])
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
