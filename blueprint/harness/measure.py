"""Measurements of the tree: crates, their graph and sizes, commits per path,
the longest files and functions, and a few named shapes (routes, `vak`
subcommands, the Tool trait, card tools, the state registry, worker tasks,
card forms). The subsystems over them are subsystems.py.

A library for model.py; the entry point is blueprint/harness/run.py. Every
count depends only on the working tree and on history up to the last commit
that touched something other than the generated files, so committing a
freshly generated blueprint and generating again changes nothing.
"""

import json
import os
import re
import subprocess
import sys
from collections import Counter
from functools import lru_cache

HARNESS = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.dirname(HARNESS)
ROOT = os.path.dirname(HOME)
TEMPLATE = os.path.join(HARNESS, "template.html")
CONTENT = os.path.join(HOME, "content.toml")
# What the harness writes, relative to the repository root.
GENERATED = ("blueprint/README.md", "blueprint/blueprint.html", "blueprint/blueprint.json", "blueprint/measures.json")
CC = "blueprint/measures.json"
# Commits that touch only the generated files are left out of every count.
SELF = tuple(":(exclude)" + p for p in GENERATED)

# Where each crate sits and what it is for. A crate missing here is drawn in
# an "Unassigned" band and reported; add it (blueprint/AGENTS.md).
LAYERS = [
    ("L7", "Surfaces"),
    ("L6", "Service"),
    ("L5", "Orchestration facade"),
    ("L4", "Agent runtime"),
    ("L3", "Decision and policy"),
    ("L2", "Domain engines"),
    ("L1", "Data substrate"),
]
CRATES = {
    "vak-client-ui": ("L7", "SolidJS workspace client: the desktop bundle and the /app web bundle"),
    "vak-admin-ui": ("L7", "SolidJS admin console at /admin"),
    "vak-desktop": ("L7", "Tauri 2 shell around an embedded server router"),
    "vak-terminal": ("L7", "ratatui client of a running server (vak term)"),
    "vak": ("L7", "The CLI binary and every subcommand"),
    "vak-server": ("L6", "HTTP+SSE, gateway, bots, admin API, projection, Library, Canvas, preview origin, voice and mail routes"),
    "vak-ops": ("L6", "launchd/systemd control and service logs"),
    "vak-bus": ("L6", "NATS JetStream fabric, causal lineage, encrypted envelopes"),
    "vak-core": ("L5", "SDK facade: admission, routing, prompts, capabilities, memory, triggers, sync, backup, erasure"),
    "vak-flow": ("L5", "Static flow DAGs and the dynamic planner"),
    "vak-eval": ("L5", "Deterministic suite, live mode, context and banned-token gates"),
    "vak-agent": ("L4", "The agent loop: steps, retries, breaker, tool waves, cards, workers"),
    "vak-context": ("L4", "Capacity profiles, working-set planner, prompt assembly"),
    "vak-llm": ("L4", "Provider wires, SSE, model discovery, route ladder, receipts"),
    "vak-tools": ("L4", "Built-in tools, argument reader, broker worker protocol, egress proxy"),
    "vak-mcp": ("L4", "Lazy MCP client pool behind one meta-tool"),
    "vak-hooks": ("L4", "Lifecycle hooks"),
    "vak-plugin": ("L4", "Plugin manifests, stores, catalog signatures"),
    "vak-intent": ("L3", "Behavioural axes, the signal resolver, strands, the Limits lattice"),
    "vak-control": ("L3", "Decision points, step contracts, verdicts, the correction ladder"),
    "vak-judge": ("L3", "Receipts to verdicts: call, outcome"),
    "vak-commit": ("L3", "Durable commitments and the satisfaction lattice"),
    "vak-permission": ("L3", "Modes x rules over declared reach"),
    "vak-sandbox": ("L3", "Seatbelt, Landlock, seccomp, Docker; candidate promotion"),
    "vak-ooxml": ("L2", "Open XML engine: read, edit, blank packages"),
    "vak-pdf": ("L2", "PDF engine from ISO 32000-2"),
    "vak-intake": ("L2", "Feed connectors, bounded parsers, HTML reader"),
    "vak-mail-calendar": ("L2", "Mail and calendar accounts, OAuth, IMAP, vault, effects"),
    "vak-voice": ("L2", "Voice provider vocabulary and session protocol"),
    "vak-presentation": ("L2", "Closed primitive vocabulary, packs, compiler"),
    "vak-delivery": ("L2", "Semantic output contract, Markdown fallback, channel markup"),
    "vak-session": ("L1", "Sealed append-only ledgers, objects, fencing, runs, effects, cursors, turns"),
    "vak-catalog": ("L1", "SQLite catalog: nodes, lineage, FTS5"),
    "vak-lifecycle": ("L1", "Retention labels and the deterministic plan"),
    "vak-storage": ("L1", "Keys, refs, encrypted objects, records, segments"),
    "vak-config": ("L1", "Layered TOML, credentials, paths, scopes"),
    "vak-telemetry": ("L1", "Content-free structured telemetry"),
}
# Names the drawn sheets (processes, the turn) depend on. One that is no
# longer found means those drawings need a look.
ANCHORS = [
    ("fn", "run_turn_with_message", "crates/vak-core/src"),
    ("fn", "run_turn_recorded", "crates/vak-core/src"),
    ("fn", "run_message_inner", "crates/vak-agent/src"),
    ("fn", "complete_with_reliability", "crates/vak-agent/src"),
    ("fn", "authorize", "crates/vak-agent/src"),
    ("fn", "show_card", "crates/vak-agent/src"),
    ("fn", "plan_for_session", "crates/vak-context/src"),
    ("fn", "read_arguments", "crates/vak-tools/src"),
    ("fn", "fill_defaults", "crates/vak-tools/src"),
    ("fn", "project", "crates/vak-server/src/client_events.rs"),
    ("fn", "begin_dispatch", "crates/vak-session/src"),
    ("fn", "claim_due", "crates/vak-core/src"),
    ("text", "enum WorkerTask", "crates/vak-tools/src/broker.rs"),
    ("text", "struct CorePool", "crates/vak-server/src"),
    ("text", "allow(dead_code)", "crates/vak-server/src/bus.rs"),
]

