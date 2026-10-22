#!/usr/bin/env python3
"""Capture multi-turn acceptance evidence from a running development server.

Uses its configured route and ordinary authenticated session APIs. This captures
results for semantic review; successful transport is never a semantic pass.
Use a disposable trusted workspace, not a person's active conversation.
"""
import argparse
import json
from pathlib import Path
import time
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--token-file", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, default=Path(__file__).resolve().parents[2] / "docs/audits/fixtures/conversation-continuity-scenarios-2026-10-22.json")
    parser.add_argument("--case", action="append", default=[])
    parser.add_argument("--turn-timeout", type=float, default=300)
    args = parser.parse_args()
    token = args.token_file.read_text().strip()
    corpus = json.loads(args.corpus.read_text())
    cases = [case for case in corpus["cases"] if not args.case or case["id"] in args.case]
    if not cases or set(args.case) - {case["id"] for case in cases}:
        parser.error("case selection does not match the corpus")
    if args.output.exists():
        parser.error("output already exists; preserve earlier evidence and choose a new path")

    def request(path, data=None):
        query = urllib.request.Request(
            args.base_url.rstrip("/") + path,
            headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"},
            data=json.dumps(data).encode() if data is not None else None,
        )
        with urllib.request.urlopen(query, timeout=30) as response:
            return json.load(response)

    report = {"method": corpus["method"], "semantic_review": "pending", "cases": []}

    def save():
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        args.output.chmod(0o600)

    for case in cases:
        session_id = request("/sessions", {})["session_id"]
        path = "/sessions/" + session_id
        result = {"id": case["id"], "session": session_id, "acceptance": case["acceptance"], "turns": []}
        report["cases"].append(result)
        print(case["id"], session_id, flush=True)
        save()
        for number, prompt in enumerate(case["prompts"], 1):
            started = time.monotonic()
            request(path + "/run", {"prompt": prompt})
            while request(path + "/control-state").get("running"):
                if time.monotonic() - started > args.turn_timeout:
                    result["incomplete_turn"] = number
                    save()
                    raise TimeoutError("turn still running; inspect the recorded session before continuing")
                time.sleep(1)
            result["turns"].append({"prompt": prompt, "seconds": round(time.monotonic() - started, 1), "snapshot": request(path + "/snapshot")})
            save()
            print("  settled", number, flush=True)
        result["transcript"] = request(path + "/transcript")
        save()
    print("Capture complete. Review each criterion against narration, forms and actual source receipts; semantic review remains pending.")


if __name__ == "__main__":
    main()
