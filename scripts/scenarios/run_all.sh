#!/usr/bin/env bash
# Run every end-to-end scenario; print the matrix; fail on any red.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FAILED=0
for s in "$HERE"/s*.sh; do
  name=$(basename "$s")
  echo "── scenario: $name"
  if BIN="${BIN:-$HERE/../../target/release/vakcoder}" bash "$s"; then
    echo "   ✓ $name"
  else
    echo "   ✗ $name"; FAILED=1
  fi
done
exit $FAILED