FN_RE = re.compile(r"^(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)")


def run(*args, check=True):
    return subprocess.run(args, cwd=ROOT, capture_output=True, text=True, check=check).stdout


def files_under(path, exts):
    out = []
    base = os.path.join(ROOT, path)
    for dirpath, dirnames, filenames in os.walk(base):
        dirnames[:] = [d for d in dirnames if d not in ("node_modules", "dist", "dist-web", "target")]
        for f in filenames:
            if f.endswith(exts):
                out.append(os.path.join(dirpath, f))
    return sorted(out)


def lines_of(paths):
    total = 0
    for p in paths:
        with open(p, encoding="utf-8", errors="replace") as fh:
            total += sum(1 for _ in fh)
    return total


def rel(p):
    return os.path.relpath(p, ROOT)


@lru_cache(maxsize=None)
def _production_lines(path):
    """The file's lines with every top-level inline test module blanked."""
    with open(path, encoding="utf-8", errors="replace") as fh:
        lines = fh.read().split("\n")
    out, skipping = [], False
    for i, line in enumerate(lines):
        if not skipping and line.startswith("#[cfg(test)]"):
            j = i + 1
            while j < len(lines) and lines[j].startswith("#["):
                j += 1
            skipping = j < len(lines) and re.match(r"mod \w+ \{", lines[j]) is not None
        out.append("" if skipping else line)
        if skipping and line == "}":
            skipping = False
    return out


@lru_cache(maxsize=None)
def _functions(path):
    """(name, start line, length) for every rustfmt-formatted fn outside tests."""
    lines = production_lines(path)
    out = []
    for i, line in enumerate(lines):
        m = FN_RE.match(line)
        if not m or line.rstrip().endswith(";"):
            continue
        indent = m.group(1)
        for j in range(i + 1, len(lines)):
            if lines[j] == indent + "}":
                out.append((m.group(2), i + 1, j - i + 1))
                break
    return out


def production_lines(path):
    return list(_production_lines(path))


def functions(path):
    return list(_functions(path))


def commits(path):
    return int(run("git", "rev-list", "--count", "HEAD", "--", path, *SELF).strip() or 0)


