#!/usr/bin/env python3
"""Build the public website that vak-server serves at `/`.

Why a builder at all, for four static pages
-------------------------------------------

The front door started as one hand-written `landing.html` with its CSS and
JS inline. That is the right shape for one page and the wrong shape for
four: a shared rail, a shared footer and a shared design system copied four
times drift the moment anyone edits three of them. This turns the copies
into one source and one command.

What it emits, and why
----------------------

- CSS and `site.js` are **inlined into every page**. The front door has to
  render before, and independently of, anything else being up; an external
  stylesheet is one more round trip between "the server answered" and "the
  reader sees the product", and a theme that resolves in a second request
  flashes the wrong ground. They are small, they gzip, and there are four
  pages.
- `vendor/motion.js` is emitted as a **hashed, shared file**. It is 140 KB
  and identical on every page, so it is the one thing that must be cached
  across a visit rather than repeated inside it. It is also deferred, and
  the site is fully usable if it never arrives (see `site.js`).
- `dist/` is **committed**, like the two UI bundles, so a clone builds
  without Python or Node. `dist/.src-manifest` is the same format
  `scripts/ui_bundle_check.rs` verifies, so `cargo build` refuses to embed
  a `dist/` that no longer matches `src/` — the failure this repository has
  already been bitten by twice.

Usage
-----

    python3 crates/vak-server/site/build.py            # build
    python3 crates/vak-server/site/build.py --check    # CI: fail if stale
    python3 crates/vak-server/site/build.py --vendor-motion 13.2.0
"""

from __future__ import annotations

import argparse
import hashlib
import html
import re
import shutil
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

SITE = Path(__file__).resolve().parent
SRC = SITE / "src"
DIST = SITE / "dist"

# Order matters: it is the order of the rail, and of the reading.
PAGES = [
    # (source, route, nav label, <title>, meta description)
    (
        "index.html",
        "/",
        "Home",
        "vak — an agent you can inspect, constrain, and extend",
        "vak is a local-first Rust harness for running serious general-purpose "
        "agents without giving up the receipts. Append-only ledgers, permission "
        "before every effect, a receipt for every dispatch.",
    ),
    (
        "surfaces.html",
        "/surfaces",
        "Surfaces",
        "Surfaces — vak",
        "One auditable core behind a CLI, a desktop app, a browser client, an "
        "HTTP/SSE server and chat gateways. The same session contract, the same "
        "policy gate, the same ledger, whichever way you come in.",
    ),
    (
        "security.html",
        "/security",
        "Security",
        "Security — vak",
        "Permission before dispatch, three permission modes, OS-level sandboxing, "
        "a broker boundary for restricted tools, and secrets that never enter the "
        "agent's ambient environment.",
    ),
    (
        "install.html",
        "/install",
        "Install",
        "Install — vak",
        "Install vak on macOS or Linux, run it headless behind a browser, or in "
        "Docker. Configuration, secrets, services, updating and uninstalling.",
    ),
]

# Files that are inlined into every page rather than linked.
INLINE_CSS = SRC / "styles.css"
INLINE_JS = SRC / "site.js"
VENDOR_MOTION = SRC / "vendor" / "motion.js"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


# --- page assembly ----------------------------------------------------------


def nav_html(current_route: str) -> str:
    out = []
    for _src, route, label, _title, _desc in PAGES:
        on = ' class="on" aria-current="page"' if route == current_route else ""
        out.append(f'<a href="{route}"{on}>{html.escape(label)}</a>')
    return "\n      ".join(out)


def build_page(
    layout: str,
    body: str,
    *,
    route: str,
    title: str,
    desc: str,
    css: str,
    js: str,
    motion_src: str,
) -> str:
    # A page may declare `<!--@class: something -->` on its first line to put
    # a modifier class on <body>; that is the only per-page hook, deliberately.
    body_class = ""
    m = re.match(r"\s*<!--@class:\s*([a-z0-9 \-]+)\s*-->\s*\n", body)
    if m:
        body_class = m.group(1).strip()
        body = body[m.end() :]

    return (
        layout.replace("{{TITLE}}", html.escape(title))
        .replace("{{DESCRIPTION}}", html.escape(desc, quote=True))
        .replace("{{BODY_CLASS}}", html.escape(body_class, quote=True))
        .replace("{{NAV}}", nav_html(route))
        .replace("{{MOTION_SRC}}", motion_src)
        # Content substitutions go last: page markup may legitimately contain
        # a brace pair, and must never be scanned for placeholders.
        .replace("{{STYLES}}", css)
        .replace("{{SCRIPT}}", js)
        .replace("{{BODY}}", body)
    )


