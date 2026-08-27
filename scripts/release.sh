#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -1)"
GIT_SHA="$(git -C "$ROOT_DIR" rev-parse HEAD)"
if [[ -n "$(git -C "$ROOT_DIR" status --porcelain)" ]]; then
  GIT_SHA+="-dirty"
fi
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$OS" in
  darwin) PLATFORM="macos" ;;
  linux) PLATFORM="linux" ;;
  *) printf 'error: unsupported release host: %s\n' "$OS" >&2; exit 2 ;;
esac
case "$ARCH" in
  arm64|aarch64) ARCH="aarch64" ;;
  x86_64|amd64) ARCH="x86_64" ;;
  *) printf 'error: unsupported release architecture: %s\n' "$ARCH" >&2; exit 2 ;;
esac

DIST="$ROOT_DIR/dist/v$VERSION/$PLATFORM-$ARCH"
mkdir -p "$DIST"

cd "$ROOT_DIR"
if [[ -n "$(git status --porcelain)" ]]; then
  printf 'error: release tree is dirty; commit all changes before packaging\n' >&2
  exit 1
fi
"$ROOT_DIR/scripts/check-release-version.sh"
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

VAKCODER_GIT_SHA="$GIT_SHA" cargo build --release -p vakcoder --no-default-features
cp target/release/vakcoder "$DIST/vakcoder"
VAKCODER_GIT_SHA="$GIT_SHA" cargo build --release -p vak-tui --bin vakcoder-tui
cp target/release/vakcoder-tui "$DIST/vakcoder-tui"
VAKCODER_CHECK_BINARIES=vakcoder,vakcoder-tui "$ROOT_DIR/scripts/check-release-version.sh"

if [[ "$PLATFORM" == "macos" ]] && command -v cargo-tauri >/dev/null 2>&1; then
  (cd crates/vak-desktop && cargo tauri build)
  ditto "target/release/bundle/macos/VakCoder.app" "$DIST/VakCoder.app"
fi

cp README.md "$DIST/"
if [[ -f LICENSE ]]; then
  cp LICENSE "$DIST/"
else
  printf 'warning: no LICENSE file to package; Cargo.toml declares MIT\n' >&2
fi
(
  cd "$DIST"
  shasum -a 256 vakcoder vakcoder-tui > SHA256SUMS
)
tar -C "$(dirname "$DIST")" -czf "$DIST.tar.gz" "$(basename "$DIST")"

# Update feed consumed by `vakcoder self update <url>`. It points at the bare
# base binary, because the updater stages downloaded bytes directly as the
# executable -- a tarball URL here would install an unrunnable archive. Each
# host contributes its own platform key, so the file is merged rather than
# overwritten by whichever machine happens to build last.
RELEASE_BASE_URL="${VAKCODER_RELEASE_BASE_URL:-https://github.com/vakcoder/vakcoder/releases/download/v$VERSION}"
FEED="$ROOT_DIR/dist/v$VERSION/release.json"
VAKCODER_FEED="$FEED" \
VAKCODER_VERSION="$VERSION" \
VAKCODER_KEY="$PLATFORM/$ARCH" \
VAKCODER_URL="$RELEASE_BASE_URL/$PLATFORM-$ARCH/vakcoder" \
VAKCODER_SHA256="$(shasum -a 256 "$DIST/vakcoder" | awk '{print $1}')" \
python3 "$ROOT_DIR/scripts/write_release_feed.py"

printf 'release candidate: %s\n' "$DIST.tar.gz"
printf 'update feed:       %s\n' "$FEED"
printf 'git sha: %s\n' "$GIT_SHA"
