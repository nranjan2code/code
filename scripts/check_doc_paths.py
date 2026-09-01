#!/usr/bin/env python3
"""CI gate: every backticked repo path cited in docs/design/*.md must exist.

Catches stale citations (the AGENTS.md contract is load-bearing; a design
doc pointing at renamed files is drift). Skips URLs and anchors.
"""
import os
import re
import sys

DOCS = "docs/design"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PAT = re.compile(r"`((?:crates|docs|scripts|Cargo\.(?:toml|lock)|AGENTS\.md|README\.md)[^`\s]*)`")

missing = []
for name in sorted(os.listdir(DOCS)):
    if not name.endswith(".md"):
        continue
    path = os.path.join(DOCS, name)
    body = open(path, encoding="utf-8").read()
    # A proposal names the layout it INTENDS to create. Those paths are
    # targets, not citations, and holding them to "must exist" would either
    # fail forever or push authors to stop writing plans down.
    if re.search(r"^Status:.*\bproposal\b", body, re.MULTILINE | re.IGNORECASE):
        continue
    for line_no, line in enumerate(body.splitlines(), 1):
        for cite in PAT.findall(line):
            if "://" in cite or cite.startswith("http"):
                continue
            # strip trailing punctuation commonly inside code spans
            cite_clean = cite.rstrip(".:,;")
            full = os.path.join(ROOT, cite_clean)
            if not os.path.exists(full):
                missing.append(f"{path}:{line_no}: {cite}")

if missing:
    print(f"FAIL: {len(missing)} stale doc citation(s):")
    for m in missing:
        print(" ", m)
    sys.exit(1)
print("doc citations ok")
