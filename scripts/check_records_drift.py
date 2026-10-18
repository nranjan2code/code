"""Drift checks for scripts/check_records.py: claims in the record that the
tree can contradict.

Each check returns a list of problems, one line each, naming the file and
what to do. The record's rules are in docs/README.md ("The record").
"""

import glob
import os
import re
import subprocess

# Markdown files that are rules, logs, generated output or entry points
# rather than documents with a status. Each is kept true some other way:
# AGENTS.md by review, the CHANGELOG and decision log by their own rules,
# the tracker and handover by check_records.py, generated files by the tests
# that generate them.
NO_STATUS = {
    "AGENTS.md", "CHANGELOG.md", "README.md", "SECURITY.md",
    "docs/README.md", "docs/decisions.md", "docs/plans/open-items.md",
    "docs/design/INDEX.md", "docs/reference/records.md", "blueprint/README.md",
}
# Markdown allowed at the repository root. Anything else belongs in docs/.
ROOT_DOCS = {"AGENTS.md", "CHANGELOG.md", "README.md", "SECURITY.md", "DESIGN.md", "PRODUCT.md"}
# Folders of dated records: what was found, reviewed or written on a day.
# Their findings that are still open belong in docs/plans/open-items.md.
RECORD_DIRS = ("docs/audits/", "docs/reviews/", "docs/research/", "docs/linkedin/")
RECORD_WORDS = ("historical", "reference", "superseded")
NUMBER_WORDS = r"(?:twenty|thirty|forty|fifty)(?:[- ](?:one|two|three|four|five|six|seven|eight|nine))?"


def tracked_markdown(root):
    out = subprocess.run(["git", "ls-files", "*.md"], cwd=root, capture_output=True, text=True).stdout.split()
    untracked = subprocess.run(["git", "ls-files", "--others", "--exclude-standard", "*.md"], cwd=root, capture_output=True, text=True).stdout.split()
    return sorted(p for p in set(out + untracked) if os.path.exists(os.path.join(root, p)))


def record_docs(root):
    """The documents the record covers: docs/**, blueprint/**, and Markdown
    at the root."""
    return [p for p in tracked_markdown(root) if p.startswith(("docs/", "blueprint/")) or "/" not in p]


def is_living(path, word):
    """A document that must describe today: not a dated record, not history."""
    if word in ("historical", "superseded") or path.startswith(RECORD_DIRS):
        return False
    return not re.search(r"20\d\d-\d\d-\d\d", path)


def check_status(root, status_word, words):
    problems = []
    for p in record_docs(root):
        name = os.path.basename(p)
        if p in NO_STATUS or name == "AGENTS.md" or name.startswith("handover-"):
            continue
        if "/" not in p and p not in ROOT_DOCS:
            problems.append(f"{p}: only {', '.join(sorted(ROOT_DOCS))} live at the root; move it into docs/ with a Status line")
            continue
        text = open(os.path.join(root, p), encoding="utf-8").read()
        word = status_word(text)
        if word is None:
            problems.append(f"{p}: add a `Status:` line starting with one of: {', '.join(words)}")
        elif p.startswith(RECORD_DIRS) and word not in RECORD_WORDS:
            problems.append(f"{p}: a dated record's Status is historical (or reference for a method it describes); move open work to docs/plans/open-items.md")
    return problems


