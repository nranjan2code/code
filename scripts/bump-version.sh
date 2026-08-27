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
else
    printf 'bumping %s → %s\n' "$current" "$target"
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

printf 'refreshing Cargo.lock…\n'
cargo check --workspace --quiet

"$ROOT_DIR/scripts/check-version.sh"
printf '\nnext: review `git diff`, commit, then scripts/release.sh\n'