def measure():
    meta = json.loads(run("cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"))
    rust = {p["name"]: p for p in meta["packages"]}
    names = set(rust)
    crate_dirs = sorted(d for d in os.listdir(os.path.join(ROOT, "crates")) if os.path.isdir(os.path.join(ROOT, "crates", d)))
    crates = []
    all_rs = []
    for d in crate_dirs:
        cdir = f"crates/{d}"
        if os.path.exists(os.path.join(ROOT, cdir, "Cargo.toml")):
            src = files_under(f"{cdir}/src", (".rs",))
            tests = files_under(f"{cdir}/tests", (".rs",))
            all_rs += src
            pkg = rust.get(d)
            deps = sorted({x["name"] for x in pkg["dependencies"] if x["name"] in names and x.get("kind") is None}) if pkg else []
            ext = sorted({x["name"] for x in pkg["dependencies"] if x["name"] not in names and x.get("kind") is None}) if pkg else []
            kind = "rust"
        elif os.path.exists(os.path.join(ROOT, cdir, "package.json")):
            src = files_under(f"{cdir}/src", (".ts", ".tsx", ".mts", ".mjs"))
            tests = [p for p in src if ".test." in p]
            src = [p for p in src if ".test." not in p]
            with open(os.path.join(ROOT, cdir, "package.json"), encoding="utf-8") as fh:
                ext = sorted(json.load(fh).get("dependencies", {}))
            deps = []
            kind = "ts"
        else:
            continue
        largest = max(((lines_of([p]), rel(p)) for p in src), default=(0, ""))
        crates.append({
            "name": d, "kind": kind, "src": lines_of(src), "tests": lines_of(tests), "files": len(src),
            "deps": deps, "ext": ext, "commits": commits(cdir), "largest": largest,
            "layer": CRATES.get(d, ("L?", ""))[0], "role": CRATES.get(d, ("L?", "Not yet placed on a layer"))[1],
        })
    by = {c["name"]: c for c in crates}
    for c in crates:
        c["used_by"] = sorted(o["name"] for o in crates if c["name"] in o["deps"])
        c["upward"] = [d for d in c["deps"] if d in by and lnum(by[d]["layer"]) > lnum(c["layer"])]

    file_sizes = sorted(((lines_of([p]), rel(p)) for p in all_rs), reverse=True)
    fns = []
    for p in all_rs:
        for name, start, length in functions(p):
            fns.append((length, name, rel(p), start))
    fns.sort(reverse=True)

    m = {}
    m["routes"] = len(routes())
    tool_src = open(os.path.join(ROOT, "crates/vak-tools/src/lib.rs"), encoding="utf-8").read().split("\n")
    m["tool_methods"] = count_block(tool_src, r"^pub trait Tool\b", r"^    (?:async )?fn ")
    m["core_pub_fns"] = sum(len(re.findall(r"^\s+pub (?:async )?fn ", "\n".join(production_lines(p)), re.MULTILINE)) for p in files_under("crates/vak-core/src", (".rs",)))
    state = open(os.path.join(ROOT, "crates/vak-core/src/state.rs"), encoding="utf-8").read()
    m["state_paths"] = len(re.findall(r'path:\s*"', state)) + len(re.findall(r'agent_entry\(\s*"', state))
    broker = open(os.path.join(ROOT, "crates/vak-tools/src/broker.rs"), encoding="utf-8").read().split("\n")
    m["worker_tasks"] = count_block(broker, r"^(?:pub(?:\([^)]*\))? )?enum WorkerTask\b", r"^    [A-Z][A-Za-z0-9]*\s*(?:\{|\(|,|$)")
    forms = open(os.path.join(ROOT, "crates/vak-delivery/src/skills.rs"), encoding="utf-8").read()
    fm = re.search(r"pub const FORMS: &\[&str\] = &\[(.*?)\];", forms, re.S)
    m["forms"] = len(re.findall(r'"[^"]+"', fm.group(1))) if fm else 0
    m["tools"] = builtin_tools()
    m["card_tools"] = card_tools()
    m["lock_crates"] = open(os.path.join(ROOT, "Cargo.lock"), encoding="utf-8").read().count("\n[[package]]")
    m["design_docs"] = len([f for f in os.listdir(os.path.join(ROOT, "docs/design")) if f.endswith(".md") and f != "INDEX.md"])
    m["agents_words"] = len(open(os.path.join(ROOT, "AGENTS.md"), encoding="utf-8").read().split())
    m["total_commits"] = int(run("git", "rev-list", "--count", "HEAD", "--", ".", *SELF).strip())
    m["docs_commits"] = commits("docs")
    dates = run("git", "log", "--format=%ad", "--date=short", "--", ".", *SELF).split()
    m["first_date"], m["last_date"] = (min(dates), max(dates)) if dates else ("", "")
    m["peak_day"] = Counter(dates).most_common(1)[0][1] if dates else 0
    m["days"] = days_between(m["first_date"], m["last_date"]) + 1 if dates else 0
    m["revision"] = run("git", "log", "-1", "--format=%h", "--", ".", *SELF).strip()
    cargo = open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8").read()
    m["version"] = re.search(r'^version = "([^"]+)"', cargo, re.MULTILINE).group(1)
    m["rust_src"] = sum(c["src"] for c in crates if c["kind"] == "rust")
    m["rust_tests"] = sum(c["tests"] for c in crates if c["kind"] == "rust")
    m["ts_src"] = sum(c["src"] for c in crates if c["kind"] == "ts")
    m["rust_crates"] = sum(1 for c in crates if c["kind"] == "rust")
    m["ts_crates"] = sum(1 for c in crates if c["kind"] == "ts")
    return crates, file_sizes, fns, m


