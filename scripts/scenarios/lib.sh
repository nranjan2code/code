#!/usr/bin/env bash
# Shared harness for end-to-end Runtime scenarios.
# Each scenario runs the REAL binary against the offline mock provider —
# exactly one stubbed boundary. An unmatched mock expectation fails loudly.
set -euo pipefail

SCEN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$(dirname "$SCEN_DIR")")"
BIN="${BIN:-$ROOT/target/release/vakcoder}"
PORT="${SCENARIO_PORT:-8931}"
GATEWAY_PORT="${SCENARIO_GATEWAY_PORT:-8901}"
MOCK_PID=""
GATEWAY_PID=""

start_mock() {
  python3 "$ROOT/scripts/mock_anthropic.py" "$PORT" >/dev/null 2>&1 &
  MOCK_PID=$!
  sleep 0.7
}

stop_mock() { [[ -n "${MOCK_PID:-}" ]] && kill "$MOCK_PID" 2>/dev/null || true; }
stop_gateway() { [[ -n "${GATEWAY_PID:-}" ]] && kill "$GATEWAY_PID" 2>/dev/null || true; }
cleanup() { stop_gateway; stop_mock; [[ -n "${WORK_DIR:-}" ]] && rm -rf "$WORK_DIR" 2>/dev/null || true; }
trap cleanup EXIT

new_workspace() {
  WORK_DIR="$(mktemp -d /tmp/vak-scenario.XXXXXX)"
  mkdir -p "$WORK_DIR/.vakcoder" "$WORK_DIR/home"
  printf '[permission]\nmode = "full_access"\n' > "$WORK_DIR/.vakcoder/project.toml"
}

# Offline lane: anthropic-shaped mock, pinned provider. Exported so every
# later `$BIN` invocation inherits the lane.
setup_offline() {
  export VAKCODER_PROVIDER=anthropic
  export VAKCODER_MODEL=claude-sonnet-4-5
  export ANTHROPIC_API_KEY=test
  export VAKCODER_ANTHROPIC_BASE_URL="http://127.0.0.1:$PORT"
  export VAKCODER_HOME="$WORK_DIR/home"
  export VAKCODER_GATEWAY_TOKEN="scenario-gateway-token"
  printf '[provider]\nname = "anthropic"\nendpoint = "http://127.0.0.1:%s"\n[model]\nname = "claude-sonnet-4-5"\n' "$PORT" > "$VAKCODER_HOME/config.toml"
  "$BIN" serve --gateway --port "$GATEWAY_PORT" >"$WORK_DIR/gateway.log" 2>&1 &
  GATEWAY_PID=$!
  for _ in $(seq 1 50); do
    if [[ -s "$WORK_DIR/home/runtime/gateway.json" ]]; then
      python3 - "$WORK_DIR" "$GATEWAY_PORT" <<'PY'
import json
import os
import sys
import urllib.request
import uuid

root = os.path.realpath(sys.argv[1])
port = sys.argv[2]
payload = json.dumps({"id": str(uuid.uuid4()), "root": root, "display_name": "scenario"}).encode()
request = urllib.request.Request(
    f"http://127.0.0.1:{port}/projects",
    data=payload,
    method="POST",
    headers={"authorization": "Bearer scenario-gateway-token", "content-type": "application/json"},
)
with urllib.request.urlopen(request, timeout=5) as response:
    if response.status != 201:
        raise SystemExit(f"project registration failed: {response.status}")
PY
      return 0
    fi
    sleep 0.1
  done
  echo "gateway failed to start:" >&2
  sed -n '1,120p' "$WORK_DIR/gateway.log" >&2 || true
  return 1
}

pass() { printf 'PASS: %s\n' "$1"; }
fail() { printf 'FAIL: %s\n' "$1"; SCENARIO_FAILED=1; }
