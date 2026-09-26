#!/usr/bin/env bash
# Install the vak gateway (and optionally the Telegram bridge) as a
# persistent OS service. See docs/hosting.md.
#
#   macOS:  LaunchAgent  ~/Library/LaunchAgents/com.vak.gateway.plist
#   Linux:  systemd user units ~/.config/systemd/user/vak-*.service
#
# Usage:
#   scripts/install_gateway_service.sh [WORKSPACE_DIR] [--with-telegram]
#
# Reads secrets from the canonical ~/vak-home/.env (TELEGRAM_BOT_TOKEN etc.) and pins the
# gateway bearer token via VAK_GATEWAY_TOKEN so bridges survive
# restarts. Re-run after changing .env or upgrading the binary.
set -euo pipefail

WORKSPACE="${1:-${VAK_WORKSPACE:-$HOME/vak-home}}"
WITH_TELEGRAM=false
[[ "${2:-}" == "--with-telegram" ]] && WITH_TELEGRAM=true

ENV_FILE="${VAK_ENV_FILE:-${HOME}/vak-home/.env}"
SCRIPT_DIR="$(cd -- "$(dirname -- "$0")" && pwd)"
BIN_DIR="${VAK_BIN_DIR:-$SCRIPT_DIR/../target/release}"

case "$WORKSPACE" in
  /*) ;;
  *) echo "error: workspace must be an absolute path: $WORKSPACE" >&2; exit 2 ;;
esac

# --- build -------------------------------------------------------------------
echo "→ building release binary"
cargo build --locked --release -p vak

# --- ensure pinned gateway token ---------------------------------------------
mkdir -p "$(dirname "$ENV_FILE")"
touch "$ENV_FILE"
chmod 600 "$ENV_FILE"
if ! grep -q '^VAK_GATEWAY_TOKEN=' "$ENV_FILE"; then
  NEWTOKEN="vk_$(openssl rand -hex 24 2>/dev/null || head -c 24 /dev/urandom | xxd -p)"
  echo "VAK_GATEWAY_TOKEN=${NEWTOKEN}" >> "$ENV_FILE"
  echo "→ generated VAK_GATEWAY_TOKEN in $ENV_FILE"
fi

env_kv() { grep "^$1=" "$ENV_FILE" | tail -1 | cut -d= -f2-; }

# --- macOS launchd -----------------------------------------------------------
if [[ "$(uname)" == "Darwin" ]]; then
  PLIST_DIR="$HOME/Library/LaunchAgents"
  PLIST="$PLIST_DIR/com.vak.gateway.plist"
  mkdir -p "$PLIST_DIR"

  ENVVARS=""
  # Defaults come from $ENV_FILE so .env stays the single source of truth.
  PROV="${VAK_PROVIDER:-$(env_kv VAK_PROVIDER || true)}"
  MODEL="${VAK_MODEL:-$(env_kv VAK_MODEL || true)}"
  [[ -n "$PROV" ]] && ENVVARS+=",\"VAK_PROVIDER\":\"$PROV\""
  [[ -n "$MODEL" ]] && ENVVARS+=",\"VAK_MODEL\":\"$MODEL\""
  TELEGRAM_ENV=$(env_kv TELEGRAM_BOT_TOKEN || true)
  GWTOKEN=$(env_kv VAK_GATEWAY_TOKEN)

  TMPJSON=$(mktemp /tmp/vak-gateway.XXXXXX).json
  cat > "$TMPJSON" <<PLIST
{
  "Label": "com.vak.gateway",
  "ProgramArguments": [
    "$BIN_DIR/vak", "serve", "--gateway", "--trust", "--port", "${PORT:-8901}"
  ],
  "WorkingDirectory": "$WORKSPACE",
  "EnvironmentVariables": {
    "HOME": "$HOME",
    "VAK_GATEWAY_TOKEN": "$GWTOKEN"$ENVVARS
  },
  "RunAtLoad": true,
  "KeepAlive": true,
  "StandardOutPath": "$HOME/.vak/logs/gateway.log",
  "StandardErrorPath": "$HOME/.vak/logs/gateway.log"
}
PLIST
  plutil -convert xml1 "$TMPJSON" -o "$PLIST"
  rm -f "$TMPJSON"

  # Stop any manual instance squatting on the port, then (re)load.
  pkill -f "vak serve --gateway" 2>/dev/null || true
  launchctl bootout "gui/$(id -u)/com.vak.gateway" 2>/dev/null || true
  mkdir -p "$HOME/.vak/logs"
  launchctl bootstrap "gui/$(id -u)" "$PLIST"
  echo "✓ gateway service installed (launchd)"

  if $WITH_TELEGRAM; then
    [[ -n "${TELEGRAM_ENV:-}" ]] || { echo "error: TELEGRAM_BOT_TOKEN missing in $ENV_FILE" >&2; exit 2; }
    TPLIST="$PLIST_DIR/com.vak.telegram.plist"
    TMPJ2=$(mktemp /tmp/vak-tg.XXXXXX).json
    cat > "$TMPJ2" <<TPLIST
{
  "Label": "com.vak.telegram",
  "ProgramArguments": [
    "$BIN_DIR/vak", "telegram",
    "--server", "http://127.0.0.1:${PORT:-8901}"
  ],
  "EnvironmentVariables": {
    "HOME": "$HOME",
    "VAK_GATEWAY_TOKEN": "$GWTOKEN",
    "TELEGRAM_BOT_TOKEN": "$TELEGRAM_ENV"
  },
  "RunAtLoad": true,
  "KeepAlive": true,
  "StandardOutPath": "$HOME/.vak/logs/telegram.log",
  "StandardErrorPath": "$HOME/.vak/logs/telegram.log"
}
TPLIST
    plutil -convert xml1 "$TMPJ2" -o "$TPLIST"
    rm -f "$TMPJ2"
    launchctl bootout "gui/$(id -u)/com.vak.telegram" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$TPLIST"
    echo "✓ telegram bridge service installed (launchd)"
  fi

  # The desktop process owns the menu-bar icon. A separately launchd-owned
  # tray inside Vakyartha.app prevents Finder from launching the desktop.
  if [[ "${3:-}" == "--with-tray" ]]; then
    launchctl bootout "gui/$(id -u)/com.vak.tray" 2>/dev/null || true
    rm -f "$PLIST_DIR/com.vak.tray.plist"
    echo "✓ retired separate tray controller (desktop now owns the menu bar)"
  fi

  echo "→ manage with:  launchctl kickstart -k gui/\$(id -u)/com.vak.gateway"
  exit 0
fi

# --- Linux systemd (user units) ----------------------------------------------
if command -v systemctl >/dev/null 2>&1; then
  UNIT_DIR="$HOME/.config/systemd/user"
  mkdir -p "$UNIT_DIR" "$HOME/.vak/logs"

  cat > "$UNIT_DIR/vak-gateway.service" <<UNIT
[Unit]
Description=vak gateway
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=$BIN_DIR/vak serve --gateway --trust --port ${PORT:-8901}
WorkingDirectory=$WORKSPACE
EnvironmentFile=$ENV_FILE
Restart=always
RestartSec=3

[Install]
WantedBy=default.target
UNIT

  if $WITH_TELEGRAM; then
    cat > "$UNIT_DIR/vak-telegram.service" <<UNIT
[Unit]
Description=vak telegram bridge
After=vak-gateway.service
Requires=vak-gateway.service

[Service]
ExecStartPre=/bin/sh -c 'until curl -sf http://127.0.0.1:${PORT:-8901}/health >/dev/null; do sleep 1; done'
ExecStart=$BIN_DIR/vak telegram --server http://127.0.0.1:${PORT:-8901}
# Gateway token comes from EnvironmentFile (never argv: `ps` visibility).
Environment=VAK_GATEWAY_TOKEN=${VAK_GATEWAY_TOKEN}
EnvironmentFile=$ENV_FILE
Restart=always
RestartSec=3

[Install]
WantedBy=default.target
UNIT
  fi

  systemctl --user daemon-reload
  systemctl --user enable --now vak-gateway.service
  $WITH_TELEGRAM && systemctl --user enable --now vak-telegram.service || true
  echo "✓ services installed (systemd --user)"
  echo "→ journalctl --user -u vak-gateway -f"
  exit 0
fi

echo "error: unsupported platform (need launchd or systemd)" >&2
exit 2
