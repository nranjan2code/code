"""Readers for the project's record, for the blueprint's front sheets.

Each source stays the one place its fact is written (docs/README.md, "The
record"); these readers only summarise it, and every summary links back:

- what is decided: docs/decisions.md
- what is open: docs/plans/open-items.md
- what changed for users and is not released: CHANGELOG.md, `## Unreleased`
- what each design is and whether it is built: each doc's `Status:` line
- what the last fortnight of work touched: git history

Everything is relative to the last commit the blueprint measures, never to
the wall clock, so the output depends only on the tree and its history.
"""

import os
import re
import sys
from collections import Counter, defaultdict
from datetime import date, timedelta


def _first_sentence(text, limit=None):
    text = " ".join(text.split())
    m = re.match(r"(.+?[.;:])(\s|$)", text)
    out = m.group(1) if m else text
    return out if limit is None or len(out) <= limit else out


def _bullets(body):
    """Top-level `- ` items of a Markdown body, each joined into one line."""
    items, cur = [], None
    for line in body.split("\n"):
        if line.startswith("- "):
            if cur is not None:
                items.append(cur)
            cur = line[2:].strip()
        elif cur is not None and line.startswith("  ") and line.strip():
            cur += " " + line.strip()
        elif cur is not None and not line.strip():
            items.append(cur)
            cur = None
    if cur is not None:
        items.append(cur)
    return items


def decisions(root, limit):
    """The newest decisions, newest first: date, first sentence, and the
    documents the line links to."""
    text = open(os.path.join(root, "docs/decisions.md"), encoding="utf-8").read()
    out = []
    for section in re.split(r"(?m)^## ", text)[1:]:
        day, _, body = section.partition("\n")
        for item in _bullets(body):
            links = re.findall(r"\]\(([^)\s]+)\)", item)
            out.append({"date": day.strip(), "text": _first_sentence(re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", item)),
                        "links": [os.path.normpath(os.path.join("docs", l)) for l in links if not l.startswith("http")]})
    return out[:limit], len(out)


def _checkboxes(body):
    """Every checkbox item of a Markdown body, top-level or nested, as
    (checked, text), each joined into one line."""
    items, cur = [], None
    for line in body.split("\n"):
        m = re.match(r"^(\s*)- \[([ xX])\] (.*)$", line)
        if m:
            if cur:
                items.append(cur)
            cur = [m.group(2) != " ", m.group(3).strip(), len(m.group(1))]
        elif cur and line.strip() and len(line) - len(line.lstrip()) > cur[2] and not re.match(r"^\s*- ", line):
            cur[1] += " " + line.strip()
        else:
            if cur:
                items.append(cur)
            cur = None
    if cur:
        items.append(cur)
    return [(checked, text) for checked, text, _ in items]


def open_items(root):
    """Per tracker section: open and closed counts, and the open items'
    first sentences. Nested items count as items of their own."""
    text = open(os.path.join(root, "docs/plans/open-items.md"), encoding="utf-8").read()
    sections = []
    for section in re.split(r"(?m)^## ", text)[1:]:
        title, _, body = section.partition("\n")
        items = _checkboxes(body)
        todo = [t for checked, t in items if not checked]
        done = [t for checked, t in items if checked]
        if todo or done:
            sections.append({"section": title.strip(), "open": len(todo), "closed": len(done),
                             "items": [_first_sentence(t) for t in todo]})
    return sections


def unreleased(root):
    text = open(os.path.join(root, "CHANGELOG.md"), encoding="utf-8").read()
    body = text.split("## Unreleased", 1)[1].split("\n## ", 1)[0] if "## Unreleased" in text else ""
    return [_first_sentence(i) for i in _bullets(body)]


def specs(root):
    """Every design doc and plan with its status word and summary."""
    sys.path.insert(0, os.path.join(root, "scripts"))
    import check_records as cr
    rows = []
    for path in [*cr.design_docs(), *cr.plan_docs()]:
        text = open(path, encoding="utf-8").read()
        title = cr.TITLE.search(text)
        word = cr.status_word(text) or "?"
        rows.append({"path": os.path.relpath(path, root), "title": title.group(1).strip() if title else os.path.basename(path),
                     "status": word, "summary": cr.summary(cr.status_line(text), word) if word != "?" else ""})
    return rows


def activity(b, last_date, days):
    """Commits in the `days` before the last measured commit: how many, which
    crates they touched, and their subjects."""
    since = (date.fromisoformat(last_date) - timedelta(days=days)).isoformat()
    log = b.run("git", "log", f"--since={since}T00:00:00", "--format=%h%x1f%ad%x1f%s", "--date=short", "--name-only", "--", ".", *b.SELF)
    commits, areas = [], Counter()
    cur = None
    for line in log.split("\n"):
        if "\x1f" in line:
            h, d, s = line.split("\x1f", 2)
            cur = {"hash": h, "date": d, "subject": s, "areas": set()}
            commits.append(cur)
        elif line.strip() and cur is not None:
            parts = line.split("/")
            area = parts[1] if parts[0] == "crates" and len(parts) > 1 else (parts[0] if "/" in line else "(root)")
            cur["areas"].add(area)
    for c in commits:
        areas.update(c["areas"])
        c["areas"] = sorted(c["areas"])
    return {"since": since, "until": last_date, "commits": commits, "areas": areas.most_common()}
