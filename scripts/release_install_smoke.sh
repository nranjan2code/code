#!/usr/bin/env bash

set -Eeuo pipefail

if (($# != 1)); then
  printf 'usage: %s /path/to/vakcoder\n' "$(basename "$0")" >&2
  exit 2
fi

BASE_BIN="$(cd -- "$(dirname -- "$1")" && pwd)/$(basename -- "$1")"
test -x "$BASE_BIN" || { printf 'not executable: %s\n' "$BASE_BIN" >&2; exit 2; }

SMOKE_ROOT="$(mktemp -d)"
trap 'rm -rf "$SMOKE_ROOT"' EXIT
export HOME="$SMOKE_ROOT/home"
export VAKCODER_HOME="$SMOKE_ROOT/state"
mkdir -p "$HOME"

PREFIX="$SMOKE_ROOT/programs"
"$BASE_BIN" self install --prefix "$PREFIX" --no-service
test -x "$PREFIX/current/bin/vakcoder"
test -L "$HOME/.local/bin/vakcoder"
test -f "$PREFIX/install.json"
test -f "$VAKCODER_HOME/install-root.json"
"$HOME/.local/bin/vakcoder" --version
"$HOME/.local/bin/vakcoder" self status >/dev/null
test ! -e "$PREFIX/current/bin/vakcoder-tui"
test ! -e "$PREFIX/current/bin/vak-desktop"
test ! -e "$PREFIX/current/bin/vakcoder-tray"

# The gateway service starts unattended and cannot prompt, so install must
# leave a usable bearer token behind — an install that does not is a service
# that crash-loops forever.
test -f "$VAKCODER_HOME/.env"
# Values are rendered quoted by vak-config's env writer.
grep -qE '^VAKCODER_GATEWAY_TOKEN="[0-9a-f]{64}"$' "$VAKCODER_HOME/.env"
test "$(stat -f '%Lp' "$VAKCODER_HOME/.env" 2>/dev/null || stat -c '%a' "$VAKCODER_HOME/.env")" = 600

# Reinstall must be idempotent and must not rotate a credential that live
# surfaces are already holding.
TOKEN_BEFORE="$(grep '^VAKCODER_GATEWAY_TOKEN=' "$VAKCODER_HOME/.env")"
"$BASE_BIN" self install --prefix "$PREFIX" --no-service >/dev/null
test "$(grep '^VAKCODER_GATEWAY_TOKEN=' "$VAKCODER_HOME/.env")" = "$TOKEN_BEFORE"

# A fresh install has no provider credential. The gateway must still come up,
# because the console that sets that credential is served by this process.
"$HOME/.local/bin/vakcoder" serve --gateway --port 8929 >"$SMOKE_ROOT/gateway.log" 2>&1 &
GATEWAY_PID=$!
for _ in $(seq 1 40); do
  test -f "$VAKCODER_HOME/runtime/gateway.json" && break
  sleep 0.25
done
test -f "$VAKCODER_HOME/runtime/gateway.json" || {
  printf 'gateway never published a receipt:\n' >&2
  cat "$SMOKE_ROOT/gateway.log" >&2
  exit 1
}
test -f "$VAKCODER_HOME/locks/runtime.lock"
"$HOME/.local/bin/vakcoder" doctor >/dev/null || true
"$HOME/.local/bin/vakcoder" admin --print >/dev/null

# A second gateway must be refused rather than stealing the receipt.
if "$HOME/.local/bin/vakcoder" serve --gateway --port 8930 >/dev/null 2>&1; then
  printf 'a second gateway was allowed to start\n' >&2
  exit 1
fi

# SIGTERM is what launchd and systemd send; it must release both artifacts.
kill -TERM "$GATEWAY_PID"
wait "$GATEWAY_PID" 2>/dev/null || true
test ! -e "$VAKCODER_HOME/runtime/gateway.json"
test ! -e "$VAKCODER_HOME/locks/runtime.lock"

"$HOME/.local/bin/vakcoder" self uninstall --yes --no-service
test ! -e "$PREFIX"
test ! -e "$HOME/.local/bin/vakcoder"
test -d "$VAKCODER_HOME"

printf 'base install smoke passed\n'
