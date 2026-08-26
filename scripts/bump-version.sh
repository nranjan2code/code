#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
CURRENT="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -1)"

usage() {
  printf 'Usage: %s patch|minor|major|X.Y.Z\n' "$(basename "$0")"
}

[[ $# == 1 ]] || { usage >&2; exit 2; }
IFS=. read -r MAJOR MINOR PATCH <<< "$CURRENT"
case "$1" in
  patch) NEXT="$MAJOR.$MINOR.$((PATCH + 1))" ;;
  minor) NEXT="$MAJOR.$((MINOR + 1)).0" ;;
  major) NEXT="$((MAJOR + 1)).0.0" ;;
  [0-9]*.[0-9]*.[0-9]*) NEXT="$1" ;;
  *) usage >&2; exit 2 ;;
esac
[[ "$NEXT" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { printf 'error: invalid version: %s\n' "$NEXT" >&2; exit 2; }
[[ "$NEXT" != "$CURRENT" ]] || { printf 'error: version is already %s\n' "$CURRENT" >&2; exit 1; }

VAKCODER_CURRENT="$CURRENT" VAKCODER_NEXT="$NEXT" perl -0pi -e '
  $old = quotemeta($ENV{"VAKCODER_CURRENT"});
  die "workspace version not found\n" unless s/version = "$old"/version = "$ENV{"VAKCODER_NEXT"}"/;
' "$ROOT_DIR/Cargo.toml"
VAKCODER_NEXT="$NEXT" perl -0pi -e '
  die "canonical Unreleased heading not found\n" unless s/## Unreleased\n/## Unreleased\n\n## $ENV{"VAKCODER_NEXT"} — release candidate\n/;
' "$ROOT_DIR/CHANGELOG.md"

cargo metadata --manifest-path "$ROOT_DIR/Cargo.toml" --no-deps --format-version 1 >/dev/null
"$ROOT_DIR/scripts/check-release-version.sh"
printf 'bumped %s -> %s; edit the new changelog heading before tagging v%s\n' "$CURRENT" "$NEXT" "$NEXT"
