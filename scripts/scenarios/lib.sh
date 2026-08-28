#!/usr/bin/env bash
# Shared harness for end-to-end scenarios (docs/design/27 Phase F).
# Each scenario runs the REAL binary against the offline mock provider —
# exactly one stubbed boundary. An unmatched mock expectation fails loudly.
set -euo pipefail

SCEN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$(dirname "$SCEN_DIR")")"
BIN="${BIN:-$ROOT/target/release/vak}"
PORT="${SCENARIO_PORT:-8931}"
MOCK_PID=""

start_mock() {
  python3 "$ROOT/scripts/mock_anthropic.py" "$PORT" >/dev/null 2>&1 &
  MOCK_PID=$!
  sleep 0.7
}

stop_mock() { [[ -n "${MOCK_PID:-}" ]] && kill "$MOCK_PID" 2>/dev/null || true; }
cleanup() { stop_mock; [[ -n "${WORK_DIR:-}" ]] && rm -rf "$WORK_DIR" 2>/dev/null || true; }
trap cleanup EXIT

new_workspace() {
  WORK_DIR="$(mktemp -d /tmp/vak-scenario.XXXXXX)"
  mkdir -p "$WORK_DIR/.vak" "$WORK_DIR/home"
  echo 'permission_mode = "full-access"' > "$WORK_DIR/.vak/config.toml"
}

# Offline lane: anthropic-shaped mock, pinned provider. Exported so every
# later `$BIN` invocation inherits the lane.
setup_offline() {
  export VAK_PROVIDER=anthropic
  export VAK_MODEL=claude-sonnet-4-5
  export ANTHROPIC_API_KEY=test
  export VAK_ANTHROPIC_BASE_URL="http://127.0.0.1:$PORT"
  export VAK_HOME="$WORK_DIR/home"
}

pass() { printf 'PASS: %s\n' "$1"; }
fail() { printf 'FAIL: %s\n' "$1"; SCENARIO_FAILED=1; }
