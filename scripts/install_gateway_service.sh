#!/usr/bin/env bash
# Install the vakcoder gateway (and optionally the Telegram bridge) as a
# persistent OS service. See docs/hosting.md.
#
#   macOS:  LaunchAgent  ~/Library/LaunchAgents/com.vakcoder.gateway.plist
#   Linux:  systemd user units ~/.config/systemd/user/vakcoder-*.service
#
# Usage:
#   scripts/install_gateway_service.sh [WORKSPACE_DIR] [--with-telegram]
#
# Reads secrets from ~/.vakcoder/.env (TELEGRAM_BOT_TOKEN etc.) and pins the
# gateway bearer token via VAKCODER_GATEWAY_TOKEN so bridges survive
# restarts. Re-run after changing .env or upgrading the binary.
set -euo pipefail

WORKSPACE="${1:-$PWD}"
WITH_TELEGRAM=false
[[ "${2:-}" == "--with-telegram" ]] && WITH_TELEGRAM=true

ENV_FILE="${HOME}/.vakcoder/.env"
BIN_DIR="$PWD/target/release"

# --- build -------------------------------------------------------------------
echo "→ building release binary"
cargo build --release -p vakcoder

# --- ensure pinned gateway token ---------------------------------------------
mkdir -p "$(dirname "$ENV_FILE")"
touch "$ENV_FILE"
chmod 600 "$ENV_FILE"
if ! grep -q '^VAKCODER_GATEWAY_TOKEN=' "$ENV_FILE"; then
  NEWTOKEN="vk_$(openssl rand -hex 24 2>/dev/null || head -c 24 /dev/urandom | xxd -p)"
  echo "VAKCODER_GATEWAY_TOKEN=${NEWTOKEN}" >> "$ENV_FILE"
  echo "→ generated VAKCODER_GATEWAY_TOKEN in $ENV_FILE"
fi

env_kv() { grep "^$1=" "$ENV_FILE" | tail -1 | cut -d= -f2-; }

