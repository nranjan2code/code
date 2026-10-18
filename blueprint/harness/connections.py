"""How the subsystems connect, measured from the code.

Two kinds of connection, each counted as distinct (from file, to file)
pairs between modules that belong to different subsystems:

- `code`: one module names another. In Rust, a path (`vak_core::health`,
  `crate::erasure::…`, `vak_core::{a, b}`), an item by name
  (`vak_core::Core` resolves to the module that defines `pub struct Core`),
  or a call of a method defined once across the workspace and by none of
  the locked dependencies (`core.erase_conversation(…)`), from a crate that
  depends on the defining one. In TypeScript, a relative import.
- `http`: a client names a route. A string literal in any file outside
  `vak-server` that has the shape of a registered route (`${…}` and `{}`
  standing for a path segment) connects that file to the route's owner.

A method name defined more than once, or by a dependency, is skipped rather
than guessed, so a connection may be missing; one that is drawn was named in
the code. When a locked dependency's source is not in the cargo registry, no
method call is resolved and the harness says so.
"""

import os
import re
from collections import defaultdict

RUST_ITEM = re.compile(r"^\s*pub(?:\([^)]*\))?\s+(?:async\s+)?(?:unsafe\s+)?(?:struct|enum|trait|type|fn|const|static)\s+([A-Za-z_][A-Za-z0-9_]*)", re.M)
RUST_METHOD = re.compile(r"^\s+pub(?:\([^)]*\))?\s+(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)", re.M)
CRATE_PATH = re.compile(r"\b(vak_[a-z_]+|crate)::((?:[A-Za-z_][A-Za-z0-9_]*::)*)(\{[^{}]*\}|[A-Za-z_][A-Za-z0-9_]*)")
CALL = re.compile(r"\.([a-z_][a-z0-9_]{2,})\s*\(")
TS_IMPORT = re.compile(r"""(?:from|import)\s*\(?\s*['"](\.{1,2}/[^'"]+)['"]""")
LITERAL = re.compile(r"""["'`](/[A-Za-z0-9_\-./{}$]*?)["'`?]""")


def _crate_of(path):
    parts = path.split("/")
    return parts[1] if parts[0] == "crates" else None


def _module_file(crate, segments, files):
    """The deepest module file `crates/<crate>/src/<segments…>` names."""
    for n in range(len(segments), 0, -1):
        base = f"crates/{crate}/src/" + "/".join(segments[:n])
        for cand in (base + ".rs", base + "/mod.rs"):
            if cand in files:
                return cand, segments[n:]
    return None, segments


def dependency_fn_names(ms):
    """Every `fn` name the locked registry dependencies define, or None when
    one's source is missing. Cached in the git dir by Cargo.lock's digest."""
    import glob
    import hashlib
    import json
    lock = open(os.path.join(ms.ROOT, "Cargo.lock"), encoding="utf-8").read()
    digest = hashlib.sha256(lock.encode()).hexdigest()
    git_dir = ms.run("git", "rev-parse", "--absolute-git-dir").strip()
    cache = os.path.join(git_dir, "vak-blueprint-depfns.json")
    try:
        saved = json.load(open(cache, encoding="utf-8"))
        if saved.get("lock") == digest:
            return set(saved["names"])
    except (OSError, ValueError):
        pass
    roots = glob.glob(os.path.join(os.path.expanduser("~/.cargo/registry/src"), "*"))
    names = set()
    fn = re.compile(r"\bfn\s+([a-z_][a-z0-9_]*)")
    for name, version, source in re.findall(r'\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"\n(source = "[^"]+")?', lock):
        if "registry" not in source:
            continue
        found = [r for r in roots if os.path.isdir(os.path.join(r, f"{name}-{version}"))]
        if not found:
            return None
        for dirpath, _, filenames in os.walk(os.path.join(found[0], f"{name}-{version}")):
            for f in filenames:
                if f.endswith(".rs"):
                    names.update(fn.findall(open(os.path.join(dirpath, f), encoding="utf-8", errors="replace").read()))
    try:
        with open(cache, "w", encoding="utf-8") as fh:
            json.dump({"lock": digest, "names": sorted(names)}, fh)
    except OSError:
        pass
    return names


def _text(ms, path):
    full = os.path.join(ms.ROOT, path)
    if path.endswith(".rs"):
        return "\n".join(ms.production_lines(full))
    return open(full, encoding="utf-8", errors="replace").read()


def _route_rx(route):
    out = ""
    for seg in route.strip("/").split("/"):
        if seg.startswith("{") and seg.endswith("}"):
            out += "/[^/]+"
        else:
            out += "/" + re.escape(seg)
    return re.compile((out or "/") + "/?$")


