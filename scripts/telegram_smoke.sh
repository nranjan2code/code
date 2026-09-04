#!/usr/bin/env bash
# Live Telegram end-to-end smoke (docs/design/22-gateway.md G1).
# Requires: TELEGRAM_BOT_TOKEN in env or ~/.vak/.env, and a running
# gateway. Sends a text message to YOUR chat via the bot? No — the bridge
# pulls messages FROM Telegram; so this script starts both processes and
# waits for you to message the bot, printing what happens.
set -euo pipefail

PORT="${PORT:-8901}"
TOKEN_FILE="${HOME}/.vak/.env"

if [[ -z "${TELEGRAM_BOT_TOKEN:-}" ]]; then
  if [[ -f "$TOKEN_FILE" ]] && grep -q '^TELEGRAM_BOT_TOKEN=' "$TOKEN_FILE"; then
    TELEGRAM_BOT_TOKEN="$(grep '^TELEGRAM_BOT_TOKEN=' "$TOKEN_FILE" | cut -d= -f2-)"
  else
    echo "error: TELEGRAM_BOT_TOKEN not set (env or $TOKEN_FILE)" >&2
    exit 2
  fi
fi

BIN="${BIN:-target/debug/vak}"
if [[ ! -x "$BIN" ]]; then
  echo "building..."
  cargo build --locked -p vak
fi

cleanup() {
  [[ -n "${BRIDGE_PID:-}" ]] && kill "$BRIDGE_PID" 2>/dev/null || true
  [[ -n "${SERVER_PID:-}" ]] && kill "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT

echo "→ starting gateway on :$PORT"
"$BIN" serve --gateway --trust --port "$PORT" &
SERVER_PID=$!

sleep 1
TOKEN=$(grep -m1 'auth token:' /dev/null 2>/dev/null || true)
echo "NOTE: copy the bearer token printed above by the server." >&2

read -r -p "paste gateway token: " GW_TOKEN

echo "→ starting telegram bridge"
TELEGRAM_BOT_TOKEN="$TELEGRAM_BOT_TOKEN" \
  "$BIN" telegram --server "http://127.0.0.1:$PORT" --token "$GW_TOKEN" &
BRIDGE_PID=$!

echo
echo "Ready. Send a message (or a photo) to your bot on Telegram."
echo "Watch this terminal for [telegram] lines; reply arrives on your phone."
wait
