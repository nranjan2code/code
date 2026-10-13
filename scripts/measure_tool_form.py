#!/usr/bin/env python3
"""Measure whether the tool-call arguments a model writes are valid, on a real route.

Usage:

  capture  a logging proxy in front of Ollama; every JSON POST body is saved
    python3 scripts/measure_tool_form.py capture --listen 127.0.0.1:11435 \\
        --upstream http://127.0.0.1:11434 --out <dir>

  replay   one captured /api/chat body, N times per form
    python3 scripts/measure_tool_form.py replay <captured.json> \\
        --base http://127.0.0.1:11434 --trials 8 [--forms free,schema] \\
        [--tool NAME] [--out results.json]

Forms: free replays the body as captured and validates each tool call's
arguments; schema drops the tools, sets format to one tool's parameters and
validates message.content. Exit code is 0 for any measurement, non-zero only
for bad arguments or an unreadable capture.

Capture on the dev route: point the Ollama provider's base URL at the proxy's
--listen address (http://127.0.0.1:11435), send one turn, and the request
bodies land in --out as NNNN-<endpoint>.json.

Judge a change to how tool arguments are asked for by replaying Vak's own
captured requests, never a toy schema (AGENTS.md, 'Tool arguments are written,
then read'; docs/design/89-constrained-tool-arguments.md §4.6).
"""

import argparse
import http.server
import json
import os
import statistics
import sys
import threading
import time
import urllib.error
import urllib.request

OUTCOMES = ["valid", "invalid", "broken", "unknown_tool", "no_call", "http_error"]
CALL_ORDER = ["unknown_tool", "broken", "invalid", "valid"]


# ---------- capture: logging proxy ----------

class Proxy(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    upstream = ""
    out_dir = ""
    lock = threading.Lock()
    count = 0

    def log_message(self, fmt, *args):
        pass

    def do_GET(self):
        self.relay(None)

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n) if n else b""
        self.save(body)
        self.relay(body or None)

    def save(self, body):
        try:
            doc = json.loads(body)
        except ValueError:
            return
        with Proxy.lock:
            Proxy.count += 1
            number = Proxy.count
        endpoint = self.path.split("?")[0].rstrip("/").rsplit("/", 1)[-1]
        name = f"{number:04d}-{endpoint}.json"
        with open(os.path.join(self.out_dir, name), "w", encoding="utf-8") as f:
            json.dump(doc, f, indent=2, ensure_ascii=False)
        tools = doc.get("tools") if isinstance(doc, dict) else None
        model = doc.get("model") if isinstance(doc, dict) else None
        print(f"{name}  model={model}  tools={len(tools or [])}", flush=True)

    def relay(self, body):
        headers = {k: v for k, v in self.headers.items()
                   if k.lower() in ("content-type", "authorization")}
        req = urllib.request.Request(self.upstream + self.path, data=body,
                                     headers=headers, method=self.command)
        try:
            resp = urllib.request.urlopen(req, timeout=600)
        except urllib.error.HTTPError as e:
            resp = e
        except urllib.error.URLError as e:
            msg = str(e.reason).encode()
            self.send_response(502)
            self.send_header("Content-Length", str(len(msg)))
            self.end_headers()
            self.wfile.write(msg)
            return
        with resp:
            code = getattr(resp, "status", None) or resp.code
            self.send_response(code)
            self.send_header("Content-Type", resp.headers.get("Content-Type", "application/octet-stream"))
            self.send_header("Transfer-Encoding", "chunked")
            self.send_header("Connection", "close")
            self.end_headers()
            read = getattr(resp, "read1", resp.read)
            while True:
                chunk = read(65536)
                if not chunk:
                    break
                self.wfile.write(f"{len(chunk):X}\r\n".encode() + chunk + b"\r\n")
                self.wfile.flush()
            self.wfile.write(b"0\r\n\r\n")
        self.close_connection = True


