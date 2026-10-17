#!/usr/bin/env bash
# Poison replay (docs/design/99-turns-that-do-not-poison.md §5): the same
# request in a clean conversation and after one turn whose calls failed and
# which wrote an unchecked claim, on one route, compared by receipts.
#
#   scripts/poison_replay.sh --provider ollama --model <model> [--request "..."] [--json]
#
# Runs in a disposable data home and workspace, removed afterwards. The
# disposable home has no provider connection: use a local route, or a key
# in the provider's environment variable. Exit 1 means a difference was
# found; read the printed receipts, then the ledger, before calling it a
# cause. Build first: cargo build -p vak --bin vak
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
bin="${VAK_BIN:-$root/target/debug/vak}"
if [ ! -x "$bin" ]; then
  echo "no vak binary at $bin; build it with: cargo build -p vak --bin vak" >&2
  exit 2
fi
scratch="$(mktemp -d "${TMPDIR:-/tmp}/vak-poison-replay.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/home" "$scratch/work"
cd "$scratch/work"
VAK_HOME="$scratch/home" "$bin" eval --poison-replay "$@"
