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

# ── the stamps a USER actually sees ─────────────────────────────────────
#
# Everything above proves the build system agrees with itself. It passed
# while the README badge said 0.8.0, the CHANGELOG head said 0.11.23, and
# the newest shipped build was 0.11.51 — a gate that certifies a mess is
# worse than no gate.

# shields.io writes a literal dash as `--`, so a prerelease badge reads
# `version-7.0.0--dev-<colour>`; unescape it before comparing.
readme_badge="$(sed -n 's|.*/badge/version-\([0-9][0-9.]*\(--[0-9A-Za-z.]*\)*\)-[0-9A-Fa-f]\{6\}.*|\1|p' "$ROOT_DIR/README.md" | head -1 | sed 's/--/-/g')"
if [[ "$readme_badge" == "$version" ]]; then
    printf '  ✓ %-40s %s\n' "README badge" "$readme_badge"
else
    printf '  ✗ %-40s %s (expected %s)\n' \
        "README badge" "${readme_badge:-none}" "$version"
    status=1
fi

changelog_head="$(sed -n 's/^## \([0-9][0-9.]*\) .*/\1/p' "$ROOT_DIR/CHANGELOG.md" | head -1)"
if [[ "$changelog_head" == "$version" ]] \
    || grep -qE '^## (Unreleased|\[Unreleased\])' "$ROOT_DIR/CHANGELOG.md"; then
    printf '  ✓ %-40s %s\n' "CHANGELOG" "${changelog_head:-unreleased}"
else
    printf '  ✗ %-40s newest entry is %s (expected %s or an Unreleased section)\n' \
        "CHANGELOG" "${changelog_head:-none}" "$version"
    status=1
fi

# ── monotonicity ────────────────────────────────────────────────────────
#
# The workspace version reached 0.11.51 and was then reset to 0.2.0. Update
# checks compare by semver precedence, so every install on the old line went
# permanently un-updatable and nothing said a word. A version that moves
# BACKWARDS is a release decision, never an accident to discover later.

highest_shipped="$(
    {
        git -C "$ROOT_DIR" tag 2>/dev/null | sed 's/^v//'
        # `|| true`: with `set -o pipefail`, a missing dist/ makes `ls` fail,
        # which fails the whole pipeline, which fails the assignment, which
        # exits the script — before the monotonicity check it feeds ever
        # runs. Every checkout without a local dist/ (i.e. every fresh
        # clone) could not pass this gate.
        { ls "$ROOT_DIR/dist" 2>/dev/null || true; } | sed 's/^v//'
    } | grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -1
)"
reset_marker="$ROOT_DIR/scripts/.version-reset"
if [[ -n "$highest_shipped" ]] \
    && [[ "$(printf '%s\n%s\n' "$version" "$highest_shipped" | sort -V | head -1)" == "$version" ]] \
    && [[ "$version" != "$highest_shipped" ]]; then
    # Matched on the line being left BEHIND, not the exact pair: the
    # declaration is "we abandoned 0.11.x", and it must not need re-editing
    # on every patch bump or it rots into a rubber stamp.
    # Match on the release LINE (major.minor) being left behind, not the
    # exact highest version. The declaration is "we abandoned 0.11.x"; the
    # highest tag on that line drifts (0.11.51 shipped untagged, leaving
    # 0.11.50 as the highest tag) and an exact match would then demand a
    # re-edit of the marker for a release nobody is making — which is
    # precisely the rubber stamp this comment warns against.
    shipped_line="${highest_shipped%.*}"
    if [[ -f "$reset_marker" ]] \
        && grep -qE "^${shipped_line//./\\.}\.[0-9]+ -> " "$reset_marker"; then
        printf '  ✓ %-40s %s (reset away from %s, declared)\n' \
            "release line" "$version" "$highest_shipped"
    else
        printf '  ✗ %-40s %s is BELOW the shipped %s\n' \
            "release line" "$version" "$highest_shipped"
        printf '      installs on %s can never update to %s (semver precedence).\n' \
            "$highest_shipped" "$version"
        printf '      If this is deliberate, record it:\n'
        printf '        echo "%s -> %s  # why" >> scripts/.version-reset\n' \
            "$highest_shipped" "$version"
        status=1
    fi
else
    printf '  ✓ %-40s %s\n' "release line" "$version"
fi

if ((status != 0)); then
    printf '\nversion singularity is broken — see docs/design/32-release-engineering.md\n' >&2
fi
exit "$status"
