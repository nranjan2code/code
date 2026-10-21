#!/bin/sh
# Known-vulnerability check of Cargo.lock against the RustSec advisory
# database (cargo-audit). Run by scripts/gate.sh and scripts/release.sh.
#
# A vulnerability fails the check. Warnings (unmaintained, unsound) are
# reported and do not. Exceptions live in .cargo/audit.toml, each with the
# reason Vak cannot reach the vulnerable code.
#
# Without network the check runs against the last fetched database and says
# so; it never passes silently on an empty one.
set -eu
cd "$(dirname "$0")/.."

# RUSTSEC-2026-0381 also flags the 0.26.11 compatibility shim, whose
# source only re-exports v1's roots. Its advisory exception is valid only
# while every real root set and the shim's resolved parent are patched.
python3 - <<'PYCODE'
import tomllib
from pathlib import Path

packages = tomllib.loads(Path("Cargo.lock").read_text())["package"]
roots = [p for p in packages if p["name"] == "webpki-roots"]
def patched(version):
    return tuple(map(int, version.split("."))) >= (1, 0, 10)
patched_versions = {p["version"] for p in roots if patched(p["version"])}
for package in roots:
    if patched(package["version"]):
        continue
    if package["version"] == "0.26.11" and any(
        f"webpki-roots {version}" in package.get("dependencies", [])
        for version in patched_versions
    ):
        continue
    raise SystemExit("check-advisories: webpki-roots must resolve to patched 1.0.10+ roots")
PYCODE

if ! cargo audit --version >/dev/null 2>&1; then
    echo "check-advisories: cargo-audit is not installed; run: cargo install cargo-audit --locked" >&2
    exit 1
fi

out=$(mktemp)
trap 'rm -f "$out"' EXIT
if cargo audit -q >"$out" 2>&1; then
    exit 0
fi
if grep -qiE "fetch|network|resolve host|could not connect|timed out" "$out"; then
    echo "check-advisories: advisory database unreachable; checking against the last fetched copy" >&2
    exec cargo audit -q --no-fetch --stale
fi
cat "$out" >&2
exit 1
