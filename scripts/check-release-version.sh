#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -1)"

if [[ -n "$(git -C "$ROOT_DIR" status --porcelain)" ]]; then
  printf 'error: release tree is dirty; commit all changes before building v%s\n' "$VERSION" >&2
  exit 1
fi

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]]; then
  printf 'error: invalid workspace version: %s\n' "$VERSION" >&2
  exit 1
fi

METADATA="$(cargo metadata --manifest-path "$ROOT_DIR/Cargo.toml" --no-deps --format-version 1)"
VAKCODER_METADATA="$METADATA" VAKCODER_VERSION="$VERSION" python3 - <<'PY'
import json
import os
import sys

metadata = json.loads(os.environ["VAKCODER_METADATA"])
expected = os.environ["VAKCODER_VERSION"]
members = set(metadata["workspace_members"])
mismatches = [
    f'{package["name"]}={package["version"]}'
    for package in metadata["packages"]
    if package["id"] in members and package["version"] != expected
]
if mismatches:
    print("error: workspace package versions disagree: " + ", ".join(mismatches), file=sys.stderr)
    raise SystemExit(1)
PY

if rg -q '"version"\s*:' "$ROOT_DIR/crates/vak-desktop/tauri.conf.json"; then
  printf 'error: tauri.conf.json must inherit the Cargo workspace version\n' >&2
  exit 1
fi
if ! rg -q "^## ${VERSION//./\\.}( |$)" "$ROOT_DIR/CHANGELOG.md"; then
  printf 'error: CHANGELOG.md has no release heading for %s\n' "$VERSION" >&2
  exit 1
fi

TAG="$(git -C "$ROOT_DIR" tag --points-at HEAD --list 'v*' | head -1)"
if [[ -n "$TAG" && "$TAG" != "v$VERSION" ]]; then
  printf 'error: HEAD tag %s disagrees with workspace version v%s\n' "$TAG" "$VERSION" >&2
  exit 1
fi

VERSION_TAG="$(git -C "$ROOT_DIR" tag --list "v$VERSION" | head -1)"
if [[ -n "$VERSION_TAG" ]]; then
  TAG_COMMIT="$(git -C "$ROOT_DIR" rev-parse "$VERSION_TAG^{commit}")"
  HEAD_COMMIT="$(git -C "$ROOT_DIR" rev-parse HEAD)"
  if [[ "$TAG_COMMIT" != "$HEAD_COMMIT" ]]; then
    printf 'error: version v%s is already tagged at %s; bump the workspace version before releasing\n' "$VERSION" "$TAG_COMMIT" >&2
    exit 1
  fi
fi

if [[ -n "${VAKCODER_CHECK_BINARIES:-}" ]]; then
  binary_list="${VAKCODER_CHECK_BINARIES//,/ }"
  for binary in $binary_list; do
    case "$binary" in
      vakcoder|vakcoder-tui) ;;
      *) printf 'error: unsupported binary version check: %s\n' "$binary" >&2; exit 2 ;;
    esac
    path="$ROOT_DIR/target/release/$binary"
    [[ -x "$path" ]] || { printf 'error: missing release binary: %s\n' "$path" >&2; exit 1; }
    actual="$($path --version | awk '{print $NF}')"
    if [[ "$actual" != "$VERSION" ]]; then
      printf 'error: %s reports %s, expected %s\n' "$path" "$actual" "$VERSION" >&2
      exit 1
    fi
  done
fi

printf 'release version consistent: %s\n' "$VERSION"
