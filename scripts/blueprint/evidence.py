"""Evidence checks behind the blueprint's findings.

Each function measures one claim from the tree or its history and returns
plain data; scripts/blueprint.py renders it. Cheap checks run on every build.
`deep()` runs the slow ones (clippy's cognitive complexity, cargo audit) and
is cached in docs/architecture/measures.json by `blueprint.py --deep`.

Heuristics are named where they are used. Nothing here reads a data home,
and only `deep()` may reach the network (the RustSec advisory database).
"""

import json
import os
import re
import subprocess
import sys
import tempfile
from collections import defaultdict

# A commit that reports a failure it repairs. Used only to compare files
# with each other, so its blind spots fall on every file alike.
FIX_WORDS = re.compile(
    r"\b(fix(e[sd])?|bug|regress\w*|broke(n)?|crash\w*|hang\w*|deadlock\w*|was wrong|no longer|"
    r"never (worked|reached|ran)|wrongly|stale|leak\w*)\b", re.I)


def git(root, *args):
    return subprocess.run(["git", *args], cwd=root, capture_output=True, text=True).stdout


def spearman(a, b):
    def rank(xs):
        order = sorted(range(len(xs)), key=lambda i: xs[i])
        r = [0.0] * len(xs)
        for pos, i in enumerate(order):
            r[i] = pos
        return r
    if len(a) < 3:
        return 0.0
    ra, rb = rank(a), rank(b)
    ma, mb = sum(ra) / len(ra), sum(rb) / len(rb)
    cov = sum((x - ma) * (y - mb) for x, y in zip(ra, rb))
    va = sum((x - ma) ** 2 for x in ra) ** .5
    vb = sum((y - mb) ** 2 for y in rb) ** .5
    return cov / (va * vb) if va and vb else 0.0


def commit_files(b):
    """{hash: (is_fix, Rust source files under crates/ it touched)} for HEAD's
    history. Commits never change, so the answer is cached in the git dir
    (local, never committed) and only new commits are read."""
    git_dir = git(b.ROOT, "rev-parse", "--absolute-git-dir").strip()
    cache_path = os.path.join(git_dir, "vak-blueprint-commits.json")
    try:
        cache = json.load(open(cache_path, encoding="utf-8"))
    except (OSError, ValueError):
        cache = {}
    wanted = git(b.ROOT, "rev-list", "HEAD", "--", "crates").split()
    missing = [h for h in wanted if h not in cache]
    if missing:
        log = git(b.ROOT, "log", "--no-walk=unsorted", "--format=@@%H%x1f%B%x1f", "--name-only", *missing)
        for chunk in log.split("@@")[1:]:
            parts = chunk.split("\x1f")
            files = [f for f in (parts[2] if len(parts) > 2 else "").split("\n") if f.startswith("crates/") and f.endswith(".rs") and "/src/" in f]
            cache[parts[0]] = [bool(FIX_WORDS.search(parts[1])), files]
        try:
            with open(cache_path, "w", encoding="utf-8") as fh:
                json.dump(cache, fh)
        except OSError:
            pass
    return {h: cache[h] for h in wanted if h in cache}


