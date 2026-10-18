#!/usr/bin/env python3
"""Measure the tree and write docs/architecture/blueprint.html.

Every number on the page comes from this run: line counts, the crate graph
(`cargo metadata`), commits per path (`git log`), the longest files and
functions, and a few named shapes (routes, the Tool trait, the state
registry, worker tasks, card forms). The prose and the drawn diagrams live
in scripts/blueprint/template.html; see docs/architecture/AGENTS.md for
what to review by hand when this prints a warning.

The output depends only on the tree and on history up to the last commit
that touched something other than the blueprint itself, so committing a
freshly generated page and generating again changes nothing.

Usage: python3 scripts/blueprint.py [--deep] [--quiet]

--deep also runs the slow checks in scripts/blueprint/evidence.py (clippy's
cognitive_complexity over every workspace library, and cargo audit) and
rewrites docs/architecture/measures.json, which every other run reads.
"""

import html
import json
import os
import re
import subprocess
import sys
from collections import Counter
from functools import lru_cache

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "blueprint"))
import evidence  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = "docs/architecture/blueprint.html"
TEMPLATE = os.path.join(ROOT, "scripts", "blueprint", "template.html")
CC = "docs/architecture/measures.json"
# Commits that touch only the generated files are left out of every count.
SELF = (":(exclude)" + OUT, ":(exclude)" + CC)

# Where each crate sits and what it is for. A crate missing here is drawn in
# an "Unassigned" band and reported; add it (docs/architecture/AGENTS.md).
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
    "vak-agent": ("L4", "The agent loop: steps, retries, breaker, tool waves, cards, plan check, workers"),
    "vak-context": ("L4", "Capacity profiles, working-set planner, prompt assembly"),
    "vak-llm": ("L4", "Provider wires, SSE, model discovery, route ladder, receipts"),
    "vak-tools": ("L4", "Built-in tools, argument reader, broker worker protocol, egress proxy"),
    "vak-mcp": ("L4", "Lazy MCP client pool behind one meta-tool"),
    "vak-hooks": ("L4", "Lifecycle hooks"),
    "vak-plugin": ("L4", "Plugin manifests, stores, catalog signatures"),
    "vak-intent": ("L3", "Behavioural axes, the signal resolver, strands, the Limits lattice"),
    "vak-control": ("L3", "Decision points, step contracts, verdicts, the correction ladder"),
    "vak-judge": ("L3", "Receipts to verdicts: call, plan check, outcome"),
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
    m["routes"] = sum(len(re.findall(r"\.route\(\s*\"", open(p, encoding="utf-8").read())) for p in files_under("crates/vak-server/src", (".rs",)))
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
    for d in ("crates/vak-tools/src", "crates/vak-core/src", "crates/vak-agent/src"):
        for p in files_under(d, (".rs",)):
            lines = production_lines(p)
            for i, line in enumerate(lines):
                if "fn name(&self)" in line:
                    window = " ".join(lines[i:i + 3])
                    hit = re.search(r'fn name\(&self\)[^"]*"([a-z_]+)"', window)
                    if hit:
                        found.add(hit.group(1))
    return sorted(found)


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


def esc(s):
    return html.escape(str(s))


def k(n):
    return f"{n/1000:.0f}k" if n >= 10000 else f"{n:,}"


