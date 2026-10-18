"""Evidence behind the blueprint: the checks findings rest on, and the
record's health.

Each function measures one thing from the tree or its history and returns
plain data; model.py assembles it and render.py draws it. `health()` runs
the gate's own record checks so the blueprint shows their results. `deep()`
runs the slow measures (clippy's cognitive complexity and cargo audit),
cached in blueprint/measures.json by `run.py deep`.

Heuristics are named where they are used. Nothing here reads a data home,
and only `deep()` reaches the network (the RustSec advisory database).
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
        "density": {r[3]: r[2] / r[0] * 1000 for r in rows},
        "top": [{"file": r[3], "lines": r[0], "commits": r[1], "fixes": r[2], "density": r[2] / r[0] * 1000} for r in top],
    }


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


def health(b):
    """The gate's record checks, run here so the blueprint shows their state:
    each as ok or failing with the lines it printed, plus the count of code
    names living documents cite that no longer exist."""
    checks = []
    for name, cmd in (("records", ["python3", "scripts/check_records.py"]),
                      ("doc paths", ["python3", "scripts/check_doc_paths.py"]),
                      ("versions", ["scripts/check-version.sh"])):
        out = subprocess.run(cmd, cwd=b.ROOT, capture_output=True, text=True)
        lines = [l.strip() for l in (out.stdout + out.stderr).splitlines() if l.strip()]
        problems = [l for l in lines if not l.startswith(("✓", "records ok", "doc citations"))] if out.returncode else []
        checks.append({"check": name, "ok": out.returncode == 0, "problems": problems[:20]})
    names = subprocess.run(["python3", "scripts/check_records.py", "--names"], cwd=b.ROOT, capture_output=True, text=True).stdout
    head = re.match(r"(\d+) cited code names not found, in (\d+) living documents", names)
    stale = {"names": int(head.group(1)), "documents": int(head.group(2))} if head else {"names": 0, "documents": 0}
    return {"checks": checks, "stale_names": stale}


def deep(b, revision):
    """The slow measures: clippy cognitive complexity over every workspace
    library, and cargo audit's advisories against Cargo.lock."""
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
        out["advisories"] = {
            "vulnerabilities": sorted(f'{v["advisory"]["id"]} {v["package"]["name"]} {v["package"]["version"]}' for v in report["vulnerabilities"]["list"]),
            "warnings": sum(len(ws) for ws in report.get("warnings", {}).values()),
            "ignored": "see .cargo/audit.toml",
        }
    except (ValueError, KeyError):
        out["advisories"] = None

    return out