# --- macOS launchd -----------------------------------------------------------
if [[ "$(uname)" == "Darwin" ]]; then
  PLIST_DIR="$HOME/Library/LaunchAgents"
  PLIST="$PLIST_DIR/com.vakcoder.gateway.plist"
  mkdir -p "$PLIST_DIR"

  ENVVARS=""
  # Defaults come from $ENV_FILE so .env stays the single source of truth.
  PROV="${VAKCODER_PROVIDER:-$(env_kv VAKCODER_PROVIDER || true)}"
  MODEL="${VAKCODER_MODEL:-$(env_kv VAKCODER_MODEL || true)}"
  [[ -n "$PROV" ]] && ENVVARS+=",\"VAKCODER_PROVIDER\":\"$PROV\""
  [[ -n "$MODEL" ]] && ENVVARS+=",\"VAKCODER_MODEL\":\"$MODEL\""
  TELEGRAM_ENV=$(env_kv TELEGRAM_BOT_TOKEN || true)
  GWTOKEN=$(env_kv VAKCODER_GATEWAY_TOKEN)

  TMPJSON=$(mktemp /tmp/vak-gateway.XXXXXX).json
  cat > "$TMPJSON" <<PLIST
{
  "Label": "com.vakcoder.gateway",
  "ProgramArguments": [
    "$BIN_DIR/vakcoder", "serve", "--gateway", "--trust", "--port", "${PORT:-8901}"
  ],
  "WorkingDirectory": "$WORKSPACE",
  "EnvironmentVariables": {
    "HOME": "$HOME",
    "VAKCODER_GATEWAY_TOKEN": "$GWTOKEN"$ENVVARS
  },
  "RunAtLoad": true,
  "KeepAlive": true,
  "StandardOutPath": "$HOME/.vakcoder/logs/gateway.log",
  "StandardErrorPath": "$HOME/.vakcoder/logs/gateway.log"
}
PLIST
  plutil -convert xml1 "$TMPJSON" -o "$PLIST"
  rm -f "$TMPJSON"

  # Stop any manual instance squatting on the port, then (re)load.
  pkill -f "vakcoder serve --gateway" 2>/dev/null || true
  launchctl bootout "gui/$(id -u)/com.vakcoder.gateway" 2>/dev/null || true
  mkdir -p "$HOME/.vakcoder/logs"
  launchctl bootstrap "gui/$(id -u)" "$PLIST"
  echo "✓ gateway service installed (launchd)"

  if $WITH_TELEGRAM; then
    [[ -n "${TELEGRAM_ENV:-}" ]] || { echo "error: TELEGRAM_BOT_TOKEN missing in $ENV_FILE" >&2; exit 2; }
    TPLIST="$PLIST_DIR/com.vakcoder.telegram.plist"
    TMPJ2=$(mktemp /tmp/vak-tg.XXXXXX).json
    cat > "$TMPJ2" <<TPLIST
{
  "Label": "com.vakcoder.telegram",
  "ProgramArguments": [
    "$BIN_DIR/vakcoder", "telegram",
    "--server", "http://127.0.0.1:${PORT:-8901}"
  ],
  "EnvironmentVariables": {
    "HOME": "$HOME",
    "VAKCODER_GATEWAY_TOKEN": "$GWTOKEN",
    "TELEGRAM_BOT_TOKEN": "$TELEGRAM_ENV"
  },
  "RunAtLoad": true,
  "KeepAlive": true,
  "StandardOutPath": "$HOME/.vakcoder/logs/telegram.log",
  "StandardErrorPath": "$HOME/.vakcoder/logs/telegram.log"
}
TPLIST
    plutil -convert xml1 "$TMPJ2" -o "$TPLIST"
    rm -f "$TMPJ2"
    launchctl bootout "gui/$(id -u)/com.vakcoder.telegram" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$TPLIST"
    echo "✓ telegram bridge service installed (launchd)"
  fi

  # Menu-bar controller (watchdog + start/stop indicators).
  if [[ "${3:-}" == "--with-tray" ]]; then
    TRAY_PLIST="$PLIST_DIR/com.vakcoder.tray.plist"
    TMPJ3=$(mktemp /tmp/vak-tray.XXXXXX).json
    cat > "$TMPJ3" <<TRAYJ
{
  "Label": "com.vakcoder.tray",
  "ProgramArguments": ["$BIN_DIR/vakcoder-tray"],
  "EnvironmentVariables": { "HOME": "$HOME" },
  "RunAtLoad": true,
  "KeepAlive": true,
  "StandardOutPath": "$HOME/.vakcoder/logs/tray.log",
  "StandardErrorPath": "$HOME/.vakcoder/logs/tray.log"
}
TRAYJ
    plutil -convert xml1 "$TMPJ3" -o "$TRAY_PLIST"
    rm -f "$TMPJ3"
    launchctl bootout "gui/$(id -u)/com.vakcoder.tray" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$TRAY_PLIST"
    echo "✓ tray controller installed (menu bar)"
  fi

  echo "→ manage with:  launchctl kickstart -k gui/\$(id -u)/com.vakcoder.gateway"
  exit 0
fi

# --- Linux systemd (user units) ----------------------------------------------
if command -v systemctl >/dev/null 2>&1; then
  UNIT_DIR="$HOME/.config/systemd/user"
  mkdir -p "$UNIT_DIR" "$HOME/.vakcoder/logs"

  cat > "$UNIT_DIR/vakcoder-gateway.service" <<UNIT
[Unit]
Description=vakcoder gateway
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=$BIN_DIR/vakcoder serve --gateway --trust --port ${PORT:-8901}
WorkingDirectory=$WORKSPACE
EnvironmentFile=$ENV_FILE
Restart=always
RestartSec=3

[Install]
WantedBy=default.target
UNIT

  if $WITH_TELEGRAM; then
    cat > "$UNIT_DIR/vakcoder-telegram.service" <<UNIT
[Unit]
Description=vakcoder telegram bridge
After=vakcoder-gateway.service
Requires=vakcoder-gateway.service

[Service]
ExecStartPre=/bin/sh -c 'until curl -sf http://127.0.0.1:${PORT:-8901}/health >/dev/null; do sleep 1; done'
ExecStart=$BIN_DIR/vakcoder telegram --server http://127.0.0.1:${PORT:-8901}
# Gateway token comes from EnvironmentFile (never argv: `ps` visibility).
Environment=VAKCODER_GATEWAY_TOKEN=${VAKCODER_GATEWAY_TOKEN}
EnvironmentFile=$ENV_FILE
Restart=always
RestartSec=3

[Install]
WantedBy=default.target
UNIT
  fi

  systemctl --user daemon-reload
  systemctl --user enable --now vakcoder-gateway.service
  $WITH_TELEGRAM && systemctl --user enable --now vakcoder-telegram.service || true
  echo "✓ services installed (systemd --user)"
  echo "→ journalctl --user -u vakcoder-gateway -f"
  exit 0
fi

echo "error: unsupported platform (need launchd or systemd)" >&2
exit 2
