#!/usr/bin/env python3
"""Capability coverage: judge a contract change across everything a turn can reach.

One script, five steps (``all`` runs them in order):

  registry  start a dev server on a fresh home, seed the shipped packages,
            bind one turn and read ``GET /sessions/{id}/interface``: every
            tool definition the turn was bound to, every skill, card pack
            and MCP server in the published capability set. Each becomes a
            *unit* with a digest of its own declaration.
  corpus    keep ``scripts/capability_coverage/cases.json`` in step with the
            registry: a unit whose declaration digest changed or that has no
            cases is listed for generation (``--emit-prompts DIR`` writes the
            prompts; ``--generate PORT`` sends them to a Vak server on a
            hosted route and stores its answers); a unit no longer in the
            registry loses its cases. Cases are reviewed fixtures.
  run       per route, each case live through the server API on a fresh home
            (``--route ollama:qwen3.5:0.8b-mlx``); the Ollama route runs behind
            a capture proxy so each case's first request is kept.
  replay    replay each captured first request ``--trials`` times on Ollama
            (scripts/measure_tool_form.py): the first-call distribution, for
            A/B between two builds' captures (``--label``).
  report    the per-capability table from the receipts and decisions.

Every number is Vak's side, read from typed records: the call the first step
made against the unit that produced the case, each call's ``accept_call``
verdict and the ``Fault`` of each failed call, calls outside the case's path
(detours), the share of a call's new tokens the answer carries, the
corrections by ``Cause`` (``plan`` is a plan-check miss) and how the turn
ended. Nothing here reads the request's or the answer's words to decide
anything, and nothing is a verdict on the model (AGENTS.md "Investigating a
failure", "No word lists, no tuned limits").

Method and first results: docs/audits/capability-coverage-2026-10-16.md.
"""

import argparse
import hashlib
import http.server
import json
import os
import re
import shutil
import signal
import statistics
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import measure_tool_form as mtf  # noqa: E402

CASES = os.path.join(HERE, "capability_coverage", "cases.json")
WORK = os.environ.get("VAK_COVERAGE_DIR", "/tmp/vak-coverage")
BINARY = os.path.join(ROOT, "target", "debug", "vak")
OLLAMA = os.environ.get("OLLAMA_HOST_URL", "http://127.0.0.1:11434")


# ---------- server ----------

