#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
DESKTOP_DIR="$ROOT_DIR/crates/vak-desktop"
UI_DIR="$DESKTOP_DIR/ui"
APP_NAME="VakCoder.app"
INSTALL_DIR="${VAKCODER_INSTALL_DIR:-$HOME/Applications}"

usage() {
    printf 'Usage: %s [--clean] [--install]\n' "$(basename "$0")"
    printf '\nBuilds the optional macOS desktop addon (Tauri bundle).\n'
    printf 'For the base install use scripts/build-install.sh --with-desktop.\n'
    printf '\nEnvironment:\n'
    printf '  VAKCODER_INSTALL_DIR  Installation directory (default: ~/Applications)\n'
}

CLEAN=false
INSTALL=false

while (($# > 0)); do
    case "$1" in
        --clean) CLEAN=true ;;
        --install) INSTALL=true ;;
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
    printf 'cargo-tauri is required. Install it with: cargo install tauri-cli --version 2.11.4 --locked\n' >&2
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
(cd "$DESKTOP_DIR" && cargo tauri build --bundles app)

APP_PATH="$ROOT_DIR/target/release/bundle/macos/$APP_NAME"
if [[ ! -d "$APP_PATH" ]]; then
    printf 'Expected app bundle was not produced: %s\n' "$APP_PATH" >&2
    exit 1
fi
codesign --force --deep --sign - "$APP_PATH"
codesign --verify --deep --strict "$APP_PATH"

printf 'Bundle created: %s\n' "$ROOT_DIR/target/release/bundle"

if [[ "$INSTALL" == true ]]; then
    mkdir -p "$INSTALL_DIR"
    INSTALLED_APP="$INSTALL_DIR/$APP_NAME"
    if [[ -e "/Applications/$APP_NAME" && "$INSTALLED_APP" != "/Applications/$APP_NAME" ]]; then
        printf 'Refusing to create a second app beside /Applications/%s. Set VAKCODER_INSTALL_DIR=/Applications or remove the stale copy explicitly.\n' "$APP_NAME" >&2
        exit 2
    fi
    rm -rf "$INSTALLED_APP"
    ditto "$APP_PATH" "$INSTALLED_APP"
    codesign --verify --deep --strict "$INSTALLED_APP"
    printf 'Installed: %s\n' "$INSTALLED_APP"
fi

if [[ "$INSTALL" == false ]]; then
    printf '%s\n' 'Not installed. Pass --install explicitly after reviewing the bundle.'
fi