def measure(ms, owner, route_owner, crate_deps):
    """The connections as dicts (from, to, kind, weight, example), and the
    review lines for what could not be measured. `crate_deps` maps a crate
    to the internal crates it depends on directly."""
    review = []
    dep_fns = dependency_fn_names(ms)
    if dep_fns is None:
        review.append("measure connections: a locked dependency's source is missing, so method calls were not resolved (cargo fetch)")
    reach = {}

    def reaches(a, b):
        if a not in reach:
            seen, todo = set(), [a]
            while todo:
                for d in crate_deps.get(todo.pop(), []):
                    if d not in seen:
                        seen.add(d)
                        todo.append(d)
            reach[a] = seen
        return a == b or b in reach[a]
    files = {f for f in owner if f.startswith("crates/") and "/src/" in f}
    rust = sorted(f for f in files if f.endswith(".rs"))
    texts = {f: _text(ms, f) for f in sorted(files) if not os.path.isdir(os.path.join(ms.ROOT, f))
             and f.endswith((".rs", ".ts", ".tsx", ".mts", ".mjs"))}

    items, methods = defaultdict(dict), defaultdict(list)
    for f in rust:
        crate = _crate_of(f)
        for name in RUST_ITEM.findall(texts[f]):
            items[crate].setdefault(name, f)
        for name in sorted(set(RUST_METHOD.findall(texts[f]))):
            methods[name].append(f)
    unique = {} if dep_fns is None else {name: fs[0] for name, fs in methods.items() if len(fs) == 1 and name not in dep_fns}
    crate_dir = {c.replace("-", "_"): c for c in {_crate_of(f) for f in rust}}

    pairs = defaultdict(set)  # (from file, to file) -> kinds

    def resolve(crate, segments):
        target, rest = _module_file(crate, segments, files)
        if target and rest:
            return items[crate].get(rest[-1], target) if rest[-1][:1].isupper() else target
        if target:
            return target
        return items[crate].get(segments[-1]) if segments else None

    for f in rust:
        here = _crate_of(f)
        text = texts[f]
        for root, path, last in CRATE_PATH.findall(text):
            crate = here if root == "crate" else crate_dir.get(root)
            if not crate:
                continue
            prefix = [s for s in path.split("::") if s]
            leaves = [x.strip().split("::")[0].split(" as ")[0].strip() for x in last[1:-1].split(",")] if last.startswith("{") else [last]
            for leaf in leaves:
                if leaf and leaf not in ("self", "super", "*"):
                    t = resolve(crate, prefix + [leaf])
                    if t and t != f:
                        pairs[(f, t)].add("code")
        for name in sorted(set(CALL.findall(text))):
            t = unique.get(name)
            if t and t != f and reaches(here, _crate_of(t)):
                pairs[(f, t)].add("code")

    for f, text in texts.items():
        if f.endswith(".rs"):
            continue
        for spec in TS_IMPORT.findall(text):
            base = os.path.normpath(os.path.join(os.path.dirname(f), spec))
            for cand in (base, *(base + e for e in (".ts", ".tsx", ".mts", ".mjs")), base + "/index.ts", base + "/index.tsx"):
                if cand in files:
                    if cand != f:
                        pairs[(f, cand)].add("code")
                    break

    routes = [(r, _route_rx(r)) for r in route_owner]
    for f, text in texts.items():
        if f.startswith("crates/vak-server/"):
            continue
        for lit in sorted(set(LITERAL.findall(text))):
            shaped = re.sub(r"\$\{[^}]*\}|\{\}|\{[a-z_]*\}", "x", lit).split("?")[0]
            if len(shaped) < 2:
                continue
            for r, rx in routes:
                if rx.match(shaped):
                    pairs[(f, ("route", r))].add("http")
                    break

    weight, sample = defaultdict(int), {}
    for (src, dst), kinds in sorted(pairs.items(), key=lambda kv: (kv[0][0], str(kv[0][1]))):
        a = owner.get(src)
        b = route_owner.get(dst[1]) if isinstance(dst, tuple) else owner.get(dst)
        if not a or not b or a == b:
            continue
        for kind in sorted(kinds):
            weight[(a, b, kind)] += 1
            sample.setdefault((a, b, kind), (src, dst[1] if isinstance(dst, tuple) else dst))
    edges = [{"from": a, "to": b, "kind": k, "weight": w, "example": list(sample[(a, b, k)])}
             for (a, b, k), w in sorted(weight.items(), key=lambda kv: (-kv[1], kv[0]))]
    return edges, review
