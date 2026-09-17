#!/usr/bin/env python3
"""Discover, execute, and report a large VAK capability scenario matrix.

Each Rust test is treated as a distinct scenario and tagged by crate/test
target. The runner can execute a bounded prefix or the complete discovered
matrix, in parallel, with per-case timeouts and JSON/Markdown reports.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import json
import os
import pathlib
import re
import signal
import subprocess
import sys
import time
from collections import Counter
from dataclasses import asdict, dataclass


ROOT = pathlib.Path(__file__).resolve().parents[1]
TEST_LINE = re.compile(r"^(.+): test$")


@dataclass(frozen=True)
class Scenario:
    scenario_id: str
    package: str
    target: str
    test: str
    capability: str
    command: tuple[str, ...]
    profile: str = "unique"


@dataclass
class Outcome:
    scenario_id: str
    package: str
    target: str
    test: str
    capability: str
    status: str
    returncode: int
    elapsed_seconds: float
    output_tail: str
    profile: str


def captured(command: tuple[str, ...] | list[str], timeout: int) -> tuple[int, str, bool]:
    process = subprocess.Popen(command, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, start_new_session=True)
    try:
        output, _ = process.communicate(timeout=timeout)
        return process.returncode, output, False
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
        return 124, partial + remainder + "\nTIMEOUT\n", True


def rust_targets() -> list[tuple[str, str, str]]:
    targets: list[tuple[str, str, str]] = []
    seen: set[tuple[str, str, str]] = set()
    for crate_dir in sorted((ROOT / "crates").glob("*")):
        if not crate_dir.is_dir():
            continue
        package = crate_dir.name
        if (crate_dir / "src/lib.rs").exists():
            seen.add((package, "lib", "lib"))
        if (crate_dir / "src/main.rs").exists():
            seen.add((package, package, "bin"))
    for path in sorted((ROOT / "crates").glob("*/tests/*.rs")):
        package = path.parent.parent.name
        seen.add((package, path.stem, "test"))
    return sorted(seen)


def capability(package: str, target: str, test: str = "") -> str:
    text = f"{package}/{target}/{test}".lower()
    groups = {
        "agent-orchestration": ("adoption", "fanout", "worker", "goal", "reliability", "run_endurance", "stop_guard", "steering", "task::"),
        "memory-knowledge": ("memory", "learning", "reflection", "session_search", "search_endpoint", "personal_os"),
        "mcp": ("mcp",),
        "hooks": ("hook",),
        "plugins": ("plugin",),
        "channels": ("chat_bridge", "telegram", "gateway", "webhook", "media_passthrough"),
        "schedules-events": ("scheduler", "heartbeat", "inbox"),
        "permissions-security": ("permission", "security", "sandbox", "claims"),
        "tools": ("tool",),
        "flows-planning": ("flow", "planner", "validation"),
        "llm-providers": ("vak-llm", "stream", "vision", "provider", "route", "model", "context"),
        "persistence-recovery": ("checkpoint", "session_log", "backup", "store"),
    }
    for name, needles in groups.items():
        if any(needle in text for needle in needles):
            return name
    return "core-runtime"


def discover(timeout: int) -> list[Scenario]:
    scenarios: list[Scenario] = []
    for package, target, target_kind in rust_targets():
        target_args = ["--lib"] if target_kind == "lib" else ["--bin", target] if target_kind == "bin" else ["--test", target]
        command = ["cargo", "test", "-q", "-p", package, *target_args, "--", "--list"]
        returncode, output, timed_out = captured(command, timeout)
        if timed_out:
            raise RuntimeError(f"test discovery timed out for {package}/{target}")
        if returncode:
            raise RuntimeError(f"test discovery failed for {package}/{target}:\n{output[-2000:]}")
        for line in output.splitlines():
            match = TEST_LINE.match(line.strip())
            if not match:
                continue
            test = match.group(1)
            scenario_id = f"{package}/{target_kind}:{target}/{test}"
            run_command = ["cargo", "test", "-q", "-p", package, *target_args, test, "--", "--exact", "--test-threads=1"]
            scenarios.append(Scenario(
                scenario_id=scenario_id,
                package=package,
                target=target,
                test=test,
                capability=capability(package, target, test),
                command=tuple(run_command),
            ))
    return sorted({scenario.scenario_id: scenario for scenario in scenarios}.values(), key=lambda item: item.scenario_id)


def repeat_scenarios(scenarios: list[Scenario], repetitions: int) -> list[Scenario]:
    executions: list[Scenario] = []
    for repetition in range(repetitions):
        for scenario in scenarios:
            executions.append(Scenario(
                scenario_id=f"{scenario.scenario_id}#repetition={repetition}",
                package=scenario.package,
                target=scenario.target,
                test=scenario.test,
                capability=scenario.capability,
                command=scenario.command,
                profile=f"repetition-{repetition}",
            ))
    return executions


def select_scenarios(scenarios: list[Scenario], limit: int) -> list[Scenario]:
    """Select a balanced prefix so every discovered capability is exercised."""
    if len(scenarios) <= limit:
        return scenarios
    grouped: dict[str, list[Scenario]] = {}
    for scenario in scenarios:
        grouped.setdefault(scenario.capability, []).append(scenario)
    selected: list[Scenario] = []
    groups = [grouped[key] for key in sorted(grouped)]
    index = 0
    while len(selected) < limit:
        added = False
        for group in groups:
            if index < len(group) and len(selected) < limit:
                selected.append(group[index])
                added = True
        if not added:
            break
        index += 1
    return selected


def execute(scenario: Scenario, timeout: int) -> Outcome:
    started = time.monotonic()
    code, output, timed_out = captured(scenario.command, timeout)
    status = "timeout" if timed_out else "passed" if code == 0 else "failed"
    return Outcome(scenario.scenario_id, scenario.package, scenario.target, scenario.test,
                   scenario.capability, status, code, time.monotonic() - started, output[-3000:], scenario.profile)


def markdown(report: dict) -> str:
    counts = report["summary"]["by_status"]
    lines = [
        "# VAK 500+ Scenario Harness Report",
        "",
        f"Generated: `{report['generated_at']}`",
        f"Total scenarios: **{report['summary']['total']}**",
        f"Passed: **{counts.get('passed', 0)}** · Failed: **{counts.get('failed', 0)}** · Timeouts: **{counts.get('timeout', 0)}**",
        "",
        "## Capability coverage",
        "",
        "| Capability | Scenarios | Passed | Failed | Timeout |",
        "|---|---:|---:|---:|---:|",
    ]
    for group, count in sorted(report["summary"]["by_capability"].items()):
        rows = [item for item in report["outcomes"] if item["capability"] == group]
        by_status = Counter(item["status"] for item in rows)
        lines.append(f"| {group} | {count} | {by_status['passed']} | {by_status['failed']} | {by_status['timeout']} |")
    profiles = Counter(item.get("profile", "unique") for item in report["outcomes"])
    lines.extend(["", "## Execution profiles", "", "| Profile | Executions |", "|---|---:|"])
    for profile, count in sorted(profiles.items()):
        lines.append(f"| {profile} | {count} |")
    lines.extend(["", "## Slowest scenarios", "", "| Scenario | Capability | Seconds |", "|---|---|---:|"])
    for item in sorted(report["outcomes"], key=lambda value: value["elapsed_seconds"], reverse=True)[:10]:
        lines.append(f"| `{item['scenario_id']}` | {item['capability']} | {item['elapsed_seconds']:.3f} |")
    failures = [item for item in report["outcomes"] if item["status"] != "passed"]
    lines.extend(["", "## Failures and timeouts", ""])
    if not failures:
        lines.append("None.")
    else:
        for item in failures:
            lines.extend([f"### `{item['scenario_id']}`", "", f"```text\n{item['output_tail']}\n```", ""])
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--limit", type=int, default=500, help="maximum unique scenarios to select")
    parser.add_argument("--repeat", type=int, default=1, help="honest repetitions of the selected unique set")
    parser.add_argument("--workers", type=int, default=min(8, os.cpu_count() or 1))
    parser.add_argument("--discovery-timeout", type=int, default=120)
    parser.add_argument("--case-timeout", type=int, default=120)
    parser.add_argument("--json", type=pathlib.Path, default=ROOT / "target" / "harness-500.json")
    parser.add_argument("--markdown", type=pathlib.Path, default=ROOT / "target" / "harness-500.md")
    parser.add_argument("--discover-only", action="store_true")
    args = parser.parse_args()
    scenarios = discover(args.discovery_timeout)
    unique_count = len(scenarios)
    if args.limit > unique_count:
        raise SystemExit(f"requested {args.limit} unique scenarios but only {unique_count} were discovered")
    selected = select_scenarios(scenarios, args.limit)
    executions = repeat_scenarios(selected, max(1, args.repeat))
    print(f"DISCOVERED {unique_count} distinct Rust test scenarios; SELECTED {len(selected)}; EXECUTIONS {len(executions)}")
    if args.discover_only:
        for scenario in selected:
            print(f"{scenario.capability}\t{scenario.scenario_id}")
        return 0
    started = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.workers)) as pool:
        outcomes = list(pool.map(lambda scenario: execute(scenario, args.case_timeout), executions))
    outcomes.sort(key=lambda item: item.scenario_id)
    report = {
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "runner": str(pathlib.Path(__file__).resolve()),
        "configuration": {"unique_limit": args.limit, "repetitions": max(1, args.repeat),
                          "workers": args.workers, "case_timeout": args.case_timeout,
                          "unique_discovered": unique_count, "unique_selected": len(selected),
                          "total_executions": len(outcomes)},
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "summary": {
            "total": len(outcomes),
            "by_status": dict(Counter(item.status for item in outcomes)),
            "by_capability": dict(Counter(item.capability for item in outcomes)),
        },
        "outcomes": [asdict(item) for item in outcomes],
    }
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(report, indent=2) + "\n")
    args.markdown.write_text(markdown(report))
    print(f"REPORT_JSON {args.json}")
    print(f"REPORT_MARKDOWN {args.markdown}")
    print(json.dumps(report["summary"], sort_keys=True))
    return 0 if report["summary"]["by_status"].get("failed", 0) == 0 and report["summary"]["by_status"].get("timeout", 0) == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
