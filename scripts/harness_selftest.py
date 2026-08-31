#!/usr/bin/env python3
"""Deterministic tests for VAK's Python harness contracts."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import harness_500
import prompt_scenarios
import resource_watch


class HarnessContracts(unittest.TestCase):
    def test_prompt_fingerprints_collapse_renamed_duplicate_fixtures(self) -> None:
        original = prompt_scenarios.CASES[4]
        renamed = prompt_scenarios.Case(
            "renamed-only",
            original.task,
            original.files,
            original.check,
            original.required_output,
            original.forbidden_files,
            original.allowed_changes,
        )
        self.assertEqual(prompt_scenarios.case_fingerprint(original),
                         prompt_scenarios.case_fingerprint(renamed))

    def test_prompt_corpus_has_only_unique_semantic_cases(self) -> None:
        fingerprints = [prompt_scenarios.case_fingerprint(case)
                        for case in prompt_scenarios.CASES]
        self.assertEqual(len(prompt_scenarios.CASES), 14)
        self.assertEqual(len(fingerprints), len(set(fingerprints)))

    def test_ledger_parser_ignores_prompt_text_and_reads_tool_blocks(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            ledger = Path(directory) / "session.jsonl"
            entries = [
                {"kind": "message", "message": {"content": [
                    {"type": "text", "text": "I used task and read SKILL.md"},
                ]}},
                {"kind": "message", "message": {"content": [
                    {"type": "tool_use", "name": "read", "input": {"path": "/x/SKILL.md"}},
                    {"type": "tool_use", "name": "task", "input": {"label": "review"}},
                ]}},
            ]
            ledger.write_text("".join(json.dumps(entry) + "\n" for entry in entries))
            self.assertEqual(
                [name for name, _ in prompt_scenarios.ledger_tool_calls([ledger])],
                ["read", "task"],
            )

    def test_report_lock_refuses_a_second_live_writer(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.json"
            _, release = prompt_scenarios.acquire_report_lock(report)
            try:
                with self.assertRaises(SystemExit):
                    prompt_scenarios.acquire_report_lock(report)
            finally:
                release()

    def test_report_lock_recovers_a_dead_owner(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.json"
            lock = report.with_suffix(".json.lock")
            lock.write_text(json.dumps({"pid": 999_999_999, "report": str(report)}))
            _, release = prompt_scenarios.acquire_report_lock(report)
            release()
            self.assertFalse(lock.exists())

    def test_interrupt_cleanup_terminates_registered_process_group(self) -> None:
        process = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(30)"],
            start_new_session=True,
        )
        with prompt_scenarios.ACTIVE_PROCESSES_LOCK:
            prompt_scenarios.ACTIVE_PROCESSES.add(process)
        try:
            prompt_scenarios.terminate_active_processes()
            self.assertIsNotNone(process.wait(timeout=3))
        finally:
            with prompt_scenarios.ACTIVE_PROCESSES_LOCK:
                prompt_scenarios.ACTIVE_PROCESSES.discard(process)
            if process.poll() is None:
                process.kill()

    def test_workspace_snapshot_ignores_runtime_cache_but_detects_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "app.py").write_text("before\n")
            cache = root / "__pycache__"
            cache.mkdir()
            (cache / "app.pyc").write_bytes(b"cache")
            snapshot = prompt_scenarios.snapshot_workspace(root)
            self.assertEqual(set(snapshot), {"app.py"})

    def test_repetitions_are_labeled_executions_not_unique_scenarios(self) -> None:
        scenario = harness_500.Scenario(
            "one", "pkg", "target", "test", "tools", ("true",),
        )
        executions = harness_500.repeat_scenarios([scenario], 3)
        self.assertEqual(len(executions), 3)
        self.assertEqual({item.test for item in executions}, {"test"})
        self.assertEqual(len({item.scenario_id for item in executions}), 3)

    def test_capability_uses_test_name(self) -> None:
        self.assertEqual(harness_500.capability("vak-core", "lib", "memory::recall"),
                         "memory-knowledge")
        self.assertEqual(harness_500.capability("vak-agent", "hooks", "pre_tool"), "hooks")

    def test_provider_failure_precedes_fixture_postcondition(self) -> None:
        self.assertEqual(
            prompt_scenarios.failure_class(
                2,
                ["postcondition test failed: fixture is unchanged"],
                "error: provider auth missing: set OPENROUTER_API_KEY",
            ),
            "provider",
        )

    def test_resource_snapshot_separates_run_tree_from_ollama(self) -> None:
        snapshot = resource_watch.snapshot(os.getpid())
        self.assertIn("process_cpu_percent_sum", snapshot)
        self.assertIn("ollama_cpu_percent_sum", snapshot)
        self.assertIn("ollama_processes", snapshot)


if __name__ == "__main__":
    unittest.main()
