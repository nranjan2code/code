#!/usr/bin/env bash
# Set THE version and refresh the lockfile.
#
# There is exactly one place to change, by design: nothing mirrors this
# value (see scripts/version.sh). The lockfile is refreshed in the same
# run so the commit leaves a clean, buildable tree.
#
# Usage: scripts/bump-version.sh <semver>

set -Eeuo pipefail
# shellcheck source=scripts/version.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/version.sh"

target="${1:-}"
current="$(workspace_version)"
if [[ -z "$target" ]]; then
    printf 'usage: %s <semver>   (current: %s)\n' "$(basename "$0")" "$current" >&2
    exit 2
fi
if ! is_semver "$target"; then
    printf 'error: %s is not a semver version\n' "$target" >&2
    exit 2
fi
if [[ "$target" == "$current" ]]; then
    printf 'already at %s\n' "$current"
    VERSION_CHANGED=false
else
    printf 'bumping %s → %s\n' "$current" "$target"
    VERSION_CHANGED=true
fi

cd "$ROOT_DIR"
tmp="$(mktemp)"
awk -v v="$target" '
    /^\[workspace\.package\]/ { in_section = 1; print; next }
    /^\[/                     { in_section = 0 }
    in_section && !done && /^version[[:space:]]*=/ {
        print "version = \"" v "\""
        done = 1
        next
    }
    { print }
' Cargo.toml > "$tmp"
mv "$tmp" Cargo.toml
printf '  ✓ Cargo.toml\n'

if [[ "$VERSION_CHANGED" == true && -d "$ROOT_DIR/target" ]]; then
    printf 'removing Cargo artifacts invalidated by the workspace version…\n'
    cargo clean --target-dir "$ROOT_DIR/target"
fi

# The README badge is the version a person reads first, and it is the one
# stamp check-version.sh cannot let drift silently. It is derived here rather
# than hand-edited, so "one place to change" stays true.
if [[ -n "$(sed -n 's|.*/badge/version-\([0-9][0-9.]*\)-.*|\1|p' README.md | head -1)" ]]; then
    tmp="$(mktemp)"
    sed "s|badge/version-[0-9][0-9.]*-|badge/version-${target}-|" README.md > "$tmp"
    mv "$tmp" README.md
    printf '  ✓ README badge\n'
else
    printf '  ! README has no version badge to update\n' >&2
fi

# Resolving metadata refreshes path-package versions in Cargo.lock without
# compiling a complete new workspace generation merely to update the lockfile.
printf 'refreshing Cargo.lock metadata…\n'
HOST_TARGET="$(rustc -vV | awk '/^host:/ { print $2 }')"
cargo metadata --format-version 1 --filter-platform "$HOST_TARGET" >/dev/null

"$ROOT_DIR/scripts/check-version.sh"
printf '\nnext: review `git diff`, commit, then scripts/release.sh\n'