def cmd_capture(args):
    host, _, port = args.listen.rpartition(":")
    os.makedirs(args.out, exist_ok=True)
    Proxy.upstream = args.upstream.rstrip("/")
    Proxy.out_dir = args.out
    server = http.server.ThreadingHTTPServer((host or "127.0.0.1", int(port)), Proxy)
    print(f"proxy {args.listen} -> {Proxy.upstream}, writing to {args.out}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


# ---------- validation: the JSON Schema subset tool parameters use ----------

def type_ok(value, t):
    if t == "object":
        return isinstance(value, dict)
    if t == "array":
        return isinstance(value, list)
    if t == "string":
        return isinstance(value, str)
    if t == "boolean":
        return isinstance(value, bool)
    if t == "null":
        return value is None
    if t == "integer":
        return (isinstance(value, int) and not isinstance(value, bool)) or (
            isinstance(value, float) and value.is_integer())
    if t == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    return True  # unsupported type keyword: ignored


def validate(value, schema, path=""):
    """Return error strings; an empty list means valid. Paths look like rows[2].cells."""
    errors = []
    if not isinstance(schema, dict):
        return errors
    where = path or "$"
    branches = schema.get("anyOf") or schema.get("oneOf")
    if branches and not any(not validate(value, b, path) for b in branches):
        errors.append(f"{where}: matches none of the alternatives")
    if "enum" in schema and value not in schema["enum"]:
        errors.append(f"{where}: not one of {schema['enum']}")
    types = schema.get("type")
    if types is not None:
        names = types if isinstance(types, list) else [types]
        if not any(type_ok(value, t) for t in names):
            errors.append(f"{where}: expected {'/'.join(names)}, got {type(value).__name__}")
            return errors
    if isinstance(value, dict):
        props = schema.get("properties", {})
        for key in schema.get("required", []):
            if key not in value:
                errors.append(f"{where}: missing required '{key}'")
        for key, sub in props.items():
            if key in value:
                errors += validate(value[key], sub, f"{path}.{key}" if path else key)
        if schema.get("additionalProperties") is False:
            for key in value:
                if key not in props:
                    errors.append(f"{where}: unexpected key '{key}'")
    if isinstance(value, list) and "items" in schema:
        for i, item in enumerate(value):
            errors += validate(item, schema["items"], f"{path}[{i}]")
    return errors


# ---------- replay ----------

def chat(base, body):
    data = json.dumps(dict(body, stream=False)).encode()
    req = urllib.request.Request(base.rstrip("/") + "/api/chat", data=data,
                                 headers={"Content-Type": "application/json"}, method="POST")
    with urllib.request.urlopen(req, timeout=600) as resp:
        return json.loads(resp.read())


def run_free(base, body, schemas, _tool):
    """Returns (outcome, tools_called, errors, note)."""
    msg = chat(base, body).get("message") or {}
    calls = msg.get("tool_calls") or []
    if not calls:
        return "no_call", [], [], (msg.get("content") or "")[:200]
    called, errors, verdicts = [], [], []
    for call in calls:
        fn = call.get("function") or {}
        name = fn.get("name")
        called.append(str(name))
        args = fn.get("arguments")
        if isinstance(args, str):
            try:
                args = json.loads(args)
            except ValueError:
                verdicts.append("broken")
                errors.append(f"{name}: arguments are not JSON")
                continue
        if name not in schemas:
            verdicts.append("unknown_tool")
            errors.append(f"unknown tool {name!r}")
            continue
        errs = validate(args, schemas[name])
        verdicts.append("invalid" if errs else "valid")
        errors += [f"{name}: {e}" for e in errs]
    outcome = next(o for o in CALL_ORDER if o in verdicts)
    return outcome, called, errors, ""


def run_schema(base, body, schemas, tool):
    req = {k: v for k, v in body.items() if k != "tools"}
    req["format"] = schemas[tool]
    content = (chat(base, req).get("message") or {}).get("content") or ""
    try:
        obj = json.loads(content)
    except ValueError:
        return "broken", [], ["content is not JSON"], content[:200]
    errs = validate(obj, schemas[tool])
    return ("invalid" if errs else "valid"), [], errs, ""


RUNNERS = {"free": run_free, "schema": run_schema}


def summarise(records, forms):
    summary = {}
    for form in forms:
        rows = [r for r in records if r["form"] == form]
        counts = {o: sum(1 for r in rows if r["outcome"] == o) for o in OUTCOMES}
        times = [r["elapsed_s"] for r in rows]
        summary[form] = dict(counts, n=len(rows),
                             median_s=round(statistics.median(times), 2) if times else None)
    return summary


def print_summary(summary):
    print()
    header = f"{'form':<8}{'valid/N':>9}" + "".join(f"{o:>14}" for o in OUTCOMES[1:]) + f"{'median_s':>11}"
    print(header)
    for form, s in summary.items():
        row = f"{form:<8}{str(s['valid']) + '/' + str(s['n']):>9}"
        row += "".join(f"{s[o]:>14}" for o in OUTCOMES[1:])
        row += f"{s['median_s']:>11}"
        print(row)


def cmd_replay(args, parser):
    try:
        with open(args.capture_file, encoding="utf-8") as f:
            body = json.load(f)
    except (OSError, ValueError) as e:
        print(f"cannot read capture {args.capture_file}: {e}", file=sys.stderr)
        return 2
    if not isinstance(body, dict):
        print("capture is not a JSON object", file=sys.stderr)
        return 2
    forms = [f.strip() for f in args.forms.split(",") if f.strip()]
    for form in forms:
        if form not in RUNNERS:
            parser.error(f"unknown form {form!r}; use free or schema")
    schemas = {t["function"]["name"]: t["function"].get("parameters", {})
               for t in body.get("tools") or []}
    if "schema" in forms:
        if not args.tool:
            parser.error("the schema form needs --tool NAME")
        if args.tool not in schemas:
            parser.error(f"--tool {args.tool!r} is not among the captured tools: {sorted(schemas)}")

    records = []
    for form in forms:
        for trial in range(1, args.trials + 1):
            start = time.monotonic()
            try:
                outcome, called, errors, note = RUNNERS[form](args.base, body, schemas, args.tool)
            except Exception as e:  # a failed request is a measurement, not a crash
                outcome, called, errors, note = "http_error", [], [f"{type(e).__name__}: {e}"], ""
            elapsed = round(time.monotonic() - start, 2)
            records.append({"form": form, "trial": trial, "outcome": outcome,
                            "tools_called": called, "errors": errors[:3],
                            "note": note, "elapsed_s": elapsed})
            detail = f"  {errors[0]}" if errors else (f"  {note}" if note else "")
            print(f"{form:<7} #{trial:<3}{outcome:<13} tools={','.join(called) or '-'}"
                  f"  {elapsed:7.1f}s{detail}", flush=True)

    summary = summarise(records, forms)
    print_summary(summary)
    if args.out:
        result = {"capture": args.capture_file, "model": body.get("model"),
                  "base": args.base, "trials": records, "summary": summary}
        with open(args.out, "w", encoding="utf-8") as f:
            json.dump(result, f, indent=2, ensure_ascii=False)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description="Measure tool-call argument validity on a real route.")
    sub = parser.add_subparsers(dest="cmd", required=True)

    cap = sub.add_parser("capture", help="logging proxy that saves each JSON request body")
    cap.add_argument("--listen", required=True, help="host:port to listen on, e.g. 127.0.0.1:11435")
    cap.add_argument("--upstream", required=True, help="real Ollama base URL")
    cap.add_argument("--out", required=True, help="directory for captured bodies")

    rep = sub.add_parser("replay", help="replay a captured /api/chat body N times per form")
    rep.add_argument("capture_file", help="captured request body (JSON)")
    rep.add_argument("--base", default="http://127.0.0.1:11434", help="Ollama base URL")
    rep.add_argument("--trials", type=int, default=8)
    rep.add_argument("--forms", default="free,schema", help="comma list of free, schema")
    rep.add_argument("--tool", help="tool name for the schema form")
    rep.add_argument("--out", help="write all trial records and the summary as JSON")

    args = parser.parse_args(argv)
    if args.cmd == "capture":
        return cmd_capture(args)
    return cmd_replay(args, parser)


if __name__ == "__main__":
    sys.exit(main())