class Server:
    """A dev server on its own fresh home, workspace and port."""

    def __init__(self, name, route, port, ollama_base=None, env_keys=()):
        self.name, self.route, self.port = name, route, port
        self.dir = os.path.join(WORK, name)
        self.home = os.path.join(self.dir, "home")
        self.ws = os.path.join(self.dir, "ws")
        self.token = uuid.uuid4().hex
        self.ollama_base = ollama_base
        self.env_keys = env_keys
        self.proc = None

    def start(self):
        shutil.rmtree(self.dir, ignore_errors=True)
        os.makedirs(os.path.join(self.ws, ".vak"))
        os.makedirs(self.home)
        provider, _, model = self.route.partition(":")
        with open(os.path.join(self.ws, ".vak", "config.toml"), "w") as f:
            # The installed home's own settings, so a case runs as an owner's would.
            f.write(f'provider = "{provider}"\nmodel = "{model}"\n'
                    'permission_mode = "workspace-write"\napproval_mode = "auto-approve"\n')
        env = {k: os.environ[k] for k in ("PATH", "HOME", "TMPDIR", "USER", "LANG") if k in os.environ}
        env.update(VAK_HOME=self.home, VAK_GATEWAY_TOKEN=self.token)
        for key in self.env_keys:
            if key in os.environ:
                env[key] = os.environ[key]
        if self.ollama_base:
            env["VAK_OLLAMA_BASE_URL"] = self.ollama_base
        log = open(os.path.join(self.dir, "server.log"), "w")
        self.proc = subprocess.Popen([BINARY, "-C", self.ws, "serve", "--trust", "--port", str(self.port)],
                                     env=env, stdout=log, stderr=log, start_new_session=True)
        for _ in range(120):
            try:
                self.get("/health")
                break
            except (urllib.error.URLError, ConnectionError):
                time.sleep(1)
        else:
            raise SystemExit(f"{self.name}: server did not start, see {self.dir}/server.log")
        self.post("/onboarding/seed", {})

    def stop(self):
        if self.proc and self.proc.poll() is None:
            os.killpg(self.proc.pid, signal.SIGTERM)
            try:
                self.proc.wait(30)
            except subprocess.TimeoutExpired:
                os.killpg(self.proc.pid, signal.SIGKILL)

    def request(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}{path}", data=data, method=method,
                                     headers={"Authorization": f"Bearer {self.token}",
                                              "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=120) as resp:
            raw = resp.read()
        return json.loads(raw) if raw else None

    def get(self, path):
        return self.request("GET", path)

    def post(self, path, body):
        return self.request("POST", path, body)

    def new_session(self):
        return self.post("/sessions", {})["session_id"]

    def running(self, sid):
        listed = self.get("/sessions")
        rows = listed if isinstance(listed, list) else listed.get("sessions", [])
        return any(r["session_id"] == sid and r.get("running") for r in rows)

    def turn(self, sid, text, timeout):
        """Send one turn and wait for it; True when it ended by itself."""
        self.post(f"/sessions/{sid}/run", {"prompt": text, "request_id": str(uuid.uuid4()),
                                           "can_show_questions": False})
        start = time.time()
        time.sleep(1)
        while self.running(sid):
            if time.time() - start > timeout:
                self.post(f"/sessions/{sid}/cancel", {})
                while self.running(sid):
                    time.sleep(1)
                return False
            time.sleep(2)
        return True

    def reset_files(self, files):
        for entry in os.listdir(self.ws):
            if entry == ".vak":
                continue
            path = os.path.join(self.ws, entry)
            shutil.rmtree(path) if os.path.isdir(path) else os.remove(path)
        for rel, content in (files or {}).items():
            path = os.path.normpath(os.path.join(self.ws, rel))
            if not path.startswith(self.ws + os.sep):
                continue
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8") as f:
                f.write(content)


# ---------- capture proxy (Ollama) ----------

class Capture:
    """measure_tool_form's logging proxy, in-process, its folder set per case."""

    def __init__(self, port):
        self.port = port
        mtf.Proxy.upstream = OLLAMA
        mtf.Proxy.out_dir = os.path.join(WORK, "capture-scratch")
        os.makedirs(mtf.Proxy.out_dir, exist_ok=True)
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", port), mtf.Proxy)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    @property
    def base(self):
        return f"http://127.0.0.1:{self.port}"

    def into(self, folder):
        os.makedirs(folder, exist_ok=True)
        mtf.Proxy.out_dir = folder

    def close(self):
        self.server.shutdown()


def first_request(folder):
    """The first chat body of the turn: not a capacity probe, carries tools."""
    for name in sorted(os.listdir(folder)) if os.path.isdir(folder) else []:
        if not name.endswith("-chat.json"):
            continue
        with open(os.path.join(folder, name)) as f:
            text = f.read()
        if "probe-call" in text:
            continue
        body = json.loads(text)
        if body.get("tools"):
            return os.path.join(folder, name)
    return None


# ---------- registry ----------

def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()[:16]


def units_from(interface):
    """Every capability a turn can admit, as units with their own declarations."""
    bound = interface.get("bound") or {}
    schemas = (bound.get("interface") or {}).get("tool_schemas") or []
    deferred = set(bound.get("deferred_tool_names") or [])
    caps = interface.get("capabilities") or []
    tool_names = {t["name"] for t in schemas}
    units = {}
    for tool in schemas:
        decl = {"name": tool["name"], "description": tool.get("description", ""),
                "schema": tool.get("input_schema") or {}}
        units[f"tool:{tool['name']}"] = dict(kind="tool", declaration=decl,
                                             loaded=tool["name"] not in deferred,
                                             owner={"call": tool["name"]})
    for cap in caps:
        kind, name = cap["id"]["kind"], cap["id"]["name"]
        serves = cap.get("serves")
        serves = serves.get("declared") if isinstance(serves, dict) else []
        if kind == "tool":
            if f"tool:{name}" in units:
                units[f"tool:{name}"]["declaration"]["serves"] = serves
        elif kind == "skill":
            decl = {"name": name, "summary": cap.get("summary", ""), "serves": serves}
            if name in tool_names:
                # A skill with a program is called as its tool (doc 93).
                units[f"tool:{name}"]["declaration"]["skill_summary"] = decl["summary"]
                continue
            units[f"skill:{name}"] = dict(kind="skill", declaration=decl, loaded=False,
                                          owner={"call": "skill", "args": {"name": name}})
        elif kind == "presentation":
            conf = cap.get("configuration") or {}
            kinds = conf.get("kinds") or []
            block_tool = next((t["name"] for t in schemas
                               if set(kinds) & set(((t.get("input_schema") or {}).get("properties", {})
                                                    .get("semantic_type", {}) or {}).get("enum", []))), None)
            decl = {"name": name, "summary": cap.get("summary", ""), "block": conf.get("block"),
                    "kinds": kinds, "drawn_by": block_tool}
            units[f"pack:{name}"] = dict(kind="pack", declaration=decl, loaded=True,
                                         owner={"call": block_tool, "args": {"semantic_type": kinds}})
        elif kind == "mcp_server" or kind == "mcpserver" or kind == "mcp":
            conf = cap.get("configuration") or {}
            for tool in conf.get("tools") or conf.get("catalog") or []:
                tname = tool.get("name") if isinstance(tool, dict) else str(tool)
                decl = {"server": name, "tool": tool}
                units[f"mcp:{name}/{tname}"] = dict(kind="mcp", declaration=decl, loaded=False,
                                                    owner={"call": "mcp", "args": {"server": name, "tool": tname}})
    loader = units.get("tool:skill", {}).get("loaded", False)
    for unit in units.values():
        if unit["kind"] == "skill":
            # Reached through the skill loader, so offered when it is.
            unit["loaded"] = loader
        unit["digest"] = digest(unit["declaration"])
    admitted = {f"{c['id']['kind']}:{c['id']['name']}" for c in caps}
    not_bound = sorted(i for i in admitted if i.startswith("tool:") and i not in units)
    return units, not_bound


def cmd_registry(args):
    server = Server("registry", args.route[0], args.port)
    try:
        server.start()
        sid = server.new_session()
        server.turn(sid, "hi", args.turn_timeout)
        interface = server.get(f"/sessions/{sid}/interface")
    finally:
        server.stop()
    units, not_bound = units_from(interface)
    os.makedirs(WORK, exist_ok=True)
    out = {"units": units, "not_bound": not_bound,
           "system_prompt": ((interface.get("bound") or {}).get("interface") or {}).get("system_prompt", "")}
    with open(os.path.join(WORK, "registry.json"), "w") as f:
        json.dump(out, f, indent=1)
    print(f"{len(units)} units; in the set but not bound to an everyday turn: {not_bound or 'none'}")
    return out


def load_registry():
    with open(os.path.join(WORK, "registry.json")) as f:
        return json.load(f)


# ---------- corpus ----------

GENERATOR = """You write test requests for an assistant platform. Below is ONE capability's
own declaration (JSON), then the one-line catalogue of every capability the
assistant has. Write requests a person might really type for which this
capability is the right thing to use.

Target unit: {unit}
Declaration:
{declaration}

Catalogue (unit id - what it declares):
{catalogue}

Return only JSON, no prose:
{{"single": [CASE, CASE], "followup": CASE}}
CASE = {{"turns": [{{"text": "...", "path": ["unit id", ...]}}, ...],
         "files": {{"relative/path": "text content"}}}}
- "single": two one-turn cases, different wording and subject; the first
  unit in "path" is what the first step should use, and the path ends with
  {unit}. Put earlier units in the path only when the target needs what
  they produce first (for example a file written or a command run).
- "followup": two turns; turn 1 uses {unit}, turn 2 needs turn 1's result
  (its path lists what turn 2 should use, possibly nothing).
- "files": text files the requests need in the workspace, else {{}}.
- Plain everyday wording; never name the unit or a tool.
"""

PAIR = """You write test requests for an assistant platform. Write ONE request a
person might type that needs BOTH capabilities below in the same turn.

{a}: {da}
{b}: {db}

Return only JSON: {{"turns": [{{"text": "...", "path": ["{a}", "{b}"]}}], "files": {{}}}}
(order the path as the work would go; files are text files the request needs).
Plain everyday wording; never name a tool.
"""


def catalogue(units):
    lines = []
    for uid, unit in sorted(units.items()):
        decl = unit["declaration"]
        text = decl.get("description") or decl.get("summary") or json.dumps(decl)[:160]
        lines.append(f"{uid} - {first_sentence(text)}")
    return "\n".join(lines)


def first_sentence(text):
    text = text.strip()
    i = text.find(". ")
    return text[: i + 1] if i >= 0 else text.splitlines()[0] if text else ""


def pairs_for(units):
    """Each unit once, with the unit half the sorted list away: deterministic, no choice by name."""
    ids = sorted(units)
    half = len(ids) // 2
    return [(ids[i], ids[i + half]) for i in range(half)]


def load_cases():
    if not os.path.exists(CASES):
        return {"units": {}, "pairs": {}}
    with open(CASES) as f:
        return json.load(f)


def save_cases(cases):
    os.makedirs(os.path.dirname(CASES), exist_ok=True)
    with open(CASES, "w") as f:
        json.dump(cases, f, indent=1, ensure_ascii=False, sort_keys=True)
        f.write("\n")


def stale(registry, cases):
    units = registry["units"]
    need = [uid for uid, unit in sorted(units.items())
            if cases["units"].get(uid, {}).get("digest") != unit["digest"]]
    need_pairs = []
    for a, b in pairs_for(units):
        key = f"{a}+{b}"
        pair_digest = digest([units[a]["digest"], units[b]["digest"]])
        if cases["pairs"].get(key, {}).get("digest") != pair_digest:
            need_pairs.append((a, b, key, pair_digest))
    gone = sorted(set(cases["units"]) - set(units))
    return need, need_pairs, gone


def prompts_for(registry, need, need_pairs):
    units = registry["units"]
    cat = catalogue(units)
    out = {}
    for uid in need:
        out[uid] = GENERATOR.format(unit=uid, declaration=json.dumps(units[uid]["declaration"], indent=1),
                                    catalogue=cat)
    for a, b, key, _ in need_pairs:
        da = json.dumps(units[a]["declaration"])[:1200]
        db = json.dumps(units[b]["declaration"])[:1200]
        out[key] = PAIR.format(a=a, b=b, da=da, db=db)
    return out


def json_from(text):
    match = re.search(r"\{.*\}", text or "", re.S)
    return json.loads(match.group(0)) if match else None


def generate_via(port, token, prompts, timeout):
    """Each prompt as one turn on a Vak server whose route is hosted."""
    server = Server.__new__(Server)
    server.port, server.token = port, token
    answers = {}
    for key, prompt in prompts.items():
        sid = server.new_session()
        server.turn(sid, prompt, timeout)
        msgs = server.get(f"/sessions/{sid}/transcript").get("messages", [])
        text = next((b.get("text") for m in reversed(msgs) if m["role"] == "assistant"
                     for b in m["content"] if b["type"] == "text"), "")
        try:
            answers[key] = json_from(text)
        except ValueError:
            answers[key] = None
        print(f"  generated {key}: {'ok' if answers[key] else 'unreadable'}", flush=True)
    return answers


def cmd_corpus(args):
    registry = load_registry()
    cases = load_cases()
    need, need_pairs, gone = stale(registry, cases)
    for uid in gone:
        del cases["units"][uid]
    live_pairs = {f"{a}+{b}" for a, b in pairs_for(registry["units"])}
    for key in list(cases["pairs"]):
        if key not in live_pairs:
            del cases["pairs"][key]
    print(f"stale or new units: {len(need)}; pairs: {len(need_pairs)}; removed: {gone or 'none'}")
    prompts = prompts_for(registry, need, need_pairs)
    if args.emit_prompts:
        os.makedirs(args.emit_prompts, exist_ok=True)
        for key, prompt in prompts.items():
            with open(os.path.join(args.emit_prompts, key.replace("/", "_") + ".txt"), "w") as f:
                f.write(prompt)
        print(f"prompts in {args.emit_prompts}")
    answers = {}
    if args.answers:
        with open(args.answers) as f:
            answers = json.load(f)
    elif args.generate:
        answers = generate_via(args.generate, os.environ["VAK_GATEWAY_TOKEN"], prompts, args.turn_timeout)
    units = registry["units"]
    for uid in need:
        got = answers.get(uid)
        if got:
            cases["units"][uid] = {"digest": units[uid]["digest"], "generator": args.generator_label,
                                   "reviewed": False, **got}
    for a, b, key, pair_digest in need_pairs:
        got = answers.get(key)
        if got:
            cases["pairs"][key] = {"digest": pair_digest, "generator": args.generator_label,
                                   "reviewed": False, **got}
    save_cases(cases)
    need, need_pairs, _ = stale(registry, cases)
    print(f"after: {len(need)} units and {len(need_pairs)} pairs still without current cases")
    return 0 if not need and not need_pairs else 1


# ---------- run ----------

def case_list(cases, registry, only=None, singles=None):
    out = []
    for uid, entry in sorted(cases["units"].items()):
        if uid not in registry["units"] or (only and uid not in only):
            continue
        for i, case in enumerate(entry.get("single", [])[:singles]):
            out.append(dict(id=f"{uid}#s{i}", unit=uid, form="single", **case))
        if entry.get("followup"):
            out.append(dict(id=f"{uid}#f", unit=uid, form="followup", **entry["followup"]))
    for key, entry in sorted(cases["pairs"].items()):
        a, b = key.split("+", 1)
        if a in registry["units"] and b in registry["units"] and (not only or a in only or b in only):
            out.append(dict(id=key, unit=key, form="pair", turns=entry["turns"], files=entry.get("files", {})))
    return out


def route_label(route):
    return re.sub(r"[^A-Za-z0-9.]+", "-", route)


def cmd_run(args):
    registry = load_registry()
    cases = case_list(load_cases(), registry, set(args.only) if args.only else None, args.singles)
    for n, route in enumerate(args.route):
        label = f"{args.label}-{route_label(route)}"
        out_dir = os.path.join(WORK, "runs", label)
        os.makedirs(out_dir, exist_ok=True)
        provider = route.split(":", 1)[0]
        capture = Capture(args.capture_port + n) if provider == "ollama" else None
        server = Server(f"serve-{route_label(route)}", route, args.port + n,
                        ollama_base=capture.base if capture else None,
                        env_keys=("OPENAI_API_KEY", "ANTHROPIC_API_KEY", "GEMINI_API_KEY", "OPENROUTER_API_KEY"))
        if provider != "ollama" and not any(k in os.environ for k in server.env_keys):
            print(f"{route}: no provider key in this process's environment; route skipped")
            continue
        server.start()
        try:
            for case in cases:
                done = os.path.join(out_dir, case["id"].replace("/", "_") + ".json")
                if os.path.exists(done) and not args.again:
                    continue
                server.reset_files(case.get("files"))
                sid = server.new_session()
                ended = []
                for t, turn in enumerate(case["turns"]):
                    if capture:
                        capture.into(os.path.join(out_dir, "capture", case["id"].replace("/", "_"), f"t{t}"))
                    ended.append(server.turn(sid, turn["text"], args.turn_timeout))
                record = dict(case=case, route=route, session=sid, ended_by_itself=ended,
                              decisions=server.get(f"/sessions/{sid}/decisions"),
                              transcript=server.get(f"/sessions/{sid}/transcript"))
                with open(done, "w") as f:
                    json.dump(record, f)
                score = score_case(record, registry)
                print(f"{route} {case['id']}: first={score['turns'][-1]['first']} "
                      f"reached={score['reached']} refused={score['refused']} end={score['turns'][-1]['end']}",
                      flush=True)
        finally:
            server.stop()
            if capture:
                capture.close()
    return 0


# ---------- scoring ----------

TOKEN = re.compile(r"[\w][\w.@:/-]*\w|\w", re.U)


def tokens(text):
    return {t.lower() for t in TOKEN.findall(text or "")}


def matches(unit, call, registry):
    """Whether a call is the unit's owner call, from the unit's declaration alone."""
    if unit not in registry["units"]:
        return False
    owner = registry["units"][unit]["owner"]
    if call["name"] != owner["call"]:
        return False
    for key, want in (owner.get("args") or {}).items():
        got = (call.get("input") or {}).get(key)
        if isinstance(want, list):
            if got not in want:
                return False
        elif got != want:
            return False
    return True


def split_turns(transcript):
    msgs = transcript.get("messages", [])
    starts = [i for i, m in enumerate(msgs) if m["role"] == "user"
              and any(b["type"] == "text" for b in m["content"])
              and not any(b["type"] == "tool_result" for b in m["content"])]
    return [msgs[s:(starts[k + 1] if k + 1 < len(starts) else len(msgs))] for k, s in enumerate(starts)]


def result_text(block):
    content = block.get("content")
    if isinstance(content, list):
        return " ".join(c.get("text", "") for c in content if isinstance(c, dict))
    return str(content or "")


def score_case(record, registry):
    case = record["case"]
    turn_ids = []
    for d in record["decisions"]:
        if d.get("turn") and d["turn"] not in turn_ids:
            turn_ids.append(d["turn"])
    verdict, fault = {}, {}
    corrections = {}
    ends = {}
    for d in record["decisions"]:
        choice = d.get("choice") or {}
        if "failed" in d:
            fault[d["failed"]["call"]] = d["failed"]["fault"]
        point = choice.get("point")
        if point == "accept_call":
            for ev in d.get("evidence") or []:
                if ev.get("kind") == "call":
                    verdict[ev["id"]] = choice.get("by") or choice.get("verdict")
        elif point == "correct":
            corrections.setdefault(d.get("turn"), []).append(choice.get("cause"))
        elif point == "end_turn":
            ends[d.get("turn")] = choice.get("end")
    system = tokens(registry.get("system_prompt", ""))
    turns = split_turns(record["transcript"])
    out_turns, owner_results = [], []
    reached_units = set()
    for k, msgs in enumerate(turns):
        spec = case["turns"][k] if k < len(case["turns"]) else {"path": []}
        path = spec.get("path") or []
        calls, results = [], {}
        for m in msgs:
            for b in m["content"]:
                if b["type"] == "tool_use":
                    calls.append({"id": b.get("id"), "name": b.get("name"), "input": b.get("input")})
                elif b["type"] == "tool_result":
                    results[b.get("tool_use_id")] = (result_text(b), bool(b.get("is_error")))
        answer = " ".join(b.get("text", "") for m in msgs if m["role"] == "assistant"
                          for b in m["content"] if b["type"] == "text")
        for call in calls:
            call["verdict"] = verdict.get(call["id"], "unrecorded")
            call["fault"] = fault.get(call["id"])
            # The loop answers its own tools (`recall`, `plan`) without an accept_call record.
            call["ok"] = call["verdict"] in ("accepted", "unrecorded") and not call["fault"] and \
                not results.get(call["id"], ("", True))[1]
            call["serves"] = [u for u in path if matches(u, call, registry)]
            if call["ok"]:
                reached_units.update(call["serves"])
        expected_first = path[0] if path else None
        # `plan` declares the deliverables; the first call that does work is the one judged.
        first = next((c for c in calls if c["name"] != "plan" or expected_first == "tool:plan"), None)
        first_ok = (first is None and expected_first is None) or \
            (first is not None and expected_first is not None and matches(expected_first, first, registry))
        detours = [c["name"] for c in calls if not c["serves"] and c["name"] != "plan"]
        # what the answer carries of the owner's result, as new tokens
        later_text = answer + " " + " ".join(json.dumps(c["input"]) for c in calls)
        request = tokens(spec.get("text", ""))
        used = []
        sources = [c for c in calls if c["serves"] and c["ok"] and c["serves"][-1] == path[-1]] if path else []
        prior = owner_results if case["form"] == "followup" and k > 0 else []
        for text in [results.get(c["id"], ("", False))[0] for c in sources] + prior:
            new = tokens(text) - request - system
            if new:
                used.append(len(new & tokens(later_text)) / len(new))
        if case["form"] == "followup" and k == 0:
            owner_results = [results.get(c["id"], ("", False))[0] for c in calls if c["ok"] and c["serves"]]
        tid = turn_ids[k] if k < len(turn_ids) else None
        out_turns.append(dict(path=path, calls=[(c["name"], c["verdict"], c["fault"]) for c in calls],
                              first=first["name"] if first else None, first_ok=first_ok,
                              detours=detours, used=max(used) if used else None,
                              corrections=corrections.get(tid, []), end=ends.get(tid, "none"),
                              ended_by_itself=(record.get("ended_by_itself") or [True])[k]
                              if k < len(record.get("ended_by_itself") or []) else None))
    wanted = set(u for t in case["turns"] for u in (t.get("path") or []))
    refused = sum(1 for t in out_turns for c in t["calls"] if c[1] not in ("accepted", "unrecorded"))
    return dict(id=case["id"], unit=case["unit"], form=case["form"], route=record["route"],
                turns=out_turns, reached=wanted <= reached_units, wanted=sorted(wanted),
                reached_units=sorted(reached_units), refused=refused)


# ---------- replay ----------

def cmd_replay(args):
    registry = load_registry()
    out = {}
    root = os.path.join(WORK, "runs", f"{args.label}-{route_label(args.route[0])}")
    cases = {c["id"].replace("/", "_"): c for c in case_list(load_cases(), registry)}
    for name in sorted(os.listdir(os.path.join(root, "capture"))):
        case = cases.get(name)
        if not case or case["form"] == "followup":
            continue
        body_path = first_request(os.path.join(root, "capture", name, "t0"))
        if not body_path:
            continue
        with open(body_path) as f:
            body = json.load(f)
        expected = (case["turns"][0].get("path") or [None])[0]
        firsts = []
        for _ in range(args.trials):
            msg = (mtf.chat(OLLAMA, body).get("message") or {})
            call = next((c for c in msg.get("tool_calls") or []
                         if (c.get("function") or {}).get("name") != "plan" or expected == "tool:plan"), None)
            if not call:
                firsts.append((None, False))
                continue
            fn = call.get("function") or {}
            inp = fn.get("arguments")
            if isinstance(inp, str):
                try:
                    inp = json.loads(inp)
                except ValueError:
                    inp = {}
            firsts.append((fn.get("name"), bool(expected) and matches(expected, {"name": fn.get("name"), "input": inp}, registry)))
        out[case["id"]] = dict(unit=case["unit"], expected=expected, firsts=firsts)
        hits = sum(1 for _, ok in firsts if ok)
        print(f"{case['id']}: {hits}/{len(firsts)} first calls reach {expected}", flush=True)
    with open(os.path.join(root, "replay.json"), "w") as f:
        json.dump(out, f, indent=1)
    return 0


# ---------- report ----------

def cmd_report(args):
    registry = load_registry()
    by_route = {}
    for route in args.route:
        root = os.path.join(WORK, "runs", f"{args.label}-{route_label(route)}")
        if not os.path.isdir(root):
            continue
        scores = []
        for name in sorted(os.listdir(root)):
            if name.endswith(".json") and name != "replay.json":
                with open(os.path.join(root, name)) as f:
                    scores.append(score_case(json.load(f), registry))
        replay = {}
        if os.path.exists(os.path.join(root, "replay.json")):
            with open(os.path.join(root, "replay.json")) as f:
                replay = json.load(f)
        by_route[route] = (scores, replay)
    rows, findings = [], []
    units = sorted(registry["units"])
    for uid in units:
        cells, failed_everywhere = [], bool(by_route)
        for route, (scores, replay) in by_route.items():
            mine = [s for s in scores if s["unit"] == uid]
            if not mine:
                cells.append("—")
                failed_everywhere = False
                continue
            first = sum(1 for s in mine if s["turns"][-1]["first_ok"])
            reached = sum(1 for s in mine if s["reached"])
            refused = sum(s["refused"] for s in mine)
            faults = sorted({c[2] for s in mine for t in s["turns"] for c in t["calls"] if c[2]})
            detours = sum(len(t["detours"]) for s in mine for t in s["turns"])
            used = [t["used"] for s in mine for t in s["turns"] if t["used"] is not None]
            plan = sum(t["corrections"].count("plan") for s in mine for t in s["turns"])
            rep = [v for k, v in replay.items() if v["unit"] == uid]
            rep_hits = sum(1 for v in rep for _, ok in v["firsts"] if ok)
            rep_all = sum(len(v["firsts"]) for v in rep)
            cell = (f"{first}/{len(mine)} · {reached}/{len(mine)}"
                    + (f" · replay {rep_hits}/{rep_all}" if rep_all else "")
                    + (f" · refused {refused}" if refused else "")
                    + (f" · {','.join(faults)}" if faults else "")
                    + (f" · detours {detours}" if detours else "")
                    + (f" · used {statistics.median(used):.2f}" if used else "")
                    + (f" · plan {plan}" if plan else ""))
            cells.append(cell)
            if reached:
                failed_everywhere = False
        loaded = "loaded" if registry["units"][uid]["loaded"] else "deferred"
        rows.append(f"| `{uid}` | {loaded} | " + " | ".join(cells) + " |")
        if failed_everywhere:
            findings.append(uid)
    pair_rows = []
    for route, (scores, _) in by_route.items():
        for form in ("pair", "followup"):
            mine = [s for s in scores if s["form"] == form]
            if mine:
                pair_rows.append(f"| {route} | {form} | {len(mine)} | "
                                 f"{sum(1 for s in mine if s['reached'])} | "
                                 f"{sum(1 for s in mine if s['turns'][-1]['first_ok'])} | "
                                 f"{sum(s['refused'] for s in mine)} |")
    header = "| unit | at start | " + " | ".join(by_route) + " |\n|---|---|" + "---|" * len(by_route)
    text = header + "\n" + "\n".join(rows)
    text += "\n\n| route | form | cases | path reached | first call | refused |\n|---|---|---|---|---|---|\n"
    text += "\n".join(pair_rows)
    text += "\n\nNot reached on any route: " + (", ".join(f"`{u}`" for u in findings) or "none") + "\n"
    out = os.path.join(WORK, f"report-{args.label}.md")
    with open(out, "w") as f:
        f.write(text)
    all_scores = {route: s for route, (s, _) in by_route.items()}
    with open(os.path.join(WORK, f"scores-{args.label}.json"), "w") as f:
        json.dump(all_scores, f, indent=1)
    print(text)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description="Domain-neutral capability coverage for Vakyartha.")
    parser.add_argument("step", choices=["registry", "corpus", "run", "replay", "report", "all"])
    parser.add_argument("--route", action="append", default=None,
                        help="provider:model, repeatable (default ollama:qwen3.5:0.8b-mlx)")
    parser.add_argument("--label", default="base", help="names this build's runs, for A/B")
    parser.add_argument("--port", type=int, default=8950)
    parser.add_argument("--capture-port", type=int, default=11440)
    parser.add_argument("--turn-timeout", type=int, default=300,
                        help="the harness's own wait per turn, never Vak's")
    parser.add_argument("--trials", type=int, default=8)
    parser.add_argument("--only", action="append", help="limit to these unit ids")
    parser.add_argument("--singles", type=int, help="run only the first N single-turn cases of each unit")
    parser.add_argument("--again", action="store_true", help="rerun cases already recorded")
    parser.add_argument("--emit-prompts", help="corpus: write generation prompts here")
    parser.add_argument("--answers", help="corpus: a JSON map of prompt key to generated case")
    parser.add_argument("--generate", type=int, help="corpus: port of a Vak server on a hosted route")
    parser.add_argument("--generator-label", default="unspecified")
    args = parser.parse_args(argv)
    args.route = args.route or ["ollama:qwen3.5:0.8b-mlx"]
    steps = {"registry": cmd_registry, "corpus": cmd_corpus, "run": cmd_run,
             "replay": cmd_replay, "report": cmd_report}
    if args.step != "all":
        result = steps[args.step](args)
        return result if isinstance(result, int) else 0
    cmd_registry(args)
    if cmd_corpus(args) != 0 and not args.answers and not args.generate:
        print("cases are missing for some units; generate them first (corpus --generate PORT)")
    cmd_run(args)
    if args.route[0].startswith("ollama:"):
        cmd_replay(args)
    return cmd_report(args)


if __name__ == "__main__":
    sys.exit(main())