def check_versions(root, status_word, version, baseline):
    """Living documents may not state a current version or baseline other
    than the workspace's. Better still, they state none and point to
    Cargo.toml."""
    major = version.split(".")[0]
    problems = []
    for p in record_docs(root):
        text = open(os.path.join(root, p), encoding="utf-8").read()
        if not is_living(p, status_word(text)) or p == "CHANGELOG.md":
            continue
        for m in re.finditer(r"(?i)what'?s new in (\d+)\.x", text):
            if m.group(1) != major:
                problems.append(f"{p}: \"{m.group(0)}\" but the workspace is {version}")
        for m in re.finditer(r"(?i)(?:current|workspace) version[:` ]+`?(\d+\.\d+\.\d+)", text):
            if m.group(1) != version:
                problems.append(f"{p}: names version {m.group(1)} as current; the workspace is {version} (say \"see Cargo.toml\" instead)")
        for m in re.finditer(r"(?i)(?:shipped behaviou?r|describes[^.\n]{0,40}) at (?:workspace )?version (\d+\.\d+\.\d+)", text):
            if m.group(1).split(".")[0] != major:
                problems.append(f"{p}: describes itself at {m.group(1)}; recheck it against {version} or mark it historical")
        for m in re.finditer(r"(?i)supported (\d+\.\d+\.\d+) baseline|baseline is `?(\d+\.\d+\.\d+)", text):
            v = m.group(1) or m.group(2)
            if v != baseline:
                problems.append(f"{p}: names baseline {v}; the supported baseline is {baseline} (vak_core::baseline)")
    return problems


def check_counts(root, status_word, crates):
    """Living documents may not count crates unless the count is right."""
    problems = []
    rx = re.compile(rf"(?i)\b(\d+|about \w+|{NUMBER_WORDS})\s+(?:rust\s+)?(crates|workspace members)\b")
    for p in record_docs(root):
        text = open(os.path.join(root, p), encoding="utf-8").read()
        if not is_living(p, status_word(text)):
            continue
        for m in rx.finditer(text):
            n = m.group(1)
            if not n.isdigit() or int(n) not in (len(crates), len([c for c in crates if os.path.exists(os.path.join(root, 'crates', c, 'Cargo.toml'))])):
                problems.append(f"{p}: \"{m.group(0)}\" but crates/ has {len(crates)}; drop the count or make it exact")
    return problems


def check_layout(root, crates):
    """AGENTS.md's layout map names every crate."""
    agents = open(os.path.join(root, "AGENTS.md"), encoding="utf-8").read()
    listed = set(re.findall(r"^crates/([\w-]+)\s", agents, re.M))
    return [f"AGENTS.md: the Layout map has no row for crates/{c}" for c in crates if c not in listed]


def check_roadmaps(root, status_word):
    """A section that presents work as not built links no document whose
    status says it is built."""
    problems = []
    heading = re.compile(r"(?im)^#{1,6} [^\n]*\b(roadmap|not (?:yet )?(?:built|shipped)|planned|coming)\b[^\n]*$")
    for p in record_docs(root):
        text = open(os.path.join(root, p), encoding="utf-8").read()
        if not is_living(p, status_word(text)):
            continue
        for h in heading.finditer(text):
            level = len(text[h.start():].split(" ", 1)[0])
            nxt = re.compile(rf"(?m)^#{{1,{level}}} ").search(text, h.end())
            body = text[h.end(): nxt.start() if nxt else len(text)]
            for link in re.findall(r"\]\(([^)#\s]+\.md)", body):
                target = os.path.normpath(os.path.join(os.path.dirname(p), link))
                full = os.path.join(root, target)
                if os.path.exists(full) and status_word(open(full, encoding="utf-8").read()) == "implemented":
                    problems.append(f"{p}: \"{h.group(0).strip('# ')}\" links {target}, whose Status is implemented")
    return problems


def cli_commands(root):
    """Top-level `vak` subcommands, read from the clap enum in source."""
    src = open(os.path.join(root, "crates/vak/src/cli.rs"), encoding="utf-8").read()
    body = re.search(r"pub\(crate\) enum Command \{(.*?)\n\}", src, re.S).group(1)
    names = {"help"}
    for v in re.findall(r"^    ([A-Z]\w*)", body, re.M):
        names.add(re.sub(r"(?<!^)(?=[A-Z])", "-", v.rstrip("_")).lower())
    return names


