#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
WITH_TUI=false
WITH_TRAY=false
WITH_DESKTOP=false
NO_SERVICE=false
RUN_GATES=false
CLEAN_BUILD=false
PREFIX=""

usage() {
  printf 'Usage: %s [OPTIONS]\n' "$(basename "$0")"
  printf '\nBuilds and installs the headless base. Addons are opt-in.\n\n'
  printf '  --with-tui          Build and install vakcoder-tui\n'
  printf '  --with-tray         Build and install the menu-bar tray addon\n'
  printf '  --with-desktop      Build and install the macOS desktop addon\n'
  printf '  --no-service        Install program files without touching launchd/systemd\n'
  printf '  --prefix DIR        Override the managed base program root\n'
  printf '  --gates             Run fmt, clippy, and workspace tests first\n'
  printf '  --clean             Remove Cargo build output before building\n'
  printf '  -h, --help          Show this help\n\n'
  printf 'Examples:\n'
  printf '  %s --no-service                 # base files only\n' "$(basename "$0")"
  printf '  %s                             # base + gateway\n' "$(basename "$0")"
  printf '  %s --with-tui                  # base + gateway + TUI\n' "$(basename "$0")"
  printf '  %s --with-tui --with-desktop   # all local UI surfaces\n' "$(basename "$0")"
}

while (($# > 0)); do
  case "$1" in
    --with-tui) WITH_TUI=true ;;
    --with-tray) WITH_TRAY=true ;;
    --with-desktop) WITH_DESKTOP=true ;;
    --no-service) NO_SERVICE=true ;;
    --gates) RUN_GATES=true ;;
    --clean) CLEAN_BUILD=true ;;
    --prefix)
      shift
      (($# > 0)) || { printf 'error: --prefix requires a directory\n' >&2; exit 2; }
      PREFIX="$1"
      ;;
    -h|--help) usage; exit 0 ;;
    *) printf 'error: unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

if [[ "$WITH_DESKTOP" == true && "$(uname -s)" != "Darwin" ]]; then
  printf 'error: --with-desktop currently supports macOS only\n' >&2
  exit 2
fi

cd "$ROOT_DIR"
if [[ -n "$(git status --porcelain)" ]]; then
  printf 'error: install tree is dirty; commit all changes before deployment\n' >&2
  exit 1
fi
"$ROOT_DIR/scripts/check-release-version.sh"

if [[ "$CLEAN_BUILD" == true ]]; then
  cargo clean
fi

if [[ "$RUN_GATES" == true ]]; then
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
fi

GIT_SHA="$(git rev-parse HEAD 2>/dev/null || printf unknown)"
if [[ -n "$(git status --porcelain 2>/dev/null)" ]]; then
  GIT_SHA+="-dirty"
fi
printf 'Building headless base  (%s)\n' "$GIT_SHA"
VAKCODER_GIT_SHA="$GIT_SHA" cargo build --release -p vakcoder --no-default-features

INSTALL_ARGS=(self install)
if [[ -n "$PREFIX" ]]; then
  INSTALL_ARGS+=(--prefix "$PREFIX")
fi
if [[ "$NO_SERVICE" == true ]]; then
  INSTALL_ARGS+=(--no-service)
fi
target/release/vakcoder "${INSTALL_ARGS[@]}"

if [[ "$WITH_TUI" == true ]]; then
  printf 'Building standalone TUI addon\n'
  VAKCODER_GIT_SHA="$GIT_SHA" cargo build --release -p vak-tui --bin vakcoder-tui
  TUI_INSTALL_DIR="${VAKCODER_TUI_INSTALL_DIR:-$HOME/.local/bin}"
  mkdir -p "$TUI_INSTALL_DIR"
  install -m 755 target/release/vakcoder-tui "$TUI_INSTALL_DIR/vakcoder-tui"
  printf 'Installed TUI: %s\n' "$TUI_INSTALL_DIR/vakcoder-tui"
fi

if [[ "$WITH_TRAY" == true ]]; then
  printf 'Building tray addon\n'
  VAKCODER_GIT_SHA="$GIT_SHA" cargo build --release -p vak-tray --bin vakcoder-tray
  TRAY_DEST="$(dirname -- "$(readlink "$HOME/.local/bin/vakcoder")")/vakcoder-tray"
  install -m 755 target/release/vakcoder-tray "$TRAY_DEST"
  printf 'Installed tray: %s\n' "$TRAY_DEST"
  if [[ "$NO_SERVICE" != true ]]; then
    target/release/vakcoder self services-sync com.vakcoder.tray
  fi
fi

if [[ "$WITH_DESKTOP" == true ]]; then
  printf 'Building optional desktop addon\n'
  DESKTOP_INSTALL_DIR="${VAKCODER_DESKTOP_INSTALL_DIR:-}"
  if [[ -z "$DESKTOP_INSTALL_DIR" && -d /Applications/VakCoder.app ]]; then
    DESKTOP_INSTALL_DIR=/Applications
  fi
  if [[ -n "$DESKTOP_INSTALL_DIR" ]]; then
    VAKCODER_INSTALL_DIR="$DESKTOP_INSTALL_DIR" "$ROOT_DIR/build-install.sh" --install
  else
    "$ROOT_DIR/build-install.sh" --install
  fi
fi

CHECK_BINARIES=vakcoder
if [[ "$WITH_TUI" == true ]]; then
  CHECK_BINARIES+=,vakcoder-tui
fi
VAKCODER_CHECK_BINARIES="$CHECK_BINARIES" "$ROOT_DIR/scripts/check-release-version.sh"

printf '\nInstalled surfaces:\n'
printf '  Base:    %s\n' "$HOME/.local/bin/vakcoder"
printf '  Admin:   vakcoder admin\n'
if [[ "$WITH_TUI" == true ]]; then
  printf '  TUI:     vakcoder-tui\n'
fi
if [[ "$WITH_DESKTOP" == true ]]; then
  printf '  Desktop: open -a VakCoder\n'
fi
if [[ "$WITH_TRAY" == true ]]; then
  printf '  Tray:    menu bar (com.vakcoder.tray)\n'
fi
