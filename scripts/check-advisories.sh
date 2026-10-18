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
