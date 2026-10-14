#!/bin/sh
# The verification gate (AGENTS.md "Verification before every commit"),
# with each step timed.
#
#   scripts/gate.sh            the full gate: required before every commit
#   scripts/gate.sh --changed  fmt, then clippy and tests of the crates
#                              changed against origin/main (committed and
#                              not) and every crate that depends on them:
#                              for checks while working, never in place of
#                              the full gate
set -eu
cd "$(dirname "$0")/.."

step() {
    name=$1
    shift
    start=$(date +%s)
    if "$@"; then
        echo "gate: $name ok ($(($(date +%s) - start))s)"
    else
        echo "gate: $name FAILED ($(($(date +%s) - start))s)"
        exit 1
    fi
}

if [ "${1:-}" = "--changed" ]; then
    base=${2:-origin/main}
    crates=$(python3 - "$base" <<'EOF'
import json, subprocess, sys
base = sys.argv[1]
changed = set(subprocess.run(["git", "diff", "--name-only", base], capture_output=True, text=True).stdout.split())
changed |= set(subprocess.run(["git", "ls-files", "--others", "--exclude-standard"], capture_output=True, text=True).stdout.split())
meta = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], capture_output=True, text=True, check=True).stdout)
root = meta["workspace_root"] + "/"
dirs = {p["name"]: p["manifest_path"][len(root):].rsplit("/", 1)[0] + "/" for p in meta["packages"]}
deps = {p["name"]: {d["name"] for d in p["dependencies"] if d["name"] in dirs} for p in meta["packages"]}
touched = {name for name, d in dirs.items() if any(f.startswith(d) for f in changed)}
if any(f in ("Cargo.toml", "Cargo.lock") for f in changed):
    touched = set(dirs)
affected, grew = set(touched), True
while grew:
    more = {name for name, d in deps.items() if d & affected} - affected
    affected |= more
    grew = bool(more)
print(" ".join(f"-p {name}" for name in sorted(affected)))
EOF
)
    if [ -z "$crates" ]; then
        echo "gate: no crate changed against $base"
        exit 0
    fi
    echo "gate: checking $crates"
    step records python3 scripts/check_records.py
    step fmt cargo fmt --all --check
    # shellcheck disable=SC2086
    step clippy cargo clippy $crates --all-targets -- -D warnings
    # shellcheck disable=SC2086
    step test cargo test $crates
    exit 0
fi

step fmt cargo fmt --all --check
step clippy cargo clippy --workspace --all-targets -- -D warnings
step test cargo test --workspace
step version scripts/check-version.sh
step doc-paths python3 scripts/check_doc_paths.py
step records python3 scripts/check_records.py
