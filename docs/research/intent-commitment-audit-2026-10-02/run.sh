#!/usr/bin/env bash
set -euo pipefail
root=$(git rev-parse --show-toplevel)
cd "$root"
research=docs/research/intent-commitment-audit-2026-10-02
commit_target=crates/vak-commit/tests/audit_probe_20261002.rs
core_target=crates/vak-core/tests/audit_probe_20261002.rs
if [[ -e "$commit_target" || -e "$core_target" ]]; then
  echo 'Audit probe test paths already exist; refusing to overwrite them.' >&2
  exit 1
fi
trap 'rm -f "$commit_target" "$core_target"' EXIT
cp "$research/commit_probe.rs" "$commit_target"
cp "$research/core_probe.rs" "$core_target"
cargo test -p vak-commit --test audit_probe_20261002 -- --nocapture --test-threads=1
cargo test -p vak-core --test audit_probe_20261002 -- --nocapture --test-threads=1