def check_commands(root, status_word):
    """Every `vak <command>` a living document shows is a real command: an
    inline code span that starts with it, or a shell line in a fenced block
    that runs it."""
    known = cli_commands(root)
    inline = re.compile(r"(?<!`)`(?!`)(?:\$ )?(?:[\w./-]*/)?vak ([a-z][\w-]*)")
    # a sentence that says the command is gone is history, not a claim
    gone = re.compile(r"(?i)\b(retired|removed|withdrawn|replaced)\b")
    shell = re.compile(r"^\s*(?:\$\s+)?(?:[\w./-]*/)?vak ([a-z][\w-]*)")
    problems = []
    for p in record_docs(root):
        text = open(os.path.join(root, p), encoding="utf-8").read()
        word = status_word(text)
        # a proposal's commands are targets; the CHANGELOG is history
        if not is_living(p, word) or word == "proposal" or p == "CHANGELOG.md":
            continue
        names = []
        for m in inline.finditer(text):
            start = max(text.rfind(". ", 0, m.start()), text.rfind("\n\n", 0, m.start()))
            end = text.find(". ", m.end())
            if not gone.search(text[start:end if end != -1 else len(text)]):
                names.append(m.group(1))
        lang = None
        for line in text.split("\n"):
            fence = re.match(r"^\s*```(\S*)", line)
            if fence:
                lang = None if lang is not None else fence.group(1)
                continue
            if lang in ("", "bash", "sh", "shell", "console", "zsh", "text"):
                m = shell.match(line)
                if m:
                    names.append(m.group(1))
        for n in sorted(set(names) - known):
            problems.append(f"{p}: shows `vak {n}`, which is not a vak command (crates/vak/src/cli.rs)")
    return problems


def crate_dirs(root):
    return sorted(d for d in os.listdir(os.path.join(root, "crates"))
                  if os.path.exists(os.path.join(root, "crates", d, "Cargo.toml"))
                  or os.path.exists(os.path.join(root, "crates", d, "package.json")))


def all_problems(root, status_word, words):
    cargo = open(os.path.join(root, "Cargo.toml"), encoding="utf-8").read()
    version = re.search(r'^version = "([^"]+)"', cargo, re.M).group(1)
    base = re.search(r'BASELINE: &str = "([^"]+)"', open(os.path.join(root, "crates/vak-core/src/baseline.rs"), encoding="utf-8").read()).group(1)
    crates = crate_dirs(root)
    return (check_status(root, status_word, words) + check_versions(root, status_word, version, base)
            + check_counts(root, status_word, crates) + check_layout(root, crates)
            + check_roadmaps(root, status_word) + check_commands(root, status_word))


def missing_names(root, status_word):
    """Report, not a check: backticked code names (snake_case identifiers,
    `Type::name` paths, `name()` calls) in living documents that occur
    nowhere in crates/ or scripts/. External API names, names a sentence
    marks as removed, and test names a plan proposed show up here too, so
    each needs a person's judgement; `check_records.py --names` lists them."""
    out = subprocess.run(["git", "grep", "-h", "-I", "-e", ".", "--", "crates/*.rs", "crates/*.ts",
                          "crates/*.tsx", "crates/*.toml", "crates/*.json", "scripts/*",
                          "blueprint/harness/*", "blueprint/content.toml"],
                         cwd=root, capture_output=True, text=True).stdout
    words = set(re.findall(r"[A-Za-z_][A-Za-z0-9_]{3,}", out))
    code = re.compile(r"(?<!`)`([A-Za-z_]\w*(?:::[A-Za-z_]\w*)*)(?:\(\))?`")
    report = []
    for p in record_docs(root):
        text = open(os.path.join(root, p), encoding="utf-8").read()
        word = status_word(text)
        # the CHANGELOG is history; the blueprint's README only repeats the record
        if not is_living(p, word) or word == "proposal" or p in ("CHANGELOG.md", "blueprint/README.md"):
            continue
        gone = set()
        for m in code.finditer(text):
            last = m.group(1).split("::")[-1]
            looks = "::" in m.group(1) or m.group(0).endswith("()`") or ("_" in last and last == last.lower())
            if looks and len(last) > 3 and last not in words:
                gone.add(m.group(1))
        if gone:
            report.append((p, sorted(gone)))
    return sorted(report, key=lambda r: -len(r[1]))
