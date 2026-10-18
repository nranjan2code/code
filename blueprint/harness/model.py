"""One data model of the system, the source of all three views.

`build()` measures the tree (measure.py), runs the evidence and health
checks (evidence.py), reads the record (record.py) and the hand-written
content (blueprint/content.toml), and returns a plain, JSON-serialisable
dict plus the resolver that turns `{{kind:key}}` placeholders into values.
render.py draws the dict as Markdown, HTML and JSON; nothing else computes
a number the views show.
"""

import json
import os
import re
import sys
import tomllib

import evidence
import measure as ms
import record
import connections
import subsystems as subs

ACTIVITY_DAYS = 14  # the "what's happening" window, before the last measured commit
DECISIONS_SHOWN = 12

PLACEHOLDER = re.compile(r"\{\{(m|k|e|fn|file|deps|usedby|src|tests|commits|pct|cc|ccrank|ccx|hist|histpct|list):([^}]+)\}\}")


def build(deep=False):
    crates, file_sizes, fns, m = ms.measure()
    if deep:
        with open(os.path.join(ms.ROOT, ms.CC), "w", encoding="utf-8") as fh:
            json.dump(evidence.deep(ms, m["revision"]), fh, indent=1)
            fh.write("\n")
    cc = ms.load_measures()
    if cc:
        scores = sorted(r["score"] for r in cc["complexity"])
        m.update(cc_count=len(scores), cc_median=scores[len(scores) // 2],
                 cc_p99=scores[int(0.99 * (len(scores) - 1))], cc_default=ms.CLIPPY_DEFAULT,
                 cc_over_default=sum(1 for x in scores if x > ms.CLIPPY_DEFAULT), cc_revision=cc["revision"])
    counts = sorted((c["commits"] for c in crates), reverse=True)
    # A hot spot is a crate in the busiest tenth by commits, at least one.
    m["hot"] = counts[max(0, len(counts) // 10 - 1)] if counts else 0
    content = tomllib.load(open(ms.CONTENT, "rb"))
    m["prose_reviewed"] = content.get("prose_reviewed", "")

    hot = evidence.hotspots(ms)
    wr = evidence.tool_wrappers(ms)
    density = hot["density"]
    ev = {
        "hot.core": f'{density.get("crates/vak-core/src/lib.rs", 0):.1f}',
        "hot.agent": f'{density.get("crates/vak-agent/src/lib.rs", 0):.1f}',
        "hot.files": hot["files"], "hot.median": f'{hot["median_density"]:.1f}',
        "hot.rho_density": f'{hot["rho_density"]:+.2f}',
        "tool.methods": wr["methods"], "tool.defaults": wr["defaults"], "tool.impls": wr["impls"],
        "tool.wrappers": len(wr["wrappers"]),
    }
    lists = {"wrappers": ["`{type}` (`{file}:{line}`): {what}".format(
        what=("does not forward " + ", ".join(f"`{x}`" for x in w["missing"])) if w["missing"] else "forwards every default", **w)
        for w in wr["wrappers"]] or ["No wrapping tools found."]}

    fn_len = {}
    for length, name, path, start in fns:
        fn_len.setdefault(name, (length, path, start))
    file_len = {p: n for n, p in file_sizes}
    by = {c["name"]: c for c in crates}
    cc_by_name = {}
    for i, r in enumerate(cc["complexity"] if cc else []):
        cc_by_name.setdefault(r["name"], (r["score"], i + 1))
    misses = ms.anchor_misses()
    history = {}

    def resolve(text):
        def sub(match):
            kind, key = match.group(1), match.group(2)
            if kind in ("m", "k"):
                if key not in m:
                    misses.append(f"measure {key}")
                    return "?"
                v = m[key]
                if kind == "k":
                    return ms.k(v)
                return f"{v:,}" if isinstance(v, int) else str(len(v) if isinstance(v, list) else v)
            if kind == "e":
                if key not in ev:
                    misses.append(f"evidence {key}")
                    return "?"
                v = ev[key]
                return f"{v:,}" if isinstance(v, int) else str(v)
            if kind == "list":
                if key not in lists:
                    misses.append(f"list {key}")
                    return "?"
                return "\n".join(f"- {line}" for line in lists[key])
            if kind == "fn":
                if key in fn_len:
                    return f"{fn_len[key][0]:,}"
                misses.append(f"fn {key}")
                return "?"
            if kind == "file":
                if key in file_len:
                    return f"{file_len[key]:,}"
                misses.append(f"file {key}")
                return "?"
            if kind in ("deps", "usedby", "src", "tests", "commits"):
                c = by.get(key)
                if not c:
                    misses.append(f"crate {key}")
                    return "?"
                v = {"deps": len(c["deps"]), "usedby": len(c["used_by"]), "src": c["src"],
                     "tests": c["tests"], "commits": c["commits"]}[kind]
                return f"{v:,}"
            if kind in ("cc", "ccrank", "ccx"):
                if key not in cc_by_name:
                    misses.append(f"complexity of {key} (run deep)")
                    return "?"
                score, rank = cc_by_name[key]
                return str(score) if kind == "cc" else (f"{rank:,}" if kind == "ccrank" else f"{score / max(1, m['cc_median']):.0f}")
            if kind in ("hist", "histpct"):
                if key not in fn_len:
                    misses.append(f"fn {key}")
                    return "?"
                if key not in history:
                    length, path, start = fn_len[key]
                    history[key] = ms.fn_history(path, start, length)
                n = history[key]
                return f"{n:,}" if kind == "hist" else f"{100 * n / max(1, m['total_commits']):.0f}%"
            if kind == "pct":
                a, b = key.split("/")
                return f"{100 * m[a] / max(1, m[b]):.0f}%"
            return match.group(0)
        return PLACEHOLDER.sub(sub, text).strip()

    findings = [{k: resolve(v) if isinstance(v, str) and k not in ("id",) else v for k, v in f.items()}
                for f in content.get("finding", [])]
    keep = [{"name": x["name"], "text": resolve(x["text"])} for x in content.get("keep", [])]
    health = evidence.health(ms)
    advisories = (cc or {}).get("advisories")
    open_sections = record.open_items(ms.ROOT)
    recent, decisions_total = record.decisions(ms.ROOT, DECISIONS_SHOWN)
    specs = record.specs(ms.ROOT)
    spec_counts = {}
    for s in specs:
        spec_counts[s["status"]] = spec_counts.get(s["status"], 0) + 1
    act = record.activity(ms, m["last_date"], ACTIVITY_DAYS)

    subsystem_list, coverage, sub_review, unit_owner, route_owner = subs.build(ms, {c["name"]: c["layer"] for c in crates})
    misses.extend(sub_review)
    edges, edge_review = connections.measure(ms, unit_owner, route_owner, {c["name"]: c["deps"] for c in crates})
    misses.extend(edge_review)
    misses.extend(subs.check_flows(edges))
    sub_history, sub_since, sub_recent = subs.history(ms, m["last_date"], ACTIVITY_DAYS)
    for x in subsystem_list:
        h = sub_history.get(x["id"], {"commits": 0, "fixes": 0, "recent": 0})
        x.update(h)
        x["fix_density"] = round(1000 * h["fixes"] / x["lines"], 1) if x["lines"] else None
        x["out"] = [e for e in edges if e["from"] == x["id"]]
        x["in"] = [e for e in edges if e["to"] == x["id"]]
    flows = [{"id": fid, "name": name, "steps": [{"from": a, "to": b, "what": w} for a, b, w in steps]}
             for fid, name, steps in subs.FLOWS]
    m["subsystems"] = len(subsystem_list)
    m["connections"] = len(edges)
    m["flows"] = len(flows)
    hosted = {}
    for x in subsystem_list:
        for h in x["homes"]:
            hosted.setdefault(h, []).append(x["id"])
    unassigned = [c["name"] for c in crates if c["layer"] == "L?"]
    stale = [n for n in ms.CRATES if n not in by]
    misses.extend([f"crate {n} has no layer" for n in unassigned] + [f"crate {n} is gone" for n in stale])
    model = {
        "meta": {k: m[k] for k in ("version", "revision", "first_date", "last_date", "prose_reviewed") if k in m},
        "summary": {
            "text": resolve(content.get("summary", "")),
            "stats": {k: m[k] for k in ("rust_crates", "ts_crates", "rust_src", "rust_tests", "ts_src", "routes",
                                        "forms", "worker_tasks", "state_paths", "lock_crates", "total_commits",
                                        "days", "peak_day", "design_docs", "agents_words", "subsystems",
                                        "connections", "flows") if k in m},
            "tools": m["tools"],
            "card_tools": m["card_tools"],
        },
        "attention": {
            "findings": [{k: f[k] for k in ("id", "verdict", "title")} for f in findings],
            "health": health["checks"],
            "advisories": advisories,
            "stale_names": health["stale_names"],
            "open_items": open_sections,
            "open_total": sum(s["open"] for s in open_sections),
            "measures_revision": m.get("cc_revision"),
        },
        "activity": {"since": act["since"], "until": act["until"], "commits": len(act["commits"]),
                     "areas": act["areas"][:12],
                     "by_subsystem": sorted(((x["id"], x["name"], x["recent"]) for x in subsystem_list if x["recent"]),
                                            key=lambda r: -r[2]), "recent": act["commits"][:15],
                     "unreleased": record.unreleased(ms.ROOT),
                     "in_progress": [s for s in specs if s["status"] == "in progress"]},
        "decisions": {"recent": recent, "total": decisions_total},
        "specs": {"counts": spec_counts, "open": [s for s in specs if s["status"] in ("proposal", "in progress")]},
        "layers": [{"id": lid, "name": name, "lines": sum(c["src"] for c in crates if c["layer"] == lid),
                    "crates": [c["name"] for c in sorted(crates, key=lambda c: -c["src"]) if c["layer"] == lid]}
                   for lid, name in ms.LAYERS + ([("L?", "Unassigned")] if unassigned else [])],
        "subsystems": subsystem_list,
        "coverage": coverage,
        "connections": edges,
        "flows": flows,
        "replay": sub_recent,
        "groups": [g for g, _ in subs.GROUPS],
        "crates": [{**{k: c[k] for k in ("name", "layer", "kind", "role", "src", "tests", "files", "commits",
                                         "deps", "used_by", "upward", "ext", "largest")},
                    "subsystems": hosted.get(c["name"], [])} for c in crates],
        "size": {"files": [{"file": p, "lines": n} for n, p in file_sizes[:12]],
                 "functions": [{"name": name, "file": p, "line": s, "lines": n} for n, name, p, s in fns[:12]],
                 "complexity": (cc or {}).get("complexity", [])[:12]},
        "work": {"docs_commits": m["docs_commits"], "hot": m["hot"]},
        "findings": findings,
        "keep": keep,
        "target": resolve(content.get("target", "")),
        # the same list resolve() appends to, so names the template quotes
        # count too once render.py has resolved it
        "review": misses,
    }
    return model, resolve, m
