#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
WITH_TUI=false
WITH_TRAY=false
WITH_DESKTOP=false
WITH_TELEGRAM=false
NO_SERVICE=false
RUN_GATES=false
CLEAN_BUILD=false
CLEAN_STALE=false
PREFIX=""

usage() {
  printf 'Usage: %s [OPTIONS]\n' "$(basename "$0")"
  printf '\nBuilds and installs the headless base. Addons are opt-in.\n\n'
  printf '  --with-tui          Build and install vakcoder-tui\n'
  printf '  --with-tray         Build and install the menu-bar tray addon\n'
  printf '  --with-desktop      Build and install the macOS desktop addon\n'
  printf '  --with-telegram     Enable the Telegram service after base install\n'
  printf '  --no-service        Install program files without touching launchd/systemd\n'
  printf '  --prefix DIR        Override the managed base program root\n'
  printf '  --gates             Run fmt, clippy, and workspace tests first\n'
  printf '  --clean             Remove Cargo build output before building\n'
  printf '  --clean-stale       macOS: remove audited legacy installs after verification\n'
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
    --with-telegram) WITH_TELEGRAM=true ;;
    --no-service) NO_SERVICE=true ;;
    --gates) RUN_GATES=true ;;
    --clean) CLEAN_BUILD=true ;;
    --clean-stale) CLEAN_STALE=true ;;
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

if [[ "$WITH_TELEGRAM" == true && "$NO_SERVICE" == true ]]; then
  printf 'error: --with-telegram conflicts with --no-service\n' >&2
  exit 2
fi
if [[ "$WITH_DESKTOP" == true && "$(uname -s)" != "Darwin" ]]; then
  printf 'error: --with-desktop currently supports macOS only\n' >&2
  exit 2
fi
if [[ "$CLEAN_STALE" == true && "$(uname -s)" != "Darwin" ]]; then
  printf 'error: --clean-stale currently supports the audited macOS layout only\n' >&2
  exit 2
fi

cd "$ROOT_DIR"
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
  if [[ -z "$DESKTOP_INSTALL_DIR" && ( "$CLEAN_STALE" == true || -d /Applications/VakCoder.app ) ]]; then
    DESKTOP_INSTALL_DIR=/Applications
  fi
  if [[ -n "$DESKTOP_INSTALL_DIR" ]]; then
    VAKCODER_INSTALL_DIR="$DESKTOP_INSTALL_DIR" "$ROOT_DIR/build-install.sh" --install
  else
    "$ROOT_DIR/build-install.sh" --install
  fi
fi

if [[ "$WITH_TELEGRAM" == true ]]; then
  target/release/vakcoder self services-sync com.vakcoder.telegram
fi

if [[ "$CLEAN_STALE" == true ]]; then
  MANAGED="$HOME/.local/bin/vakcoder"
  test -x "$MANAGED"
  "$MANAGED" --version

  TRASH="$HOME/.Trash"
  mkdir -p "$TRASH"
  trash_move() {
    local source="$1"
    local label="$2"
    if [[ -e "$source" || -L "$source" ]]; then
      local destination="$TRASH/${label}-$(date +%Y%m%d-%H%M%S)"
      mv "$source" "$destination"
      printf 'Moved stale path to Trash: %s -> %s\n' "$source" "$destination"
    fi
  }

  USER_DOMAIN="gui/$(id -u)"
  launchctl bootout "$USER_DOMAIN/com.vakcoder.tray" 2>/dev/null || true
  trash_move "$HOME/Library/LaunchAgents/com.vakcoder.tray.plist" "com.vakcoder.tray.plist"
  trash_move "$HOME/Applications/VakCoder.app" "VakCoder-0.4.0.app"
  trash_move "$HOME/Library/Application Support/vakcoder/local/release" "vakcoder-release-0.7.0"
  if [[ -f "$HOME/.cargo/bin/vakcoder" ]]; then
    cp -p "$HOME/.cargo/bin/vakcoder" "$TRASH/cargo-vakcoder-$(date +%Y%m%d-%H%M%S)"
    cargo uninstall vakcoder
  fi
  trash_move "$HOME/.cargo/bin/vakcoder-tray" "cargo-vakcoder-tray"

  if [[ "$WITH_TELEGRAM" != true ]]; then
    launchctl bootout "$USER_DOMAIN/com.vakcoder.telegram" 2>/dev/null || true
    trash_move "$HOME/Library/LaunchAgents/com.vakcoder.telegram.plist" "com.vakcoder.telegram.plist"
  fi

  if [[ "$NO_SERVICE" != true ]]; then
    "$MANAGED" self services-sync com.vakcoder.gateway
    if [[ "$WITH_TELEGRAM" == true ]]; then
      "$MANAGED" self services-sync com.vakcoder.telegram
    fi
  fi

  if rg -q 'target/|/\.cargo/bin/|/\.vakcoder/|/Applications/[vV]ak[Cc]oder\.app/' \
    "$HOME/Library/LaunchAgents/com.vakcoder.gateway.plist" \
    "$HOME/Library/LaunchAgents/com.vakcoder.telegram.plist" 2>/dev/null; then
    printf 'error: a retained service still references a stale program path\n' >&2
    exit 1
  fi
  "$MANAGED" self status
  printf 'Local stale-install cleanup verified. User data was not removed.\n'
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
