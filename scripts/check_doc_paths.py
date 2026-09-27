#!/usr/bin/env python3
"""Check current top-level doc citations and all local Markdown links.

Catches stale citations (the AGENTS.md contract is load-bearing; a doc
pointing at renamed files is drift). Skips URLs and anchors.

Backticked path checks cover top-level docs and docs/design. AGENTS.md carries
the crate map and README.md the build commands. Relative Markdown links are
checked recursively under docs, including dated records and nested folders.
"""
import os
import re
import sys
from pathlib import Path
from urllib.parse import unquote

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

# AGENTS.md's crate map is a column-aligned block inside a code fence: a
# path at column 0, two-or-more spaces, then prose that wraps onto indented
# continuation lines. Backticking those paths would be the obvious fix and
# is the wrong one — it destroys the alignment that makes the map readable,
# which is the only reason it is a fence rather than a table.
#
# So the format is parsed instead. This is the single densest set of path
# citations in the repository and it went unchecked through a crate rename.
#
# One space, not two: the map pads to a fixed column, and the two longest
# crate names (`vak-permission`, `vak-client-ui`) overflow it and get a
# single space. Requiring two silently skipped exactly those two — and a
# checker that matches nothing looks identical to one that passes.
MAP_ENTRY = re.compile(r"^((?:crates|docs|scripts)/[A-Za-z0-9_./-]*) +\S")
FENCE = re.compile(r"^```")

# Build outputs: legitimately absent from a fresh checkout, so their absence
# is not drift. Docs still need to name them — "which directory ships" is
# the whole point of docs/design/32 — and a fresh clone is exactly what CI
# checks out, so without this the gate fails on every CI run while passing
# on any machine that had built the UI once. That is the leftover-state
# dependency the release doc itself warns about, reproduced in its checker.
#
# `git check-ignore` cannot decide this: `dist/` matches DIRECTORIES only,
# so git reports a missing `dist/` as *not* ignored — precisely backwards
# for this use. An explicit list is duller and actually correct.
#
# `crates/vak-client-ui/dist-web` is deliberately NOT here: it is committed,
# so if it goes missing that IS drift and should fail.
BUILD_OUTPUTS = {
    "crates/vak-client-ui/dist",  # Tauri bundle; gitignored, `npm run build`
}


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
    in_fence = False
    for line_no, line in enumerate(body.splitlines(), 1):
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        cites = [(c, c) for c in PAT.findall(line)]
        cites += [(c, f"crates/{c}") for c in SHORTHAND.findall(line)]
        # Column-aligned map entries, which carry no backticks by design.
        if in_fence and (entry := MAP_ENTRY.match(line)):
            cite = entry.group(1)
            cites.append((cite, cite))
        for cite, target in cites:
            if "://" in cite or cite.startswith("http"):
                continue
            # A glob names a SET of paths, not one; `scripts/*.sh` is prose
            # about a directory, and "does this exact path exist" is the
            # wrong question to ask of it.
            if any(ch in cite for ch in "*?["):
                continue
            # strip trailing punctuation commonly inside code spans
            target = target.rstrip(".:,;").rstrip("/")
            if target in BUILD_OUTPUTS:
                continue
            if not os.path.exists(os.path.join(ROOT, target)):
                missing.append(f"{rel}:{line_no}: {cite}")

if missing:
    print(f"FAIL: {len(missing)} stale doc citation(s):")
    for m in missing:
        print(" ", m)
    sys.exit(1)

# Markdown links in nested docs were outside the backtick citation scan above.
# Check repository-relative destinations across the full documentation tree.
# Absolute paths in dated reports can name the original review machine or
# temporary evidence; URLs and fragment-only links have no local file target.
LINK = re.compile(r"!?\[[^]]*\]\(([^)]+)\)")
for path in sorted(Path(ROOT, "docs").rglob("*.md")):
    body = path.read_text(encoding="utf-8")
    open_fence = None
    for line_no, line in enumerate(body.splitlines(), 1):
        fence = re.match(r"^ {0,3}(`{3,}|~{3,})(.*)$", line)
        if not fence:
            continue
        marker = fence.group(1)
        if open_fence is None:
            open_fence = (marker[0], len(marker), line_no)
        elif (
            marker[0] == open_fence[0]
            and len(marker) >= open_fence[1]
            and not fence.group(2).strip()
        ):
            open_fence = None
    if open_fence is not None:
        missing.append(f"{path.relative_to(ROOT)}:{open_fence[2]}: unclosed code fence")
    for match in LINK.finditer(body):
        target = match.group(1).split(" ", 1)[0].strip("<>")
        if not target or target.startswith(("/", "#", "http:", "https:", "mailto:", "data:")):
            continue
        target = unquote(target.split("#", 1)[0])
        if not target or target in ("url", "…"):
            continue
        if not (path.parent / target).exists():
            line_no = body.count("\n", 0, match.start()) + 1
            missing.append(f"{path.relative_to(ROOT)}:{line_no}: {target}")

if missing:
    print(f"FAIL: {len(missing)} stale doc link(s):")
    for item in missing:
        print(" ", item)
    sys.exit(1)
print("doc citations and relative links ok")
