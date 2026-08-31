#!/usr/bin/env python3
"""Run real prompt-driven VAK scenarios against a selected provider/model.

This is intentionally separate from Rust unit/integration coverage. Each case
creates a fresh workspace, gives VAK a natural-language task, and verifies
observable invariants after the model turn: files changed only in scope, tests
pass, required artifacts exist, and the session ledger contains evidence.
"""

from __future__ import annotations

import argparse
import atexit
import concurrent.futures
import datetime as dt
import hashlib
import json
import os
import pathlib
import re
import signal
import subprocess
import tempfile
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass


ROOT = pathlib.Path(__file__).resolve().parents[1]
BIN = ROOT / "target" / "debug" / "vak"
ACTIVE_PROCESSES: set[subprocess.Popen] = set()
ACTIVE_PROCESSES_LOCK = threading.Lock()


@dataclass(frozen=True)
class Case:
    name: str
    task: str
    files: dict[str, str]
    check: str
    required_output: tuple[str, ...] = ()
    forbidden_files: tuple[str, ...] = ()
    allowed_changes: tuple[str, ...] = ("app.py",)


def case_fingerprint(case: Case) -> str:
    payload = json.dumps({
        "task": case.task,
        "files": case.files,
        "check": case.check,
        "required_output": case.required_output,
        "forbidden_files": case.forbidden_files,
        "allowed_changes": case.allowed_changes,
    }, sort_keys=True)
    return hashlib.sha256(payload.encode()).hexdigest()[:16]


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
    for name, requirement, _implementation, assertion in specs:
        tests = f"from app import transform\n{assertion}\nassert transform([]) == {'' if name in ('longest','join-nonempty') else '[]' if name in ('even-values','squares','unique-sorted','rotate-left','chunks-two') else '0'}\n"
        cases.append(Case(
            f"generated-{name}",
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
    if any(term in text for term in ("rate limit", "quota", "credit", "provider unavailable",
                                    "provider auth missing", "missing credential", "unauthorised",
                                    "unauthorized")):
        return "provider"
    if any(term in text for term in ("postcondition test failed", "out-of-scope file changed")):
        return "postcondition"
    if any(term in text for term in ("skill-read evidence", "required evidence missing")):
        return "contract"
    if returncode != 0 or failures:
        return "agent-run"
    return None


def ledger_tool_calls(ledgers: list[pathlib.Path]) -> list[tuple[str, dict]]:
    """Return actual tool-use blocks from the append-only session records."""
    calls: list[tuple[str, dict]] = []
    for path in ledgers:
        for line in path.read_text().splitlines():
            try:
                entry = json.loads(line)
            except json.JSONDecodeError:
                continue
            if entry.get("kind") != "message":
                continue
            message = entry.get("message", {})
            for block in message.get("content", []):
                if block.get("type") == "tool_use":
                    calls.append((block.get("name", ""), block.get("input", {})))
    return calls


def provider_is_local(provider: str) -> bool:
    return provider.strip().lower() in {"ollama", "vllm", "lmstudio", "lm-studio"}


def snapshot_workspace(root: pathlib.Path) -> dict[str, str]:
    snapshot: dict[str, str] = {}
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        relative = path.relative_to(root)
        if relative.parts[0] == "home" or "__pycache__" in relative.parts or path.suffix == ".pyc":
            continue
        snapshot[str(relative)] = hashlib.sha256(path.read_bytes()).hexdigest()
    return snapshot


def process_group_alive(group: int) -> bool:
    try:
        os.killpg(group, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def process_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def acquire_report_lock(report: pathlib.Path) -> tuple[pathlib.Path, Callable[[], None]]:
    lock = report.with_suffix(report.suffix + ".lock")
    lock.parent.mkdir(parents=True, exist_ok=True)
    try:
        descriptor = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    except FileExistsError:
        try:
            owner = int(json.loads(lock.read_text())["pid"])
        except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
            raise SystemExit(f"report lock is unreadable; refusing concurrent access: {lock}") from error
        if owner > 0 and process_alive(owner):
            raise SystemExit(f"report already has an active writer (pid {owner}): {report}")
        lock.unlink(missing_ok=True)
        descriptor = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(descriptor, "w") as stream:
        json.dump({"pid": os.getpid(), "report": str(report)}, stream)

    def release() -> None:
        try:
            payload = json.loads(lock.read_text())
            if payload.get("pid") == os.getpid():
                lock.unlink(missing_ok=True)
        except (OSError, json.JSONDecodeError):
            pass

    atexit.register(release)
    return lock, release


def write_state(path: pathlib.Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(payload, indent=2) + "\n")
    temporary.replace(path)


def terminate_active_processes() -> None:
    with ACTIVE_PROCESSES_LOCK:
        processes = list(ACTIVE_PROCESSES)
    for process in processes:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass


def interrupt_handler(_signum, _frame) -> None:
    terminate_active_processes()
    raise KeyboardInterrupt


def run_case(case: Case, provider: str, model: str, timeout: int) -> dict:
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix=f"vak-prompt-{case.name}-") as directory:
        root = pathlib.Path(directory)
        for relative, content in case.files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        workspace_before = snapshot_workspace(root)
        env = {**os.environ, "VAK_HOME": str(root / "home"), "VAK_PROVIDER": provider, "VAK_MODEL": model}
        command = [str(BIN), "exec", case.task, "--trust", "--yes", "--max-turns", "24"]
        process = subprocess.Popen(command, cwd=root, env=env, text=True,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        with ACTIVE_PROCESSES_LOCK:
            ACTIVE_PROCESSES.add(process)
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
        finally:
            with ACTIVE_PROCESSES_LOCK:
                ACTIVE_PROCESSES.discard(process)
        failures: list[str] = []
        orphaned_process_group = process_group_alive(process.pid)
        if orphaned_process_group:
            failures.append("agent process group remained alive after parent completion")
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        test = subprocess.run(case.check.split(), cwd=root, text=True, capture_output=True)
        if test.returncode:
            failures.append(f"postcondition test failed: {test.stdout}{test.stderr}")
        for relative in case.forbidden_files:
            original = case.files[relative]
            if (root / relative).read_text() != original:
                failures.append(f"out-of-scope file changed: {relative}")
        workspace_after = snapshot_workspace(root)
        allowed = set(case.allowed_changes)
        changed = sorted(path for path in workspace_before.keys() | workspace_after.keys()
                         if workspace_before.get(path) != workspace_after.get(path))
        unexpected = [path for path in changed if path not in allowed]
        if unexpected:
            failures.append(f"unexpected workspace changes: {', '.join(unexpected)}")
        ledgers = list((root / "home" / "sessions").glob("*/*.jsonl"))
        ledger = "\n".join(path.read_text() for path in ledgers)
        if not ledgers:
            failures.append("no session ledger was written")
        tool_calls = ledger_tool_calls(ledgers)
        skill_reads = [inputs for name, inputs in tool_calls
                       if name == "read" and str(inputs.get("path", "")).endswith("SKILL.md")]
        if not skill_reads:
            failures.append("skill-read tool evidence missing from ledger")
        if not any(name == "bash" and case.check in str(inputs.get("command", ""))
                   for name, inputs in tool_calls):
            failures.append("verification bash tool evidence missing from ledger")
        if "child-review" in case.name and not any(name == "task" for name, _ in tool_calls):
            failures.append("child task tool evidence missing from ledger")
        for phrase in case.required_output:
            if phrase.lower() not in output.lower():
                failures.append(f"required final-output evidence missing: {phrase}")
        return {"name": case.name, "fingerprint": case_fingerprint(case), "command": command, "returncode": returncode,
                "elapsed_seconds": round(time.monotonic() - started, 3),
                "status": "passed" if returncode == 0 and not failures else "failed",
                "failure_class": failure_class(returncode, failures, output),
                "failures": failures, "workspace_changes": changed,
                "orphaned_process_group": orphaned_process_group,
                "tool_calls": [name for name, _ in tool_calls], "output_tail": output[-5000:]}


def write_report(path: pathlib.Path, results: list[dict], provider: str, model: str,
                 expected_total: int, run_status: str, run_id: str) -> None:
    classes = {}
    for result in results:
        category = result.get("failure_class")
        if category:
            classes[category] = classes.get(category, 0) + 1
    summary = {"total": len(results), "expected_total": expected_total,
               "completed": len(results),
               "unique_scenarios": len({r.get("fingerprint", r["name"]) for r in results}),
               "passed": sum(r["status"] == "passed" for r in results),
               "failed": sum(r["status"] == "failed" for r in results),
               "provider": provider, "model": model, "by_failure_class": classes}
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps({"run_id": run_id, "run_status": run_status,
                                     "updated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
                                     "summary": summary, "results": results}, indent=2) + "\n")
    temporary.replace(path)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--provider", default="ollama")
    parser.add_argument("--model", default="gemma4:e2b-mlx")
    parser.add_argument("--timeout", type=int, default=360)
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--workers", type=int, help="parallel cases; local Ollama defaults to one")
    parser.add_argument("--limit", type=int, default=0, help="limit cases; zero runs the full prompt corpus")
    parser.add_argument("--case", help="run only the named case")
    parser.add_argument("--json", type=pathlib.Path, default=ROOT / "target" / "prompt-scenarios.json")
    parser.add_argument("--heartbeat", type=pathlib.Path,
                        help="write a small live run-state file after every completed case")
    parser.add_argument("--resume", action="store_true", help="resume completed case keys from --json")
    args = parser.parse_args()
    if not BIN.exists():
        raise SystemExit(f"missing built binary: {BIN}")
    cases = tuple(case for case in CASES if not args.case or case.name == args.case)
    cases = cases[:args.limit] if args.limit else cases
    if not cases:
        raise SystemExit(f"unknown prompt case: {args.case}")
    workers = args.workers if args.workers else (1 if provider_is_local(args.provider) else 4)
    expected_total = len(cases) * max(1, args.repeat)
    run_id = hashlib.sha256(f"{time.time_ns()}:{args.provider}:{args.model}".encode()).hexdigest()[:16]
    results: list[dict] = []
    completed_keys: set[tuple[int, str]] = set()
    _, release_lock = acquire_report_lock(args.json)
    signal.signal(signal.SIGINT, interrupt_handler)
    signal.signal(signal.SIGTERM, interrupt_handler)
    try:
        if args.resume and args.json.is_file():
            prior = json.loads(args.json.read_text())
            prior_summary = prior.get("summary", {})
            if prior_summary.get("provider") != args.provider or prior_summary.get("model") != args.model:
                raise SystemExit("resume report provider/model does not match this run")
            if prior_summary.get("expected_total") != expected_total:
                raise SystemExit("resume report scenario/repetition count does not match this run")
            results = prior.get("results", [])
            valid = {(iteration, case.name): case_fingerprint(case)
                     for iteration in range(max(1, args.repeat)) for case in cases}
            for result in results:
                key = (int(result.get("iteration", 0)), result["name"])
                if key not in valid or result.get("fingerprint") != valid[key]:
                    raise SystemExit(f"resume report contains a different scenario contract: {key}")
            completed_keys = {(int(r.get("iteration", 0)), r["name"]) for r in results}
            run_id = prior.get("run_id", run_id)
        write_report(args.json, results, args.provider, args.model, expected_total, "running", run_id)
        if args.heartbeat:
            write_state(args.heartbeat, {"run_id": run_id, "status": "running",
                                         "expected": expected_total, "completed": len(results)})
        for repeat in range(max(1, args.repeat)):
            pending = [case for case in cases if (repeat, case.name) not in completed_keys]
            with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, workers)) as pool:
                futures = {
                    pool.submit(run_case, case, args.provider, args.model, args.timeout): case
                    for case in pending
                }
                for future in concurrent.futures.as_completed(futures):
                    case = futures[future]
                    result = future.result()
                    result["iteration"] = repeat
                    results.append(result)
                    write_report(args.json, results, args.provider, args.model, expected_total, "running", run_id)
                    if args.heartbeat:
                        write_state(args.heartbeat, {"run_id": run_id, "status": "running",
                                                     "expected": expected_total, "completed": len(results),
                                                     "last_case": case.name})
                    print(f"{result['status'].upper()} {case.name} ({result['elapsed_seconds']}s)", flush=True)
        final_status = "completed" if len(results) == expected_total else "incomplete"
        write_report(args.json, results, args.provider, args.model, expected_total, final_status, run_id)
        if args.heartbeat:
            write_state(args.heartbeat, {"run_id": run_id, "status": final_status,
                                         "expected": expected_total, "completed": len(results)})
        summary = json.loads(args.json.read_text())["summary"]
        print(json.dumps(summary, sort_keys=True))
        return 0 if final_status == "completed" and summary["failed"] == 0 else 1
    except KeyboardInterrupt:
        write_report(args.json, results, args.provider, args.model, expected_total, "incomplete", run_id)
        if args.heartbeat:
            write_state(args.heartbeat, {"run_id": run_id, "status": "incomplete",
                                         "expected": expected_total, "completed": len(results)})
        return 130
    except Exception:
        write_report(args.json, results, args.provider, args.model, expected_total, "incomplete", run_id)
        if args.heartbeat:
            write_state(args.heartbeat, {"run_id": run_id, "status": "incomplete",
                                         "expected": expected_total, "completed": len(results)})
        raise
    finally:
        release_lock()


if __name__ == "__main__":
    raise SystemExit(main())
