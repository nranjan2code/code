#!/usr/bin/env bash
# Build Vak and, optionally, place it through the managed installer.
#
# Placement is deliberately NOT done by this script. `vak self
# install` owns the install root, writes the manifest, records a digest
# per component, and is what `status`, `verify`, `update`, and
# `uninstall` read. A script that copied a bundle into place behind the
# installer's back produced an app the installer could not see, at a path
# that — on a case-insensitive volume — was the same directory the
# installer used. Reinstall and uninstall could not work. One writer only.
#
# Usage: scripts/build.sh [--no-install] [--no-desktop] [--clean]
#                         [--prefix DIR] [--release|--debug]

set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

INSTALL=true
DESKTOP=true
CLEAN=false
PROFILE=release
PREFIX=""

while (($# > 0)); do
    case "$1" in
        --no-install) INSTALL=false ;;
        --no-desktop) DESKTOP=false ;;
        --clean) CLEAN=true ;;
        --debug) PROFILE=debug ;;
        --release) PROFILE=release ;;
        --prefix) PREFIX="${2:-}"; shift ;;
        -h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
    shift
done

cd "$ROOT_DIR"
command -v cargo >/dev/null || { printf 'cargo is required.\n' >&2; exit 1; }

# Versions must agree before anything is stamped into a bundle.
"$ROOT_DIR/scripts/check-version.sh"

if [[ "$CLEAN" == true ]]; then
    printf '\n== clean ==\n'
    cargo clean
fi

# The desktop frontend is embedded into the bundle's Resources by the
# installer, so it has to exist before `self install` runs.
if [[ "$DESKTOP" == true ]]; then
    if command -v npm >/dev/null; then
        printf '\n== desktop frontend ==\n'
        (cd "$ROOT_DIR/crates/vak-desktop/ui" && npm ci --silent && npm run build --silent)
    else
        printf '\nnpm not found — skipping the desktop frontend\n' >&2
        DESKTOP=false
    fi
fi

printf '\n== build (%s) ==\n' "$PROFILE"
GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
if [[ "$PROFILE" == release ]]; then
    VAK_GIT_SHA="$GIT_SHA" cargo build --release --workspace
    BIN_DIR="$ROOT_DIR/target/release"
else
    VAK_GIT_SHA="$GIT_SHA" cargo build --workspace
    BIN_DIR="$ROOT_DIR/target/debug"
fi

if [[ "$INSTALL" != true ]]; then
    printf '\nbuilt into %s (not installed)\n' "$BIN_DIR"
    exit 0
fi

printf '\n== install ==\n'
# One writer: the freshly built CLI installs itself and its siblings.
install_args=(self install --force)
[[ -n "$PREFIX" ]] && install_args+=(--prefix "$PREFIX")
"$BIN_DIR/vak" "${install_args[@]}"

printf '\n== verify ==\n'
verify_args=(self verify)
[[ -n "$PREFIX" ]] && verify_args+=(--prefix "$PREFIX")
"$BIN_DIR/vak" "${verify_args[@]}"
