#!/usr/bin/env python3
"""CI gate: every backticked repo path cited in the docs must exist.

Catches stale citations (the AGENTS.md contract is load-bearing; a doc
pointing at renamed files is drift). Skips URLs and anchors.

Scope is every prose doc that cites paths, not just docs/design/. AGENTS.md
carries the crate map and README.md the build commands — the two documents a
newcomer follows literally — and both were unchecked while a crate rename
went through them.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DOC_DIRS = ["docs/design", "docs"]
TOP_LEVEL = ["AGENTS.md", "README.md", "PRODUCT.md", "DESIGN.md", "CHANGELOG.md"]
PAT = re.compile(
    r"`((?:crates|docs|scripts|Cargo\.(?:toml|lock)|AGENTS\.md|README\.md)[^`\s]*)`"
)
# AGENTS.md cites crate-internal files in shorthand — `vak-server/src/x.rs`
# rather than `crates/vak-server/src/x.rs` — and that form was invisible to
# the pattern above, so the contract doc most likely to be followed
# literally was the one least checked.
SHORTHAND = re.compile(r"`(vak-[a-z0-9-]+/[^`\s]*)`")


def documents():
    """Every doc to check, without visiting a directory twice."""
    seen = set()
    for rel in TOP_LEVEL:
        path = os.path.join(ROOT, rel)
        if os.path.exists(path):
            seen.add(rel)
            yield rel, path
    for directory in DOC_DIRS:
        base = os.path.join(ROOT, directory)
        if not os.path.isdir(base):
            continue
        for name in sorted(os.listdir(base)):
            if not name.endswith(".md"):
                continue
            rel = os.path.join(directory, name)
            if rel in seen:
                continue
            seen.add(rel)
            yield rel, os.path.join(ROOT, rel)


missing = []
for rel, path in documents():
    body = open(path, encoding="utf-8").read()
    # A proposal names the layout it INTENDS to create. Those paths are
    # targets, not citations, and holding them to "must exist" would either
    # fail forever or push authors to stop writing plans down.
    if re.search(r"^Status:.*\bproposal\b", body, re.MULTILINE | re.IGNORECASE):
        continue
    # A dated report describes the tree as it was. Renaming a file does not
    # make the record wrong, and rewriting the record to match today would.
    if re.search(r"^>\s*\*\*Historical record", body, re.MULTILINE):
        continue
    for line_no, line in enumerate(body.splitlines(), 1):
        cites = [(c, c) for c in PAT.findall(line)]
        cites += [(c, f"crates/{c}") for c in SHORTHAND.findall(line)]
        for cite, target in cites:
            if "://" in cite or cite.startswith("http"):
                continue
            # A glob names a SET of paths, not one; `scripts/*.sh` is prose
            # about a directory, and "does this exact path exist" is the
            # wrong question to ask of it.
            if any(ch in cite for ch in "*?["):
                continue
            # strip trailing punctuation commonly inside code spans
            full = os.path.join(ROOT, target.rstrip(".:,;"))
            if not os.path.exists(full):
                missing.append(f"{rel}:{line_no}: {cite}")

if missing:
    print(f"FAIL: {len(missing)} stale doc citation(s):")
    for m in missing:
        print(" ", m)
    sys.exit(1)
print("doc citations ok")