def layer_svg(crates):
    bands = LAYERS + ([("L?", "Unassigned")] if any(c["layer"] == "L?" for c in crates) else [])
    W, X0, BAND, GAP, TOP = 1240, 150, 92, 14, 20
    BW = W - X0 - 20
    height = TOP + (len(bands) + 1) * (BAND + GAP) + 26
    out = [f'<svg viewBox="0 0 {W} {height}" role="img" aria-label="Layer stack of every crate; box width is proportional to source lines within its layer">']
    y = TOP
    for lid, name in bands:
        items = sorted((c for c in crates if c["layer"] == lid), key=lambda c: -c["src"])
        total = sum(c["src"] for c in items) or 1
        out.append(f'<rect class="band" x="10" y="{y}" width="{W-20}" height="{BAND}"/>')
        out.append(f'<text class="lid" x="22" y="{y+30}">{lid}</text><text class="lname" x="22" y="{y+50}">{esc(name)}</text><text class="lsub" x="22" y="{y+66}">{k(total)} lines</text>')
        x = X0
        minw = 64
        flex = BW - 6 * (len(items) - 1) - minw * len(items)
        for c in items:
            w = minw + flex * c["src"] / total
            cls = "box hot" if c["commits"] >= HOT else "box"
            label = "vak (CLI)" if c["name"] == "vak" else c["name"].replace("vak-", "")
            out.append(f'<g><title>{esc(c["name"])}: {c["src"]:,} source lines, {c["commits"]} commits</title><rect class="{cls}" x="{x:.1f}" y="{y+12}" width="{w:.1f}" height="{BAND-24}" rx="2"/>')
            if w > 90:
                out.append(f'<text class="bl" x="{x+8:.1f}" y="{y+36}">{esc(label)}</text><text class="bs" x="{x+8:.1f}" y="{y+54}">{c["src"]/1000:.1f}k · {c["commits"]}c</text>')
            else:
                out.append(f'<text class="bl small" x="{x+w/2:.1f}" y="{y+40}" text-anchor="middle">{esc(label)}</text><text class="bs" x="{x+w/2:.1f}" y="{y+56}" text-anchor="middle">{c["src"]/1000:.1f}k</text>')
            out.append("</g>")
            x += w + 6
        y += BAND + GAP
    ext = ["OS sandbox: Seatbelt · Landlock · seccomp", "Keychain / encrypted file", "SQLite · filesystem", "launchd · systemd · Docker",
           "Model APIs and local models", "Telegram · Discord · Slack", "Mail and calendar providers", "NATS (optional)"]
    out.append(f'<rect class="band ext" x="10" y="{y}" width="{W-20}" height="{BAND}"/><text class="lid" x="22" y="{y+30}">L0</text><text class="lname" x="22" y="{y+50}">Host and outside</text><text class="lsub" x="22" y="{y+66}">not in the repo</text>')
    ew = (BW - 18) / 4
    for i, e in enumerate(ext):
        xx, yy = X0 + (i % 4) * (ew + 6), y + 10 + (i // 4) * 38
        out.append(f'<rect class="box extb" x="{xx:.1f}" y="{yy}" width="{ew:.1f}" height="32" rx="2"/><text class="bs" x="{xx+8:.1f}" y="{yy+21}">{esc(e)}</text>')
    out.append("</svg>")
    return "\n".join(out)


def matrix(crates):
    order = [l for l, _ in LAYERS]
    by = {c["name"]: c for c in crates}
    cell = Counter()
    for c in crates:
        for d in c["deps"]:
            if d in by:
                cell[(c["layer"], by[d]["layer"])] += 1
    rows = []
    for a in order:
        tds = []
        for b in order:
            n = cell[(a, b)]
            cls = "z" if n == 0 else ("up" if lnum(b) > lnum(a) else ("same" if a == b else "dn"))
            tds.append(f'<td class="m {cls}">{n or "·"}</td>')
        rows.append(f"<tr><th>{a}</th>{''.join(tds)}</tr>")
    head = "".join(f"<th>{b}</th>" for b in order)
    return f'<table class="mx"><thead><tr><th>from \\ to</th>{head}</tr></thead><tbody>{"".join(rows)}</tbody></table>'


def upward_list(crates):
    items = [c for c in crates if c["upward"]]
    if not items:
        return "<p>No crate depends on a crate in a higher layer.</p>"
    lis = "".join(f'<li><code>{esc(c["name"])}</code> ({c["layer"]}) uses ' + ", ".join(f"<code>{esc(d)}</code>" for d in c["upward"]) + "</li>" for c in items)
    return f'<ul class="plain">{lis}</ul>'


def bom(crates):
    rows = []
    for c in sorted(crates, key=lambda c: (-lnum(c["layer"]), -c["src"])):
        up = f' · <span class="warn">up: {esc(", ".join(d.replace("vak-", "") for d in c["upward"]))}</span>' if c["upward"] else ""
        deps = str(len(c["deps"])) if c["kind"] == "rust" else "—"
        notes = []
        if c["largest"][1]:
            notes.append(f'largest file {esc(c["largest"][1].split("/src/", 1)[-1])} ({c["largest"][0]:,})')
        if c["kind"] == "rust" and c["tests"] == 0:
            notes.append('<span class="warn">no tests/</span>')
        lang = "TS" if c["kind"] == "ts" else "Rust"
        rows.append(
            f'<tr data-layer="{c["layer"]}"><td><code>{esc(c["name"])}</code></td><td>{c["layer"]}</td><td>{lang}</td>'
            f'<td class="n">{c["src"]:,}</td><td class="n">{c["tests"]:,}</td><td class="n">{c["files"]}</td><td class="n">{c["commits"]}</td>'
            f'<td class="n">{deps}{up}</td><td class="n">{len(c["used_by"])}</td><td class="n">{len(c["ext"])}</td>'
            f'<td>{esc(c["role"])}</td><td class="note">{"; ".join(notes)}</td></tr>')
    return "\n".join(rows)


def filters(crates):
    present = [l for l, _ in LAYERS if any(c["layer"] == l for c in crates)]
    if any(c["layer"] == "L?" for c in crates):
        present.append("L?")
    names = dict(LAYERS + [("L?", "Unassigned")])
    out = ['<button type="button" id="f-all" data-f="all" aria-pressed="true">All</button>']
    for l in present:
        out.append(f'<button type="button" id="f-{l.strip("?") or "u"}" data-f="{l}" aria-pressed="false">{l} {esc(names[l])}</button>')
    return "\n".join(out)


def work_bars(crates, m):
    items = [("docs/", m["docs_commits"], False)] + [((c["name"] if c["name"] != "vak" else "vak (CLI)"), c["commits"], c["commits"] >= HOT) for c in sorted(crates, key=lambda c: -c["commits"])]
    top = max(n for _, n, _ in items) or 1
    out = []
    for name, n, hot in items:
        out.append(f'<div class="bar"><span>{esc(name)}</span><span class="t"><i{" class=hot" if hot else ""} style="width:{100*n/top:.1f}%"></i></span><span class="v">{n:,}</span></div>')
    return "\n".join(out)


def top_files(file_sizes, n=12):
    return "\n".join(f'<tr><td><code>{esc(p)}</code></td><td class="n">{l:,}</td></tr>' for l, p in file_sizes[:n])


def top_fns(fns, n=12):
    return "\n".join(f'<tr><td><code>{esc(name)}</code></td><td><code>{esc(p)}:{s}</code></td><td class="n">{l:,}</td></tr>' for l, name, p, s in fns[:n])


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


def top_cc(cc, n=12):
    if not cc:
        return '<tr><td colspan="3">Not measured yet: run <code>python3 scripts/blueprint.py --deep</code>.</td></tr>'
    return "\n".join(f'<tr><td><code>{esc(r["name"])}</code></td><td><code>{esc(r["file"])}:{r["line"]}</code></td><td class="n">{r["score"]}</td></tr>' for r in cc["complexity"][:n])


def evidence_blocks(crates, m):
    """Run the cheap evidence checks; return (values for {{e:…}}, %%blocks%%)."""
    me = sys.modules[__name__]
    hot = evidence.hotspots(me)
    fan = evidence.fanout(crates)
    wr = evidence.tool_wrappers(me)
    blk = evidence.blocking(me)
    tests, tmed = evidence.test_density(me, crates)
    comp = evidence.completion_models(me)
    lex = evidence.lexicon(me)
    led = evidence.ledger_types(me)
    by_fan = sorted(fan.items(), key=lambda kv: -kv[1])
    e = {
        "hot.files": hot["files"], "hot.commits": hot["commits"], "hot.fix_commits": hot["fix_commits"],
        "hot.rho_density": f'{hot["rho_density"]:+.2f}', "hot.rho_count": f'{hot["rho_count"]:+.2f}',
        "hot.top10_lines": f'{hot["top10_lines"]:.0f}%', "hot.top10_fixes": f'{hot["top10_fixes"]:.0f}%',
        "hot.median": f'{hot["median_density"]:.1f}',
        "fan.top": ", ".join(f"<code>{esc(n)}</code> {v}" for n, v in by_fan[:3]),
        "fan.core": fan.get("vak-core", 0), "fan.session": fan.get("vak-session", 0), "fan.llm": fan.get("vak-llm", 0),
        "fan.crates": len(fan),
        "closure.session": len(evidence.closure(crates, "vak-session")),
        "closure.session.list": ", ".join(f"<code>{esc(x)}</code>" for x in evidence.closure(crates, "vak-session")),
        "tool.methods": wr["methods"], "tool.defaults": wr["defaults"], "tool.impls": wr["impls"],
        "tool.median": wr["median_overrides"], "tool.wrappers": len(wr["wrappers"]),
        "blk.async": blk["async_fns"], "blk.fs_fns": blk["fs_fns"], "blk.fs_calls": blk["fs_calls"],
        "blk.proc_fns": blk["proc_fns"], "blk.offloads": blk["offloads"],
        "blk.by_crate": ", ".join(f"<code>{esc(c)}</code> {n}" for c, n in list(blk["fs_by_crate"].items())[:4]),
        "test.median": f"{tmed:.1f}",
        "comp.count": sum(1 for r in comp["models"] if r["variants"]), "comp.translations": comp["translations"],
        "comp.variants": ", ".join(f'{r["variants"]}' for r in comp["models"] if r["variants"]),
        "lex.tables": lex["tables"], "lex.entries": lex["entries"], "lex.commits": lex["commits"],
        "led.llm": led["uses"].get("vak_llm", 0), "led.intent": led["uses"].get("vak_intent", 0),
        "led.control": led["uses"].get("vak_control", 0),
        "led.types_commits": led["types_commits"], "led.co_changed": led["co_changed"],
    }
    for name, t in tests.items():
        e[f"test.{name}"] = f'{t["per_kloc"]:.1f}'
    rows_hot = "\n".join(f'<tr><td><code>{esc(r["file"])}</code></td><td class="n">{r["lines"]:,}</td><td class="n">{r["commits"]}</td><td class="n">{r["fixes"]}</td><td class="n">{r["density"]:.1f}</td></tr>' for r in hot["top"])
    rows_wr = "\n".join(f'<li><code>{esc(w["type"])}</code> ({esc(w["file"])}:{w["line"]}): ' + (("does not forward " + ", ".join(f"<code>{esc(x)}</code>" for x in w["missing"])) if w["missing"] else "forwards every default") + "</li>" for w in wr["wrappers"]) or "<li>No wrapping tools found.</li>"
    rows_comp = "\n".join(f'<tr><td><code>{esc(r["enum"])}</code></td><td>{esc(r["role"])}</td><td class="n">{r["variants"] if r["variants"] is not None else "gone"}</td><td><code>{esc(r["file"])}</code></td></tr>' for r in comp["models"])
    thin = sorted(tests.items(), key=lambda kv: kv[1]["per_kloc"])
    rows_tests = "\n".join(f'<tr><td><code>{esc(n)}</code></td><td class="n">{t["tests"]}</td><td class="n">{t["per_kloc"]:.1f}</td><td>{"yes" if t["fuzz"] else "—"}</td></tr>' for n, t in thin)
    misses = [f"completion model {r['enum']} ({r['file']})" for r in comp["models"] if r["variants"] is None]
    return e, {"HOTSPOTS": rows_hot, "WRAPPERS": rows_wr, "COMPLETION": rows_comp, "TESTS": rows_tests}, misses


def audit_rows(cc):
    if not cc or not cc.get("audit"):
        return '<tr><td colspan="4">Not measured: run <code>python3 scripts/blueprint.py --deep</code> with network access.</td></tr>', 0, 0
    a = cc["audit"]
    rows = "\n".join(f'<tr><td><code>{esc(v["id"])}</code></td><td><code>{esc(v["package"])} {esc(v["version"])}</code></td><td>{esc(v["title"])}</td></tr>' for v in a["vulnerabilities"])
    warn = sum(len(v) for v in a["warnings"].values())
    return rows, len(a["vulnerabilities"]), warn


HOT = 0


def main():
    global HOT
    quiet = "--quiet" in sys.argv
    crates, file_sizes, fns, m = measure()
    if "--deep" in sys.argv:
        with open(os.path.join(ROOT, CC), "w", encoding="utf-8") as fh:
            json.dump(evidence.deep(sys.modules[__name__], m["revision"]), fh, indent=1)
            fh.write("\n")
    cc = load_measures()
    if cc:
        scores = sorted(r["score"] for r in cc["complexity"])
        m["cc_count"] = len(scores)
        m["cc_median"] = scores[len(scores) // 2]
        m["cc_p99"] = scores[int(0.99 * (len(scores) - 1))]
        m["cc_default"] = CLIPPY_DEFAULT
        m["cc_over_default"] = sum(1 for x in scores if x > CLIPPY_DEFAULT)
        m["cc_revision"] = cc["revision"]
    ev, ev_blocks, ev_misses = evidence_blocks(crates, m)
    audit_html, m["audit_vulns"], m["audit_warnings"] = audit_rows(cc)
    cc_by_name = {}
    for i, r in enumerate(cc["complexity"] if cc else []):
        cc_by_name.setdefault(r["name"], (r["score"], i + 1))
    counts = sorted((c["commits"] for c in crates), reverse=True)
    # A hot spot is a crate in the busiest tenth by commits, at least one.
    HOT = counts[max(0, len(counts) // 10 - 1)] if counts else 0
    m["hot"] = HOT
    fn_len = {}
    for l, name, p, s in fns:
        fn_len.setdefault(name, (l, p, s))
    file_len = {p: l for l, p in file_sizes}
    by = {c["name"]: c for c in crates}
    misses = anchor_misses() + ev_misses
    unassigned = [c["name"] for c in crates if c["layer"] == "L?"]
    stale = [n for n in CRATES if n not in by]

    def sub(match):
        kind, key = match.group(1), match.group(2)
        if kind == "m":
            if key not in m:
                misses.append(f"measure {key} (template)")
                return "?"
            v = m[key]
            return esc(f"{v:,}" if isinstance(v, int) else (len(v) if isinstance(v, list) else v))
        if kind == "k":
            if key not in m:
                misses.append(f"measure {key} (template)")
                return "?"
            return esc(k(m[key]))
        if kind == "fn":
            if key in fn_len:
                return f"{fn_len[key][0]:,}"
            misses.append(f"fn {key} (template)")
            return "?"
        if kind == "file":
            if key in file_len:
                return f"{file_len[key]:,}"
            misses.append(f"file {key} (template)")
            return "?"
        if kind in ("deps", "usedby", "src", "tests", "commits"):
            c = by.get(key)
            if not c:
                misses.append(f"crate {key} (template)")
                return "?"
            v = {"deps": len(c["deps"]), "usedby": len(c["used_by"]), "src": c["src"], "tests": c["tests"], "commits": c["commits"]}[kind]
            return f"{v:,}"
        if kind in ("cc", "ccrank", "ccx"):
            if key not in cc_by_name:
                misses.append(f"complexity of {key} (template; run --deep)")
                return "?"
            score, rank = cc_by_name[key]
            if kind == "cc":
                return str(score)
            if kind == "ccrank":
                return f"{rank:,}"
            return f"{score / max(1, m['cc_median']):.0f}"
        if kind in ("hist", "histpct"):
            if key not in fn_len:
                misses.append(f"fn {key} (template)")
                return "?"
            l, path, start = fn_len[key]
            n = fn_history(path, start, l)
            return f"{n:,}" if kind == "hist" else f"{100 * n / max(1, m['total_commits']):.0f}%"
        if kind == "e":
            if key not in ev:
                misses.append(f"evidence {key} (template)")
                return "?"
            v = ev[key]
            return f"{v:,}" if isinstance(v, int) else str(v)
        if kind == "pct":
            a, b = key.split("/")
            return f"{100*m[a]/max(1, m[b]):.0f}%"
        return match.group(0)

    page = open(TEMPLATE, encoding="utf-8").read()
    blocks = {
        "LAYER_SVG": layer_svg(crates), "MATRIX": matrix(crates), "UPWARD": upward_list(crates),
        "BOM": bom(crates), "FILTERS": filters(crates), "WORK": work_bars(crates, m),
        "TOP_FILES": top_files(file_sizes), "TOP_FNS": top_fns(fns), "TOP_CC": top_cc(cc), "AUDIT": audit_html, **ev_blocks,
        "TOOLS": ", ".join(f"<code>{esc(t)}</code>" for t in m["tools"]),
        "HOT": str(HOT),
    }
    for key, val in blocks.items():
        page = page.replace("%%" + key + "%%", val)
    page = re.sub(r"\{\{(m|k|e|fn|file|deps|usedby|src|tests|commits|pct|cc|ccrank|ccx|hist|histpct):([^}]+)\}\}", sub, page)
    review = misses + [f"crate {n} has no layer" for n in unassigned] + [f"crate {n} is gone" for n in stale]
    banner = ""
    if review:
        banner = '<section class="banner" role="status"><b>Needs review.</b> These names in the drawings or the prose are no longer found: ' + "; ".join(esc(r) for r in review) + ". See docs/architecture/AGENTS.md.</section>"
    page = page.replace("%%REVIEW%%", banner)
    with open(os.path.join(ROOT, OUT), "w", encoding="utf-8") as fh:
        fh.write(page)
    if not quiet:
        print(f"wrote {OUT} ({len(crates)} crates, {m['rust_src']:,} Rust lines, revision {m['revision']})")
    for r in review:
        print(f"blueprint: needs review: {r}", file=sys.stderr)


if __name__ == "__main__":
    main()
