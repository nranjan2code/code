#!/usr/bin/env python3
"""Run real prompt-driven VAK scenarios against a selected provider/model.

This is intentionally separate from Rust unit/integration coverage. Each case
creates a fresh workspace, gives VAK a natural-language task, and verifies
observable invariants after the model turn: files changed only in scope, tests
pass, required artifacts exist, and the session ledger contains evidence.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import pathlib
import re
import signal
import subprocess
import tempfile
import time
from dataclasses import dataclass


ROOT = pathlib.Path(__file__).resolve().parents[1]
BIN = ROOT / "target" / "debug" / "vak"


@dataclass(frozen=True)
class Case:
    name: str
    task: str
    files: dict[str, str]
    check: str
    required_output: tuple[str, ...] = ()
    forbidden_files: tuple[str, ...] = ()


BASE_CASES = (
    Case(
        "skill-code-positive-total",
        "Use the code-task skill. Read the skill, README.md, and tests before editing. Implement the requested function, edit only app.py, run python3 test_app.py, and if it fails diagnose and repair app.py until it passes. Use only tools advertised by VAK; do not ask me for requirements because they are in README.md. Report the exact final test result.",
        {".vak/skills/code-task/SKILL.md": "---\nname: code-task\ndescription: inspect, edit, and verify a small code task\n---\nRead requirements and tests, edit only the implementation, run the specified test command.\n", "README.md": "Implement total_positive: sum only values greater than zero.\n", "app.py": "def total_positive(values):\n    return sum(values)\n", "test_app.py": "from app import total_positive\nassert total_positive([3,-2,5]) == 8\nassert total_positive([-4]) == 0\n"},
        "python3 test_app.py", ("code-task",), ("README.md", "test_app.py"),
    ),
    Case(
        "skill-code-json-normalizer",
        "Use the code-task skill. Inspect README.md and test_app.py. Implement the requirement in app.py only, run python3 test_app.py, and if it fails diagnose and repair app.py until it passes. Use only tools advertised by VAK and summarize the final verification.",
        {".vak/skills/code-task/SKILL.md": "---\nname: code-task\ndescription: implement and test a focused code change\n---\nRead the repository instructions and tests. Keep the change minimal and run the requested test command.\n", "README.md": "Implement normalize_user: accept a mapping and return a new mapping with lowercase email and a stripped name.\n", "app.py": "def normalize_user(user):\n    return user\n", "test_app.py": "from app import normalize_user\nu = normalize_user({'email':' A@EXAMPLE.COM ', 'name':' Ada '})\nassert u == {'email':'a@example.com', 'name':'Ada'}\nassert normalize_user({'email':'B@X.IO','name':'Bob'}) == {'email':'b@x.io','name':'Bob'}\n"},
        "python3 test_app.py", ("code-task",), ("README.md", "test_app.py"),
    ),
    Case(
        "skill-code-csv-summary",
        "Use the code-task skill. Read all instructions and tests. Implement summarize_csv in app.py only, run python3 test_app.py, and if it fails diagnose and repair app.py until it passes. Report the passing result.",
        {".vak/skills/code-task/SKILL.md": "---\nname: code-task\ndescription: solve code tasks with tests and minimal scoped edits\n---\nInspect first. Edit only the implementation file. Run the exact test command.\n", "README.md": "Implement summarize_csv(text): return {'rows': count of data rows, 'total': sum of integer amount column}.\n", "app.py": "def summarize_csv(text):\n    return {'rows': 0, 'total': 0}\n", "test_app.py": "from app import summarize_csv\nassert summarize_csv('name,amount\\na,2\\nb,5\\n') == {'rows':2,'total':7}\nassert summarize_csv('name,amount\\n') == {'rows':0,'total':0}\n"},
        "python3 test_app.py", ("code-task",), ("README.md", "test_app.py"),
    ),
    Case(
        "skill-code-child-review",
        "Use the code-task skill by first reading .vak/skills/code-task/SKILL.md, README.md, and test_app.py. README.md is the complete requirement: implement clamp(value, low, high) in app.py so values below low return low, values above high return high, and values inside the range return unchanged. Edit only app.py, run python3 test_app.py, and repair app.py until it passes. Then use the advertised task tool (not an invented tool name) to ask a readonly child agent to independently review the implementation, test result, and scope constraint. After the child returns, inspect app.py again and run python3 test_app.py as the final action. If that final test fails, repair app.py and rerun it. Do not edit app.py after the final passing test. Include both verification results.",
        {".vak/skills/code-task/SKILL.md": "---\nname: code-task\ndescription: implement, test, and ask a child agent for review\n---\nRead first, edit only the implementation, run tests, and delegate an independent review when requested.\n", ".vak/skills/code-task/README.md": "Implement clamp(value, low, high), returning low or high at the boundaries.\n", "README.md": "Implement clamp(value, low, high), returning low or high at the boundaries.\n", "app.py": "def clamp(value, low, high):\n    return value\n", "test_app.py": "from app import clamp\nassert clamp(5, 0, 3) == 3\nassert clamp(-1, 0, 3) == 0\nassert clamp(2, 0, 3) == 2\n"},
        "python3 test_app.py", ("code-task", "review"), ("README.md", "test_app.py"),
    ),
)


def generated_cases() -> tuple[Case, ...]:
    """Build a deterministic prompt corpus with 100 distinct fixtures."""
    specs = (
        ("positive-sum", "return the sum of values greater than zero", "sum(v for v in values if v > 0)", "assert transform([3,-2,5]) == 8"),
        ("negative-count", "return how many values are below zero", "sum(v < 0 for v in values)", "assert transform([3,-2,5,-1]) == 2"),
        ("max-absolute", "return the value with the greatest absolute magnitude; return 0 for empty input", "max(values, key=abs, default=0)", "assert transform([-9,3,5]) == -9"),
        ("even-values", "return a list containing only even values in original order", "[v for v in values if v % 2 == 0]", "assert transform([3,2,8,5]) == [2,8]"),
        ("squares", "return a list of each value squared", "[v * v for v in values]", "assert transform([-2,3]) == [4,9]"),
        ("unique-sorted", "return sorted unique values", "sorted(set(values))", "assert transform([3,1,3,2,1]) == [1,2,3]"),
        ("longest", "return the longest string, or an empty string for empty input", "max(values, key=len, default='')", "assert transform(['a','abcd','xy']) == 'abcd'"),
        ("join-nonempty", "return non-empty strings joined by a comma", "','.join(v for v in values if v)", "assert transform(['a','','b']) == 'a,b'"),
        ("rotate-left", "return the list rotated left by one position; empty stays empty", "values[1:] + values[:1]", "assert transform([1,2,3]) == [2,3,1]"),
        ("chunks-two", "return consecutive chunks of at most two values", "[values[i:i+2] for i in range(0, len(values), 2)]", "assert transform([1,2,3]) == [[1,2],[3]]"),
    )
    cases: list[Case] = []
    for name, requirement, implementation, assertion in specs:
        for variant in range(10):
            case_name = f"generated-{name}-{variant:02d}"
            tests = f"from app import transform\n{assertion}\nassert transform([]) == {'' if name in ('longest','join-nonempty') else '[]' if name in ('even-values','squares','unique-sorted','rotate-left','chunks-two') else '0'}\n"
            cases.append(Case(
                case_name,
                f"Use the code-task skill. Read README.md and test_app.py. Implement the requirement in app.py only. Run python3 test_app.py; if it fails, repair the implementation until it passes. Do not edit README.md or test_app.py and use only advertised tools. Report the exact final result.",
                {".vak/skills/code-task/SKILL.md": "---\nname: code-task\ndescription: implement a focused change and verify it with tests\n---\nRead requirements and tests, edit only app.py, run the requested command, and repair failures.\n", "README.md": f"Implement transform(values): {requirement}.\n", "app.py": "def transform(values):\n    return None\n", "test_app.py": tests},
                "python3 test_app.py", ("code-task",), ("README.md", "test_app.py"),
            ))
    return tuple(cases)


CASES = BASE_CASES + generated_cases()


def failure_class(returncode: int, failures: list[str], output: str) -> str | None:
    if returncode == 124:
        return "timeout"
    text = " ".join(failures).lower() + " " + output.lower()
    if any(term in text for term in ("rate limit", "quota", "credit", "provider unavailable")):
        return "provider"
    if any(term in text for term in ("postcondition test failed", "out-of-scope file changed")):
        return "postcondition"
    if any(term in text for term in ("skill-read evidence", "required evidence missing")):
        return "contract"
    if returncode != 0 or failures:
        return "agent-run"
    return None


def run_case(case: Case, provider: str, model: str, timeout: int) -> dict:
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix=f"vak-prompt-{case.name}-") as directory:
        root = pathlib.Path(directory)
        for relative, content in case.files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        env = {**os.environ, "VAK_HOME": str(root / "home"), "VAK_PROVIDER": provider, "VAK_MODEL": model}
        command = [str(BIN), "exec", case.task, "--trust", "--yes", "--max-turns", "24"]
        process = subprocess.Popen(command, cwd=root, env=env, text=True,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        try:
            output, _ = process.communicate(timeout=timeout)
            returncode = process.returncode
        except subprocess.TimeoutExpired as error:
            partial = error.stdout or ""
            if isinstance(partial, bytes):
                partial = partial.decode(errors="replace")
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            remainder, _ = process.communicate()
            if isinstance(remainder, bytes):
                remainder = remainder.decode(errors="replace")
            output = partial + remainder + "\nTIMEOUT\n"
            returncode = 124
        failures: list[str] = []
        test = subprocess.run(case.check.split(), cwd=root, text=True, capture_output=True)
        if test.returncode:
            failures.append(f"postcondition test failed: {test.stdout}{test.stderr}")
        for relative in case.forbidden_files:
            original = case.files[relative]
            if (root / relative).read_text() != original:
                failures.append(f"out-of-scope file changed: {relative}")
        ledgers = list((root / "home" / "sessions").glob("*/*.jsonl"))
        ledger = "\n".join(path.read_text() for path in ledgers)
        if not ledgers:
            failures.append("no session ledger was written")
        if "SKILL.md" not in ledger:
            failures.append("skill-read evidence missing from ledger")
        for phrase in case.required_output:
            if phrase.lower() not in output.lower() and phrase.lower() not in ledger.lower():
                failures.append(f"required evidence missing: {phrase}")
        return {"name": case.name, "command": command, "returncode": returncode,
                "elapsed_seconds": round(time.monotonic() - started, 3),
                "status": "passed" if returncode == 0 and not failures else "failed",
                "failure_class": failure_class(returncode, failures, output),
                "failures": failures, "output_tail": output[-5000:]}


def write_report(path: pathlib.Path, results: list[dict], provider: str, model: str) -> None:
    classes = {}
    for result in results:
        category = result.get("failure_class")
        if category:
            classes[category] = classes.get(category, 0) + 1
    summary = {"total": len(results), "passed": sum(r["status"] == "passed" for r in results),
               "failed": sum(r["status"] == "failed" for r in results),
               "provider": provider, "model": model, "by_failure_class": classes}
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps({"summary": summary, "results": results}, indent=2) + "\n")
    temporary.replace(path)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--provider", default="ollama")
    parser.add_argument("--model", default="gemma4:e2b-mlx")
    parser.add_argument("--timeout", type=int, default=360)
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--limit", type=int, default=0, help="limit cases; zero runs the full prompt corpus")
    parser.add_argument("--case", help="run only the named case")
    parser.add_argument("--json", type=pathlib.Path, default=ROOT / "target" / "prompt-scenarios.json")
    args = parser.parse_args()
    if not BIN.exists():
        raise SystemExit(f"missing built binary: {BIN}")
    cases = tuple(case for case in CASES if not args.case or case.name == args.case)
    cases = cases[:args.limit] if args.limit else cases
    if not cases:
        raise SystemExit(f"unknown prompt case: {args.case}")
    results = []
    for repeat in range(max(1, args.repeat)):
        with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.workers)) as pool:
            futures = {
                pool.submit(run_case, case, args.provider, args.model, args.timeout): case
                for case in cases
            }
            for future in concurrent.futures.as_completed(futures):
                case = futures[future]
                result = future.result()
                result["iteration"] = repeat
                results.append(result)
                write_report(args.json, results, args.provider, args.model)
                print(f"{result['status'].upper()} {case.name} ({result['elapsed_seconds']}s)", flush=True)
    write_report(args.json, results, args.provider, args.model)
    summary = json.loads(args.json.read_text())["summary"]
    print(json.dumps(summary, sort_keys=True))
    return 0 if summary["failed"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