def hotspots(b):
    """Does a file's size predict how often it needs fixing, per line?"""
    touch, fixes, fix_commits, commits = defaultdict(int), defaultdict(int), 0, 0
    for is_fix, files in commit_files(b).values():
        commits += 1
        fix_commits += is_fix
        for f in files:
            touch[f] += 1
            fixes[f] += is_fix
    rows = []
    for f in touch:
        full = os.path.join(b.ROOT, f)
        if os.path.exists(full):
            n = b.lines_of([full])
            if n >= 50:
                rows.append((n, touch[f], fixes[f], f))
    rows.sort(reverse=True)
    sizes = [r[0] for r in rows]
    density = [r[2] / r[0] * 1000 for r in rows]
    lines, fixed = sum(sizes) or 1, sum(r[2] for r in rows) or 1
    top = rows[:10]
    med = sorted(density)[len(density) // 2] if density else 0
    return {
        "files": len(rows), "commits": commits, "fix_commits": fix_commits,
        "rho_count": spearman(sizes, [r[2] for r in rows]),
        "rho_density": spearman(sizes, density),
        "top10_lines": 100 * sum(r[0] for r in top) / lines,
        "top10_fixes": 100 * sum(r[2] for r in top) / fixed,
        "median_density": med,
        "top": [{"file": r[3], "lines": r[0], "commits": r[1], "fixes": r[2], "density": r[2] / r[0] * 1000} for r in top],
    }


def fanout(crates):
    """How many workspace crates rebuild when each crate changes."""
    rdeps = defaultdict(set)
    for c in crates:
        for d in c["deps"]:
            rdeps[d].add(c["name"])
    out = {}
    for c in crates:
        seen, stack = set(), [c["name"]]
        while stack:
            for r in rdeps[stack.pop()]:
                if r not in seen:
                    seen.add(r)
                    stack.append(r)
        out[c["name"]] = len(seen)
    return out


def closure(crates, name):
    by = {c["name"]: c for c in crates}
    seen, stack = set(), [name]
    while stack:
        for d in by.get(stack.pop(), {"deps": []})["deps"]:
            if d not in seen:
                seen.add(d)
                stack.append(d)
    return sorted(seen)


def tool_wrappers(b):
    """Which defaulted Tool declarations each wrapping tool fails to forward."""
    src = open(os.path.join(b.ROOT, "crates/vak-tools/src/lib.rs"), encoding="utf-8").read().split("\n")
    start = next(i for i, l in enumerate(src) if re.match(r"^pub trait Tool\b", l))
    methods, defaults = [], set()
    i = start + 1
    while not src[i].startswith("}"):
        m = re.match(r"^    (?:async )?fn (\w+)", src[i])
        if m:
            j = i
            while not re.search(r"[;{]\s*$", src[j]):
                j += 1
            methods.append(m.group(1))
            if src[j].rstrip().endswith("{"):
                defaults.add(m.group(1))
        i += 1
    impls, wrappers = [], []
    for root in sorted(os.listdir(os.path.join(b.ROOT, "crates"))):
        for p in b.files_under(f"crates/{root}/src", (".rs",)):
            lines = b.production_lines(p)
            text = "\n".join(lines)
            for k, line in enumerate(lines):
                m = re.match(r"^(\s*)impl(?:<[^>]*>)? (?:vak_tools::|crate::)?Tool for (\w+)", line)
                if not m:
                    continue
                end = next((e for e in range(k + 1, len(lines)) if lines[e] == m.group(1) + "}"), len(lines))
                over = set(re.findall(r"\bfn (\w+)", "\n".join(lines[k:end]))) & set(methods)
                impls.append(len(over))
                decl = re.search(rf"(?:struct|enum) {m.group(2)}\b[^;{{]*(\{{.*?\n\}}|\(.*?\);)", text, re.S)
                if decl and re.search(r"dyn (?:vak_tools::)?Tool\b", decl.group(1)):
                    wrappers.append({"type": m.group(2), "file": b.rel(p), "line": k + 1,
                                     "missing": sorted(defaults - over)})
    impls.sort()
    return {"methods": len(methods), "defaults": len(defaults), "impls": len(impls),
            "median_overrides": impls[len(impls) // 2] if impls else 0, "wrappers": wrappers}


def blocking(b):
    """Calls that block an executor thread, inside async fns."""
    fs = re.compile(r"\bstd::fs::|\bfs::(?:read|read_to_string|write|create_dir_all|remove_file|remove_dir_all|rename|copy|metadata|read_dir)\b|\bFile::(?:open|create)\b")
    sync_cmd_import = re.compile(r"use std::process::(?:Command\b|\{[^}]*\bCommand\b)")
    off = re.compile(r"spawn_blocking|block_in_place")
    out = {"async_fns": 0, "offloads": 0, "fs_calls": 0, "fs_fns": 0, "proc_fns": 0, "fs_by_crate": defaultdict(int)}
    for crate in sorted(os.listdir(os.path.join(b.ROOT, "crates"))):
        for p in b.files_under(f"crates/{crate}/src", (".rs",)):
            lines = b.production_lines(p)
            whole = "\n".join(lines)
            out["offloads"] += len(off.findall(whole))
            sync_cmd = bool(sync_cmd_import.search(whole))
            for name, start, length in b.functions(p):
                if "async fn" not in lines[start - 1]:
                    continue
                out["async_fns"] += 1
                body = "\n".join(lines[start - 1:start - 1 + length])
                body = re.sub(r"spawn_blocking\((?:.|\n)*?\)\s*\.await", "", body)
                n = len(fs.findall(body))
                if n:
                    out["fs_calls"] += n
                    out["fs_fns"] += 1
                    out["fs_by_crate"][crate] += 1
                # std's Command, spelled out or imported by name (tokio's is async)
                if "std::process::Command::new(" in body or (sync_cmd and re.search(r"\bCommand::new\(", body)):
                    out["proc_fns"] += 1
    out["fs_by_crate"] = dict(sorted(out["fs_by_crate"].items(), key=lambda kv: -kv[1]))
    return out


def test_density(b, crates):
    """#[test] functions (inline and tests/) per thousand source lines."""
    rx = re.compile(r"#\[(?:tokio::)?test\b")
    out = {}
    for c in crates:
        if c["kind"] != "rust" or not c["src"]:
            continue
        files = b.files_under(f"crates/{c['name']}/src", (".rs",)) + b.files_under(f"crates/{c['name']}/tests", (".rs",))
        n = sum(len(rx.findall(open(p, encoding="utf-8", errors="replace").read())) for p in files)
        fuzz = os.path.isdir(os.path.join(b.ROOT, "crates", c["name"], "fuzz"))
        out[c["name"]] = {"tests": n, "per_kloc": n / (c["src"] / 1000), "fuzz": fuzz}
    vals = sorted(v["per_kloc"] for v in out.values())
    return out, (vals[len(vals) // 2] if vals else 0)


COMPLETION = [
    ("crates/vak-session/src/types.rs", "GoalStatus", "goal mode"),
    ("crates/vak-session/src/types.rs", "WorkContractStatus", "managed work"),
    ("crates/vak-intent/src/outcome.rs", "OutcomeStatus", "outcome result"),
    ("crates/vak-intent/src/outcome.rs", "CompletionVerdict", "outcome evaluation"),
    ("crates/vak-commit/src/types.rs", "Verdict", "commitment"),
]


def completion_models(b):
    """The separate vocabularies for 'is this work done', and translations between them."""
    rows = []
    for path, name, role in COMPLETION:
        full = os.path.join(b.ROOT, path)
        if not os.path.exists(full):
            rows.append({"enum": name, "file": path, "role": role, "variants": None})
            continue
        text = open(full, encoding="utf-8").read()
        m = re.search(rf"pub enum {name}\b[^{{]*\{{(.*?)\n\}}", text, re.S)
        variants = re.findall(r"^\s{4}([A-Z]\w*)", m.group(1), re.M) if m else []
        rows.append({"enum": name, "file": path, "role": role, "variants": len(variants) if m else None})
    names = [n for _, n, _ in COMPLETION]
    pat = re.compile(r"\b(" + "|".join(names) + r")::\w+\s*=>\s*(?:\w+::)*(" + "|".join(names) + r")::")
    translations = 0
    for crate in sorted(os.listdir(os.path.join(b.ROOT, "crates"))):
        for p in b.files_under(f"crates/{crate}/src", (".rs",)):
            text = "\n".join(b.production_lines(p))
            translations += sum(1 for m in pat.finditer(text) if m.group(1) != m.group(2))
    return {"models": rows, "translations": translations}


def lexicon(b):
    text = open(os.path.join(b.ROOT, "crates/vak-intent/src/signals.rs"), encoding="utf-8").read()
    tables = re.findall(r"const [A-Z_]+: &\[&str\]", text)
    entries = len(re.findall(r'^\s*"[^"]+",', text, re.M)) + sum(len(re.findall(r'"[^"]+"', m)) for m in re.findall(r"&\[&str\] = &\[([^\]\n]*)\];", text))
    commits = int(git(b.ROOT, "rev-list", "--count", "HEAD", "--", "crates/vak-intent/src/signals.rs").strip() or 0)
    return {"tables": len(tables), "entries": entries, "commits": commits}


def ledger_types(b):
    """What the session ledger takes from the provider and intent crates."""
    uses = defaultdict(int)
    for p in b.files_under("crates/vak-session/src", (".rs",)):
        for m in re.finditer(r"\b(vak_llm|vak_intent|vak_control)::", "\n".join(b.production_lines(p))):
            uses[m.group(1)] += 1
    types_log = git(b.ROOT, "log", "--format=%h", "--", "crates/vak-llm/src/types.rs").split()
    both = 0
    for h in types_log:
        if any(f.startswith("crates/vak-session/") for f in git(b.ROOT, "show", "--name-only", "--format=", h).split("\n")):
            both += 1
    return {"uses": dict(uses), "types_commits": len(types_log), "co_changed": both}


def deep(b, revision):
    """The slow checks: clippy cognitive complexity and cargo audit."""
    out = {"revision": revision}
    with tempfile.TemporaryDirectory() as conf:
        base = os.path.join(b.ROOT, "clippy.toml")
        with open(os.path.join(conf, "clippy.toml"), "w", encoding="utf-8") as fh:
            fh.write((open(base, encoding="utf-8").read() if os.path.exists(base) else "") + "\ncognitive-complexity-threshold = 1\n")
        run = subprocess.run(
            ["cargo", "clippy", "--offline", "--workspace", "--lib", "--message-format=json", "--",
             "-A", "clippy::all", "-W", "clippy::cognitive_complexity"],
            cwd=b.ROOT, env=dict(os.environ, CLIPPY_CONF_DIR=conf), capture_output=True, text=True)
    if run.returncode != 0:
        sys.exit("blueprint: clippy failed; deep measures not taken\n" + run.stderr[-2000:])
    spans = {}
    for line in run.stdout.splitlines():
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        diag = msg.get("message") or {}
        if msg.get("reason") != "compiler-message" or (diag.get("code") or {}).get("code") != "clippy::cognitive_complexity":
            continue
        score = re.search(r"\((\d+)/1\)", diag["message"])
        span = next((x for x in diag["spans"] if x.get("is_primary")), diag["spans"][0])
        spans[(span["file_name"], span["line_start"])] = int(score.group(1))
    fns = []
    for (path, line), score in spans.items():
        name, full = "?", os.path.join(b.ROOT, path)
        if os.path.isfile(full):
            best = None
            for fname, start, length in b.functions(full):
                if start <= line <= start + length - 1 and (best is None or start > best[1]):
                    best = (fname, start)
            name = best[0] if best else "?"
        fns.append({"score": score, "name": name, "file": path, "line": line})
    fns.sort(key=lambda r: (-r["score"], r["file"], r["line"]))
    out["complexity"] = fns

    audit = subprocess.run(["cargo", "audit", "--json"], cwd=b.ROOT, capture_output=True, text=True)
    try:
        report = json.loads(audit.stdout)
        out["audit"] = {
            "vulnerabilities": sorted(({"id": v["advisory"]["id"], "package": v["package"]["name"],
                                        "version": v["package"]["version"], "title": v["advisory"]["title"]}
                                       for v in report["vulnerabilities"]["list"]), key=lambda v: (v["package"], v["id"])),
            "warnings": {k: sorted({f'{w["package"]["name"]} {w["package"]["version"]}' for w in ws}) for k, ws in report.get("warnings", {}).items()},
        }
    except (ValueError, KeyError):
        out["audit"] = None
    return out
