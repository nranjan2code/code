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

"$HOME/.local/bin/vakcoder" self uninstall --yes --no-service
test ! -e "$PREFIX"
test ! -e "$HOME/.local/bin/vakcoder"
test -d "$VAKCODER_HOME"

printf 'base install smoke passed\n'
