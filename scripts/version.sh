#!/usr/bin/env bash
# Shared version helpers.
#
# Per docs/design/32-release-engineering.md invariant 1 ("version
# singularity"), `[workspace.package] version` in the root Cargo.toml is
# THE version, and nothing hand-syncs a copy of it:
#
#   * every crate inherits it with `version.workspace = true`;
#   * `tauri.conf.json` OMITS `version` entirely, so Tauri falls back to
#     the vak-desktop crate version;
#   * the frontend package.json files are private packages pinned to
#     0.0.0 — they carry no authoritative stamp.
#
# So there is nothing to bump but Cargo.toml, and the checker's job is to
# prove that no second stamp has reappeared.

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

# Files that must NOT carry an authoritative version stamp.
UNSTAMPED_JSON=(
    "crates/vak-desktop/ui/package.json"
    "crates/vak-admin-ui/package.json"
)

workspace_version() {
    awk '
        /^\[workspace\.package\]/ { in_section = 1; next }
        /^\[/                     { in_section = 0 }
        in_section && /^version[[:space:]]*=/ {
            # Anchored trims, not a greedy gsub: `.*"` would consume
            # through the closing quote and yield an empty string.
            sub(/^version[[:space:]]*=[[:space:]]*"/, "")
            sub(/".*$/, "")
            print
            exit
        }
    ' "$ROOT_DIR/Cargo.toml"
}

json_version() {
    awk -F'"' '/^[[:space:]]*"version"[[:space:]]*:/ { print $4; exit }' "$1"
}

lock_version() {
    awk '
        /^name = "vakcoder"$/ {
            getline
            sub(/^version[[:space:]]*=[[:space:]]*"/, "")
            sub(/".*$/, "")
            print
            exit
        }' "$ROOT_DIR/Cargo.lock"
}

is_semver() {
    [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]
}
