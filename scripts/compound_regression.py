#!/usr/bin/env python3
"""Repeatable compound regression driver for VAK's real execution surfaces.

The default lane is offline and deterministic. ``--live`` adds bounded runs
against the selected provider in isolated workspaces. Every case records its
command, duration, exit status, and a short output tail; any failed case exits
non-zero.
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
import sys
import tempfile
import time
from dataclasses import dataclass

from prompt_scenarios import ledger_tool_calls, provider_is_local


ROOT = pathlib.Path(__file__).resolve().parents[1]
BIN = ROOT / "target" / "debug" / "vak"


def global_skills(explicit: str | None = None) -> pathlib.Path:
    if explicit:
        return pathlib.Path(explicit)
    if configured := os.environ.get("VAK_HOME"):
        return pathlib.Path(configured) / "skills"
    if sys.platform == "darwin":
        return pathlib.Path.home() / "Library" / "Application Support" / "vak" / "skills"
    data = pathlib.Path(os.environ.get("XDG_DATA_HOME", pathlib.Path.home() / ".local" / "share"))
    return data / "vak" / "skills"


def run(name: str, command: list[str], *, cwd: pathlib.Path = ROOT,
        env: dict[str, str] | None = None, timeout: int = 180,
        input_text: str | None = None) -> Result:
    started = time.monotonic()
    process = subprocess.Popen(command, cwd=cwd, env=env, text=True,
                               stdin=subprocess.PIPE if input_text is not None else None,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               start_new_session=True)
    try:
        output, _ = process.communicate(input=input_text, timeout=timeout)
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
    return Result(name, command, time.monotonic() - started, returncode, output[-2000:])


def assert_result(result: Result, needle: str | None = None) -> Result:
    if result.returncode != 0:
        raise RuntimeError(f"{result.name} failed ({result.returncode}):\n{result.output}")
    if needle and needle not in result.output:
        raise RuntimeError(f"{result.name} missing {needle!r}:\n{result.output}")
    return result


def run_offline_matrix(results: list[Result], repeat: int, skills_root: str | None) -> None:
    global_validation = [str(BIN), "skills", "validate", str(global_skills(skills_root)), "--json"]
    for index in range(repeat):
        results.append(assert_result(run(f"skills-validate-{index}", global_validation)))

    results.append(assert_result(run("deterministic-eval", [str(BIN), "eval"]), "passed"))

    for index in range(repeat):
        results.append(assert_result(run(f"agent-hooks-mcp-{index}", [
            "cargo", "test", "-q", "-p", "vak-agent", "--test", "hooks", "--test", "workers",
            "-p", "vak-hooks", "--test", "hooks", "-p", "vak-mcp", "--test", "mcp_roundtrip",
            "--", "--test-threads=8",
        ])))

    results.append(assert_result(run("plugin-lifecycle", ["cargo", "test", "-q", "-p", "vak-plugin"])))
    results.append(assert_result(run("plugin-runtime", [
        "cargo", "test", "-q", "-p", "vak-core", "plugin_runtime_tests",
    ])))
    results.append(memory_lifecycle())
    results.append(assert_result(run("schedules-events-memory", [
        "cargo", "test", "-q", "-p", "vak-server", "--test", "scheduler_personal_os",
        "--test", "heartbeat_personal_os", "--test", "learning_endpoints", "--test", "mcp_endpoints",
    ])))
    results.append(assert_result(run("scenario-matrix", [
        "bash", str(ROOT / "scripts/scenarios/run_all.sh"),
    ], env={**os.environ, "BIN": str(BIN)})))


def memory_lifecycle() -> Result:
    with tempfile.TemporaryDirectory(prefix="vak-memory-regression-") as directory:
        root = pathlib.Path(directory)
        env = {**os.environ, "VAK_HOME": str(root / "home")}
        add = assert_result(run(
            "memory-add",
            [str(BIN), "memory", "add", "Stable regression fact: use python3 for tests", "--kind", "fact", "--tag", "regression"],
            cwd=root,
            env=env,
        ), "added note")
        listed = assert_result(run("memory-list", [str(BIN), "memory", "list"], cwd=root, env=env), "Stable regression fact")
        match = re.search(r"^([0-9a-f]{16})\s", listed.output, re.MULTILINE)
        if not match:
            raise RuntimeError(f"memory list did not expose a usable id: {listed.output}")
        amended = assert_result(run(
            "memory-amend",
            [str(BIN), "memory", "amend", match.group(1), "Stable regression fact: use python3 and preserve provenance"],
            cwd=root,
            env=env,
        ), "amended")
        assert_result(run("memory-list-after-amend", [str(BIN), "memory", "list"], cwd=root, env=env), "preserve provenance")
        forgotten = assert_result(run(
            "memory-forget", [str(BIN), "memory", "forget", match.group(1)], cwd=root, env=env
        ), "forgot")
        assert_result(run("memory-list-after-forget", [str(BIN), "memory", "list"], cwd=root, env=env), "no memory notes in workspace")
        elapsed = add.elapsed + listed.elapsed + amended.elapsed + forgotten.elapsed
        return Result("memory-lifecycle", forgotten.command, elapsed, 0, "add/list/amend/forget preserved provenance")


def live_code_case(provider: str, model: str) -> Result:
    with tempfile.TemporaryDirectory(prefix="vak-compound-live-") as directory:
        root = pathlib.Path(directory)
        (root / ".vak/skills/code-task").mkdir(parents=True)
        (root / ".vak/skills/code-task/SKILL.md").write_text(
            "---\nname: code-task\n"
            "description: Implement a small code change by inspecting requirements, editing only the implementation, and running tests.\n"
            "---\n\nRead the README and tests first. Edit only app.py. Run tests with python3 test_app.py.\n"
        )
        (root / ".vak/config.toml").write_text('permission_mode = "full-access"\n')
        (root / "README.md").write_text("Implement total_positive: sum values greater than zero.\n")
        (root / "app.py").write_text("def total_positive(values):\n    return sum(values)\n")
        (root / "test_app.py").write_text(
            "from app import total_positive\n"
            "assert total_positive([3, -2, 5]) == 8\n"
            "assert total_positive([-4, -1]) == 0\n"
        )
        env = {**os.environ, "VAK_HOME": str(root / "home"), "VAK_PROVIDER": provider, "VAK_MODEL": model}
        result = run(
            "live-code-skill-tools-child",
            [str(BIN), "exec", "Use the code-task skill. Read .vak/skills/code-task/SKILL.md, README.md, and test_app.py. Edit only app.py and run python3 test_app.py. Then use the advertised task tool to ask a readonly child agent to review app.py and the test result. After the child returns, run python3 test_app.py again as the final action. Do not edit tests or README, and do not edit app.py after the final passing test.", "--trust", "--yes", "--max-turns", "24"],
            cwd=root,
            env=env,
            timeout=900 if provider_is_local(provider) else 360,
        )
        if result.returncode != 0:
            raise RuntimeError(result.output)
        tests = subprocess.run([sys.executable, "test_app.py"], cwd=root, text=True, capture_output=True)
        if tests.returncode != 0:
            raise RuntimeError(f"live code invariant failed: {tests.stdout}\n{(root / 'app.py').read_text()}")
        ledgers = list((root / "home" / "sessions").glob("*/*.jsonl"))
        calls = ledger_tool_calls(ledgers)
        if not any(name == "read" and str(inputs.get("path", "")).endswith("SKILL.md")
                   for name, inputs in calls):
            raise RuntimeError("live run did not execute a skill read")
        if not any(name == "task" for name, _ in calls):
            raise RuntimeError("live run did not execute a child-agent task call")
        verification_calls = [inputs for name, inputs in calls
                              if name == "bash" and "python3 test_app.py" in str(inputs.get("command", ""))]
        if len(verification_calls) < 2:
            raise RuntimeError("live run did not verify both before and after child review")
        return result


def run_live_matrix(results: list[Result], provider: str, model: str, repeat: int) -> None:
    for index in range(repeat):
        result = live_code_case(provider, model)
        result.name = f"{result.name}-{index}"
        results.append(result)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeat", type=int, default=2)
    parser.add_argument("--live", action="store_true", help="run bounded real-model cases")
    parser.add_argument("--provider", default="ollama")
    parser.add_argument("--model", default="gemma4:e2b-mlx")
    parser.add_argument("--skills-root", help="explicit directory containing global SKILL.md files")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    if not BIN.exists():
        raise SystemExit(f"missing built binary: {BIN}")

    results: list[Result] = []
    run_offline_matrix(results, max(1, args.repeat), args.skills_root)
    if args.live:
        run_live_matrix(results, args.provider, args.model, max(1, args.repeat))

    report = [{"name": r.name, "command": r.command, "elapsed": round(r.elapsed, 3), "returncode": r.returncode, "output_tail": r.output} for r in results]
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        for item in report:
            print(f"PASS {item['name']} ({item['elapsed']}s)")
        print(f"PASS {len(report)} compound regression cases")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(f"FAIL {error}", file=sys.stderr)
        raise SystemExit(1)
