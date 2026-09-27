#!/usr/bin/env python3
"""Prepare the embedded public site for static Vercel hosting.

Usage: python3 crates/vak-server/site/build_vercel.py OUTPUT_DIRECTORY
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
from pathlib import Path

SITE = Path(__file__).resolve().parent
REPO = SITE.parents[2]
BRAND = REPO / "crates/vak-client-ui/dist-web/assets/brand"
BRAND_FILES = (
    "icon-192.png",
    "songbird-colour.svg",
    "songbird-reverse.svg",
    "wordmark-colour.svg",
    "wordmark-reverse.svg",
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    if output == SITE / "dist" or SITE / "dist" in output.parents:
        parser.error("output must be separate from the embedded dist directory")
    if output.exists():
        parser.error("output directory already exists; choose a fresh directory")
    if not (SITE / "dist/index.html").is_file():
        parser.error("build the public site first with build.py")
    cargo = (REPO / "Cargo.toml").read_text(encoding="utf-8")
    package = cargo.split("[workspace.package]", 1)[1].split("\n[", 1)[0]
    match = re.search(r'^version\s*=\s*"([^"]+)"', package, re.MULTILINE)
    if not match:
        parser.error("workspace package version is missing")
    version = match.group(1)

    shutil.copytree(SITE / "dist", output)

    target_brand = output / "app/assets/brand"
    target_brand.mkdir(parents=True)
    for name in BRAND_FILES:
        shutil.copy2(BRAND / name, target_brand / name)

    install = output / "install/index.html"
    page = install.read_text(encoding="utf-8")
    page, count = re.subn(
        r'<div class="setup-card">\s*<span class="setup-tag">Already set up\?</span>.*?</div>',
        '<div class="setup-card">'
        '<span class="setup-tag">Already set up?</span>'
        '<h2>Pick up where you left off.</h2>'
        '<p>Open your desktop app, or run <code>vak open app</code> on the computer where Vakyartha is installed.</p>'
        '</div>',
        page,
        count=1,
        flags=re.DOTALL,
    )
    if count != 1:
        parser.error("could not replace the server-only app link")
    page, count = re.subn(
        r'The <a href="/admin">operations console</a>',
        'The operations console on your running Vakyartha server',
        page,
        count=1,
    )
    if count != 1:
        parser.error("could not replace the server-only admin link")
    install.write_text(page, encoding="utf-8")

    (output / "version").write_text(
        json.dumps({"version": version}) + "\n", encoding="utf-8"
    )
    (output / "vercel.json").write_text(
        json.dumps(
            {
                "$schema": "https://openapi.vercel.sh/vercel.json",
                "framework": None,
                "buildCommand": None,
                "installCommand": None,
                "outputDirectory": ".",
                "headers": [
                    {
                        "source": "/version",
                        "headers": [{"key": "Content-Type", "value": "application/json; charset=utf-8"}],
                    }
                ],
            },
            indent=2,
        ) + "\n",
        encoding="utf-8",
    )
    print(f"Vercel site ready: {output}")


if __name__ == "__main__":
    main()
