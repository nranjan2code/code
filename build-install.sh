#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$ROOT_DIR/crates/vak-desktop"
UI_DIR="$DESKTOP_DIR/ui"
APP_NAME="VakCoder.app"
INSTALL_DIR="${VAKCODER_INSTALL_DIR:-$HOME/Applications}"

usage() {
    printf 'Usage: %s [--no-clean] [--no-install]\n' "$(basename "$0")"
    printf '\nBuilds the VakCoder desktop app, creates release bundles, and installs the macOS app.\n'
    printf '\nEnvironment:\n'
    printf '  VAKCODER_INSTALL_DIR  Installation directory (default: ~/Applications)\n'
}

CLEAN=true
INSTALL=true

while (($# > 0)); do
    case "$1" in
        --no-clean) CLEAN=false ;;
        --no-install) INSTALL=false ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'Unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

if [[ "$(uname -s)" != "Darwin" ]]; then
    printf 'This installer currently supports macOS only. The Tauri bundle was still not built.\n' >&2
    exit 1
fi

command -v cargo >/dev/null || { printf 'cargo is required.\n' >&2; exit 1; }
command -v npm >/dev/null || { printf 'npm is required.\n' >&2; exit 1; }
command -v cargo-tauri >/dev/null || {
    printf 'cargo-tauri is required. Install it with: cargo install tauri-cli --version ^2\n' >&2
    exit 1
}

cd "$ROOT_DIR"

if [[ "$CLEAN" == true ]]; then
    printf '%s\n' 'Cleaning Rust build artifacts...'
    cargo clean
fi

printf '%s\n' 'Installing UI dependencies from package-lock.json...'
(cd "$UI_DIR" && npm ci)

printf '%s\n' 'Building and bundling VakCoder...'
(cd "$DESKTOP_DIR" && cargo tauri build)

APP_PATH="$ROOT_DIR/target/release/bundle/macos/$APP_NAME"
if [[ ! -d "$APP_PATH" ]]; then
    printf 'Expected app bundle was not produced: %s\n' "$APP_PATH" >&2
    exit 1
fi

printf 'Bundle created: %s\n' "$ROOT_DIR/target/release/bundle"

if [[ "$INSTALL" == true ]]; then
    mkdir -p "$INSTALL_DIR"
    INSTALLED_APP="$INSTALL_DIR/$APP_NAME"
    rm -rf "$INSTALLED_APP"
    ditto "$APP_PATH" "$INSTALLED_APP"
    printf 'Installed: %s\n' "$INSTALLED_APP"
fi