def builtin_tools():
    found = set()
    for d in ("crates/vak-tools/src", "crates/vak-core/src", "crates/vak-agent/src", "crates/vak-mcp/src"):
        for p in files_under(d, (".rs",)):
            lines = production_lines(p)
            for i, line in enumerate(lines):
                if "fn name(&self)" in line:
                    window = " ".join(lines[i:i + 3])
                    hit = re.search(r'fn name\(&self\)[^"]*"([a-z_]+)"', window)
                    if hit:
                        found.add(hit.group(1))
    return sorted(found)


def card_tools():
    """Only offered card tools; the other shape builders remain drawable."""
    lines = production_lines(os.path.join(ROOT, "crates/vak-core/src/presentation_tools.rs"))
    shapes = "\n".join(lines).split("const SHAPES:", 1)[1].split("fn shape_of_form", 1)[0]
    return sorted(set(re.findall(
        r'CardShape\s*\{\s*offered:\s*true,.*?\.\.shape\(\s*"[^"]+",\s*"(emit_[a-z_]+_card)"',
        shapes, re.S,
    )))


def routes():
    """Every route path the server registers outside test modules."""
    out = []
    for p in files_under("crates/vak-server/src", (".rs",)):
        out += re.findall(r'\.route\(\s*"([^"]+)"', "\n".join(production_lines(p)))
    return sorted(set(out))


def cli_verbs():
    """The `vak` subcommands, from the variants of the CLI's Command enum."""
    lines = open(os.path.join(ROOT, "crates/vak/src/cli.rs"), encoding="utf-8").read().split("\n")
    out, inside = [], False
    for line in lines:
        if not inside and re.match(r"^(?:pub(?:\([^)]*\))? )?enum Command\b", line):
            inside = True
            continue
        if inside:
            if line.startswith("}"):
                break
            hit = re.match(r"^    ([A-Z][A-Za-z0-9_]*)\s*(?:\{|\(|,|$)", line)
            if hit:
                out.append(hit.group(1).rstrip("_").lower())
    return out


def count_block(lines, start_re, item_re):
    n, inside = 0, False
    for line in lines:
        if not inside and re.match(start_re, line):
            inside = True
            continue
        if inside:
            if line.startswith("}"):
                break
            if re.match(item_re, line):
                n += 1
    return n


def lnum(layer):
    return int(layer[1]) if layer[1:].isdigit() else 0


def days_between(a, b):
    from datetime import date
    return (date.fromisoformat(b) - date.fromisoformat(a)).days


def anchor_misses():
    misses = []
    for kind, what, where in ANCHORS:
        target = os.path.join(ROOT, where)
        paths = files_under(where, (".rs",)) if os.path.isdir(target) else ([target] if os.path.isfile(target) else [])
        pattern = rf"\bfn\s+{re.escape(what)}\b" if kind == "fn" else re.escape(what)
        if not any(re.search(pattern, open(p, encoding="utf-8").read()) for p in paths):
            misses.append(f"{what} ({where})")
    return misses


def k(n):
    return f"{n/1000:.0f}k" if n >= 10000 else f"{n:,}"


CLIPPY_DEFAULT = 25  # clippy's own cognitive-complexity-threshold default


def load_measures():
    path = os.path.join(ROOT, CC)
    if not os.path.exists(path):
        return None
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def fn_history(path, start, length):
    """Commits that changed a function's current lines, followed back."""
    out = run("git", "log", "--format=%h", "--no-patch", "-L", f"{start},{start + length - 1}:{path}", check=False)
    return len([x for x in out.split() if re.fullmatch(r"[0-9a-f]{7,40}", x)])
