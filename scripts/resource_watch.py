#!/usr/bin/env python3
"""Sample VAK and its local Ollama service while a stress run is in progress."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import pathlib
import signal
import subprocess
import time


def snapshot(root_pid: int) -> dict:
    result = subprocess.run(["ps", "-axo", "pid=,ppid=,%cpu=,rss=,command="], text=True, capture_output=True)
    rows = {}
    for line in result.stdout.splitlines():
        fields = line.strip().split(None, 4)
        if len(fields) < 5:
            continue
        pid, ppid, cpu, rss, command = fields
        rows[int(pid)] = {"pid": int(pid), "ppid": int(ppid), "cpu": float(cpu), "rss_kb": int(rss), "command": command[:300]}
    selected = []
    frontier = {root_pid}
    while frontier:
        selected.extend(rows[pid] for pid in frontier if pid in rows)
        frontier = {row["pid"] for row in rows.values() if row["ppid"] in frontier}
    def is_ollama_process(row: dict) -> bool:
        executable = row["command"].split(None, 1)[0].lower()
        return executable.rsplit("/", 1)[-1] in {"ollama", "ollama.exe"} or \
            "/ollama.app/" in executable

    ollama = [row for row in rows.values() if is_ollama_process(row)]
    load = os.getloadavg()
    try:
        therm = subprocess.run(["pmset", "-g", "therm"], text=True,
                               capture_output=True, timeout=5).stdout[-2000:]
    except (FileNotFoundError, subprocess.TimeoutExpired):
        therm = "thermal telemetry unavailable"
    return {"timestamp": dt.datetime.now(dt.timezone.utc).isoformat(), "root_pid": root_pid,
            "load_1m": load[0], "load_5m": load[1], "load_15m": load[2],
            "process_cpu_percent_sum": round(sum(row["cpu"] for row in selected), 2),
            "process_rss_mb_sum": round(sum(row["rss_kb"] for row in selected) / 1024, 2),
            "ollama_cpu_percent_sum": round(sum(row["cpu"] for row in ollama), 2),
            "ollama_rss_mb_sum": round(sum(row["rss_kb"] for row in ollama) / 1024, 2),
            "system_cpu_percent_sum": round(sum(row["cpu"] for row in rows.values()), 2),
            "system_rss_mb_sum": round(sum(row["rss_kb"] for row in rows.values()) / 1024, 2),
            "ollama_processes": ollama,
            "processes": selected, "thermal_raw": therm}


def terminate_tree(root_pid: int) -> None:
    result = subprocess.run(["ps", "-axo", "pid=,ppid="], text=True, capture_output=True)
    children: dict[int, list[int]] = {}
    for line in result.stdout.splitlines():
        fields = line.split()
        if len(fields) == 2:
            children.setdefault(int(fields[1]), []).append(int(fields[0]))
    ordered: list[int] = []
    frontier = [root_pid]
    while frontier:
        current = frontier.pop()
        ordered.append(current)
        frontier.extend(children.get(current, []))
    for pid in reversed(ordered):
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    time.sleep(0.2)
    for pid in reversed(ordered):
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("pid", type=int)
    parser.add_argument("--interval", type=float, default=5)
    parser.add_argument("--jsonl", type=pathlib.Path, default=pathlib.Path("target/resource-watch.jsonl"))
    parser.add_argument("--max-cpu", type=float, default=0, help="stop when process-tree CPU exceeds this; zero disables")
    parser.add_argument("--max-rss-mb", type=float, default=0, help="stop when process-tree RSS exceeds this; zero disables")
    args = parser.parse_args()
    args.jsonl.parent.mkdir(parents=True, exist_ok=True)
    with args.jsonl.open("w") as stream:
        while True:
            state = snapshot(args.pid)
            stream.write(json.dumps(state) + "\n")
            stream.flush()
            if not any(item["pid"] == args.pid for item in state["processes"]):
                return 0
            if args.max_cpu and state["process_cpu_percent_sum"] > args.max_cpu:
                print(f"CPU threshold exceeded: {state['process_cpu_percent_sum']}")
                terminate_tree(args.pid)
                return 2
            if args.max_rss_mb and state["process_rss_mb_sum"] > args.max_rss_mb:
                print(f"RSS threshold exceeded: {state['process_rss_mb_sum']} MB")
                terminate_tree(args.pid)
                return 2
            time.sleep(max(0.5, args.interval))


if __name__ == "__main__":
    raise SystemExit(main())
