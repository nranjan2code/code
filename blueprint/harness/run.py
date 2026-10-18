#!/usr/bin/env python3
"""The blueprint's one entry point.

    python3 blueprint/harness/run.py [build]   measure, read the record, write
                                               README.md, blueprint.html and
                                               blueprint.json (seconds)
    python3 blueprint/harness/run.py deep      also take the slow measures
                                               (clippy complexity, cargo audit)
                                               into measures.json, then build
    python3 blueprint/harness/run.py check     print the record's health and
                                               exit 1 if a check fails
    python3 blueprint/harness/run.py names     list the code names living
                                               documents cite that no longer
                                               exist, for triage

`--quiet` prints nothing but warnings. Rules: blueprint/AGENTS.md.
"""

import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import evidence  # noqa: E402
import measure as ms  # noqa: E402
import model  # noqa: E402
import render  # noqa: E402

VERBS = ("build", "deep", "check", "names")


def write(rel, text):
    with open(os.path.join(ms.ROOT, rel), "w", encoding="utf-8") as fh:
        fh.write(text)


def build(deep, quiet):
    data, resolve, m = model.build(deep=deep)
    page = render.page(data, resolve)  # resolves the template, which may add review items
    write("blueprint/blueprint.html", page)
    write("blueprint/README.md", render.markdown(data))
    write("blueprint/blueprint.json", render.as_json(data))
    if not quiet:
        att = data["attention"]
        failing = [c["check"] for c in att["health"] if not c["ok"]]
        print(f"blueprint: {len(data['crates'])} crates, {m['rust_src']:,} Rust lines, revision {m['revision']}; "
              f"{len(data['findings'])} findings, {att['open_total']} open items"
              + (f"; failing checks: {', '.join(failing)}" if failing else ""))
    for r in data["review"]:
        print(f"blueprint: needs review: {r}", file=sys.stderr)


def check():
    health = evidence.health(ms)
    bad = False
    for c in health["checks"]:
        print(f"{'ok     ' if c['ok'] else 'FAILING'} {c['check']}")
        for p in c["problems"]:
            print(f"          {p}")
        bad |= not c["ok"]
    sn = health["stale_names"]
    print(f"report  {sn['names']} cited code names missing in {sn['documents']} living documents (run.py names)")
    cc = ms.load_measures()
    adv = (cc or {}).get("advisories")
    print("report  advisories: " + ("not measured (run.py deep)" if adv is None else
                                     f"{len(adv['vulnerabilities'])} vulnerabilities not excepted, {adv['warnings']} warnings, at {cc['revision']}"))
    return 1 if bad else 0


def main(argv):
    quiet = "--quiet" in argv
    args = [a for a in argv if not a.startswith("--")]
    verb = args[0] if args else "build"
    if verb not in VERBS:
        print(__doc__)
        return 2
    if verb in ("build", "deep"):
        build(verb == "deep", quiet)
        return 0
    if verb == "check":
        return check()
    return subprocess.run(["python3", "scripts/check_records.py", "--names"], cwd=ms.ROOT).returncode


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
