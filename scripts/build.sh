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
# shellcheck source=scripts/version.sh
source "$ROOT_DIR/scripts/version.sh"

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

# Every workspace crate inherits the release version. Cargo retains the old
# package identities after a version bump, so allowing versions to accumulate
# in one target directory grows it without bound. Preserve incremental builds
# within a version, but start a fresh cache when that shared identity changes.
VERSION="$(workspace_version)"
VERSION_STAMP="$ROOT_DIR/target/.vak-workspace-version"
PREVIOUS_VERSION=""
if [[ -f "$VERSION_STAMP" ]]; then
    PREVIOUS_VERSION="$(<"$VERSION_STAMP")"
fi
if [[ -d "$ROOT_DIR/target" && "$PREVIOUS_VERSION" != "$VERSION" ]]; then
    CLEAN=true
    if [[ -n "$PREVIOUS_VERSION" ]]; then
        printf 'workspace version changed (%s → %s); stale Cargo artifacts will be cleaned\n' \
            "$PREVIOUS_VERSION" "$VERSION"
    else
        printf 'unversioned Cargo artifacts found; establishing a bounded build cache\n'
    fi
fi

if [[ "$CLEAN" == true ]]; then
    printf '\n== clean ==\n'
    cargo clean
fi

# vak-admin-ui/dist is committed and embedded into vak-server at Cargo
# compile time. vak-server's build.rs refuses to compile a bundle that no
# longer matches its own source, so building it here is what keeps an
# ordinary `vak.sh build` working after a UI edit instead of failing with an
# instruction to go run npm.
if command -v npm >/dev/null; then
    printf '\n== admin frontend ==\n'
    ( cd "$ROOT_DIR/crates/vak-admin-ui" && npm ci --silent && npm run build --silent >/dev/null )
else
    printf '\nnpm not found — leaving crates/vak-admin-ui/dist as committed\n' >&2
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
packages=(--package vak --package vak-delivery)
if [[ "$DESKTOP" == true ]]; then
    packages+=(--package vak-desktop)
fi
if [[ "$PROFILE" == release ]]; then
    VAK_GIT_SHA="$GIT_SHA" cargo build --release "${packages[@]}"
    BIN_DIR="$ROOT_DIR/target/release"
else
    VAK_GIT_SHA="$GIT_SHA" cargo build "${packages[@]}"
    BIN_DIR="$ROOT_DIR/target/debug"
fi
mkdir -p "$ROOT_DIR/target"
printf '%s\n' "$VERSION" > "$VERSION_STAMP"

if [[ "$INSTALL" != true ]]; then
    printf '\nbuilt into %s (not installed)\n' "$BIN_DIR"
    du -sh "$ROOT_DIR/target" 2>/dev/null || true
    exit 0
fi

# The installer discovers optional sibling binaries. Do not let a desktop
# binary left by an earlier build defeat an explicit --no-desktop request.
if [[ "$DESKTOP" != true ]]; then
    rm -f -- "$BIN_DIR/vak-desktop"
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

printf '\n== disk usage ==\n'
du -sh "$ROOT_DIR/target" 2>/dev/null || true