def source_files() -> list[Path]:
    return sorted(p for p in SRC.rglob("*") if p.is_file())


def build() -> dict[str, bytes]:
    """Render the whole site in memory. Nothing is written here."""
    layout = read(SRC / "layout.html")
    css = read(INLINE_CSS)
    js = read(INLINE_JS)

    motion = VENDOR_MOTION.read_bytes()
    motion_name = f"motion.{sha256(motion)[:8]}.js"
    motion_src = f"/site/{motion_name}"

    out: dict[str, bytes] = {f"site/{motion_name}": motion}

    for src_name, route, _label, title, desc in PAGES:
        body = read(SRC / "pages" / src_name)
        page = build_page(
            layout,
            body,
            route=route,
            title=title,
            desc=desc,
            css=css,
            js=js,
            motion_src=motion_src,
        )
        # `/` is dist/index.html; `/surfaces` is dist/surfaces/index.html, so
        # the server can serve a route and its trailing-slash form from one
        # file without a rewrite table.
        rel = "index.html" if route == "/" else f"{route.strip('/')}/index.html"
        out[rel] = page.encode("utf-8")

    # The manifest the Rust build script checks. Paths are relative to the
    # directory passed to `check_bundle_matches_source`, which is `site/`.
    manifest = "".join(
        f"{sha256(p.read_bytes())}  {p.relative_to(SITE).as_posix()}\n"
        for p in source_files()
    )
    out[".src-manifest"] = manifest.encode("utf-8")
    return out


# --- entry points -----------------------------------------------------------


def write(tree: dict[str, bytes]) -> None:
    if DIST.exists():
        shutil.rmtree(DIST)
    for rel, data in tree.items():
        path = DIST / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)


def check(tree: dict[str, bytes]) -> int:
    on_disk = {
        p.relative_to(DIST).as_posix(): p.read_bytes()
        for p in DIST.rglob("*")
        if p.is_file()
    }
    problems = []
    for rel in sorted(set(tree) | set(on_disk)):
        if rel not in on_disk:
            problems.append(f"missing from dist/: {rel}")
        elif rel not in tree:
            problems.append(f"left over in dist/: {rel}")
        elif tree[rel] != on_disk[rel]:
            problems.append(f"stale in dist/: {rel}")
    if problems:
        print("site/dist is not what site/src builds:", file=sys.stderr)
        for p in problems:
            print(f"  {p}", file=sys.stderr)
        print(
            "\nRun: python3 crates/vak-server/site/build.py  (and commit dist/)",
            file=sys.stderr,
        )
        return 1
    print(f"site ok — {len(tree)} files")
    return 0


def vendor_motion(version: str) -> int:
    """Re-vendor motion.dev at `version`, preserving the provenance header.

    Kept in this script so the answer to "where did that 140 KB file come
    from, and how do I update it" is one command in the repository rather
    than a paragraph in someone's memory.
    """
    url = f"https://registry.npmjs.org/framer-motion/-/framer-motion-{version}.tgz"
    print(f"fetching {url}")
    with tempfile.TemporaryDirectory() as tmp:
        tgz = Path(tmp) / "fm.tgz"
        with urllib.request.urlopen(url, timeout=120) as r:  # noqa: S310
            tgz.write_bytes(r.read())
        with tarfile.open(tgz) as tar:
            dom = tar.extractfile("package/dist/dom.js")
            licence = tar.extractfile("package/LICENSE.md")
            if dom is None or licence is None:
                print("tarball layout changed; update this function", file=sys.stderr)
                return 1
            body = dom.read().decode("utf-8")
            (SRC / "vendor" / "motion.LICENSE.md").write_bytes(licence.read())

    header = read(VENDOR_MOTION).split("*/\n", 1)[0] + "*/\n"
    header = re.sub(r"framer-motion \S+ ", f"framer-motion {version} ", header, count=1)
    VENDOR_MOTION.write_text(header + body, encoding="utf-8")
    (SRC / "vendor" / "motion.VERSION").write_text(version + "\n", encoding="utf-8")
    print(f"vendored motion.dev {version}; now run this script with no arguments")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true", help="fail if dist/ is stale")
    ap.add_argument("--vendor-motion", metavar="VERSION")
    args = ap.parse_args()

    if args.vendor_motion:
        return vendor_motion(args.vendor_motion)

    tree = build()
    if args.check:
        return check(tree)
    write(tree)
    print(f"built {len(tree)} files into {DIST.relative_to(Path.cwd()) if DIST.is_relative_to(Path.cwd()) else DIST}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
