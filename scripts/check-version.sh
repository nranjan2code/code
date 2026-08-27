#!/usr/bin/env bash
# Prove version singularity: one authoritative version, and no second
# stamp that could drift from it. Run by release.sh before any build.

set -Eeuo pipefail
# shellcheck source=scripts/version.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/version.sh"

version="$(workspace_version)"
if [[ -z "$version" ]]; then
    printf 'error: no [workspace.package] version in Cargo.toml\n' >&2
    exit 1
fi
if ! is_semver "$version"; then
    printf 'error: workspace version %s is not semver\n' "$version" >&2
    exit 1
fi

status=0
printf 'workspace version: %s (the only authoritative stamp)\n' "$version"

# The lockfile must already describe this version, or the next clean
# build rewrites it and dirties a tree the release gate requires clean.
lock="$(lock_version)"
if [[ "$lock" == "$version" ]]; then
    printf '  ✓ %-40s %s\n' "Cargo.lock" "$lock"
else
    printf '  ✗ %-40s %s (expected %s; run: cargo check --workspace)\n' \
        "Cargo.lock" "${lock:-none}" "$version"
    status=1
fi

# Tauri must derive the version from the crate, not carry its own.
tauri="$ROOT_DIR/crates/vak-desktop/tauri.conf.json"
if grep -qE '^[[:space:]]*"version"[[:space:]]*:' "$tauri"; then
    printf '  ✗ %-40s carries a version stamp — remove the key so Tauri derives it\n' \
        "tauri.conf.json"
    status=1
else
    printf '  ✓ %-40s derives from the crate\n' "tauri.conf.json"
fi

# Private frontends must stay unstamped, so nothing can drift there.
for f in "${UNSTAMPED_JSON[@]}"; do
    found="$(json_version "$ROOT_DIR/$f")"
    if [[ "$found" == "0.0.0" ]]; then
        printf '  ✓ %-40s unstamped\n' "$f"
    else
        printf '  ✗ %-40s %s (private package; pin to 0.0.0)\n' "$f" "${found:-none}"
        status=1
    fi
done

if ((status != 0)); then
    printf '\nversion singularity is broken — see docs/design/32-release-engineering.md\n' >&2
fi
exit "$status"
