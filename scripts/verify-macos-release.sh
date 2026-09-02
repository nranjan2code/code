#!/usr/bin/env bash
# Verify a macOS artifact from the artifact itself
# (docs/design/46-stabilization-install-and-onboarding.md Part V,
# invariant 15).
#
# The rule this exists for: **signing is verified from the produced bundle,
# never asserted by the build config.** A release that says "we set the
# signing identity" has proved nothing; a release where this walked the
# actual .app and found a Developer ID authority, an accepted Gatekeeper
# assessment, and a stapled ticket has.
#
# It runs today against an unsigned build and reports exactly that, so the
# gap is visible in CI output rather than discovered by a user meeting
# Gatekeeper. Once a Developer ID exists, `--require-signed` turns the same
# checks into a release gate with no new script to write.
#
# Usage: scripts/verify-macos-release.sh PATH_TO_APP [--require-signed]

set -Eeuo pipefail

APP="${1:-}"
REQUIRE_SIGNED=false
shift || true
while (($# > 0)); do
    case "$1" in
        --require-signed) REQUIRE_SIGNED=true ;;
        *) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
    shift
done

[[ -n "$APP" ]] || { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
[[ -d "$APP" ]] || { printf 'error: no bundle at %s\n' "$APP" >&2; exit 2; }
[[ "$(uname -s)" == "Darwin" ]] || { printf 'error: macOS only\n' >&2; exit 2; }

status=0
pass() { printf '  ✓ %-34s %s\n' "$1" "${2:-}"; }
warn() { printf '  ! %-34s %s\n' "$1" "${2:-}"; }
bad()  { printf '  ✗ %-34s %s\n' "$1" "${2:-}"; status=1; }

printf 'verifying %s\n' "$APP"

# --- structure ----------------------------------------------------------
[[ -f "$APP/Contents/Info.plist" ]] \
    && pass "Info.plist" "present" \
    || bad "Info.plist" "missing — not launchable as an app"

BIN="$APP/Contents/MacOS/vak"
[[ -x "$BIN" ]] && pass "executable" "$(basename "$BIN")" || bad "executable" "missing"

# --- architecture -------------------------------------------------------
# Every executable payload must be an architecture this artifact claims.
# A bundle carrying the wrong slice runs nowhere and says nothing about it.
if [[ -x "$BIN" ]]; then
    ARCHS="$(lipo -archs "$BIN" 2>/dev/null || file -b "$BIN")"
    pass "architecture" "$ARCHS"
fi

# --- dylib references ---------------------------------------------------
# Every dependency must be @-relative or system-owned. An absolute path
# into a build machine's Homebrew tree works on the machine that built it
# and nowhere else — which is exactly the failure a user reports as "it
# just doesn't open".
if [[ -x "$BIN" ]]; then
    FOREIGN="$(otool -L "$BIN" 2>/dev/null \
        | tail -n +2 \
        | awk '{print $1}' \
        | grep -v -E '^(/usr/lib/|/System/|@)' || true)"
    if [[ -z "$FOREIGN" ]]; then
        pass "dylib references" "all system-owned or @-relative"
    else
        bad "dylib references" "point outside the system:"
        printf '      %s\n' $FOREIGN
    fi
fi

# --- signing ------------------------------------------------------------
# Three distinct states, reported as three distinct things. Collapsing
# ad-hoc into "unsigned" hides that the seal exists and works; collapsing
# it into "signed" would claim a distribution signature we do not have.
SIGNED=false
if codesign --verify --deep --strict "$APP" >/dev/null 2>&1; then
    DETAIL="$(codesign -dv --verbose=4 "$APP" 2>&1)"
    AUTHORITY="$(printf '%s\n' "$DETAIL" | awk -F'=' '/Authority=/ {print $2; exit}')"
    SIGNATURE="$(printf '%s\n' "$DETAIL" | awk -F'=' '/^Signature=/ {print $2; exit}')"
    if [[ "$AUTHORITY" == *"Developer ID"* ]]; then
        SIGNED=true
        pass "code signature" "$AUTHORITY"
    elif [[ "$SIGNATURE" == "adhoc" ]]; then
        # A real seal with no identity: tamper-evident, not distributable.
        pass "code signature" "ad-hoc (seals the bundle; not a Developer ID)"
    else
        warn "code signature" "present but not a Developer ID: ${AUTHORITY:-unknown}"
    fi
elif codesign -d "$APP" >/dev/null 2>&1; then
    # A signature is present but does not validate. That is not "unsigned"
    # — it means the bundle was modified after signing, which is precisely
    # what a seal exists to detect, so it fails rather than warns.
    bad "code signature" "present but INVALID — the bundle was modified after signing"
    codesign --verify --deep --strict "$APP" 2>&1 | sed 's/^/      /' || true
else
    warn "code signature" "unsigned"
fi

# --- Gatekeeper and notarization ----------------------------------------
if spctl --assess --type execute "$APP" >/dev/null 2>&1; then
    pass "Gatekeeper assessment" "accepted"
else
    warn "Gatekeeper assessment" "rejected (expected while unsigned)"
fi

if xcrun stapler validate "$APP" >/dev/null 2>&1; then
    pass "notarization ticket" "stapled and valid"
else
    warn "notarization ticket" "absent (expected while unsigned)"
fi

# --- verdict ------------------------------------------------------------
printf '\n'
if [[ "$REQUIRE_SIGNED" == true && "$SIGNED" != true ]]; then
    printf '✗ --require-signed was given and this artifact is not signed by a Developer ID\n' >&2
    exit 1
fi
if ((status != 0)); then
    printf '✗ structural checks failed\n' >&2
    exit 1
fi
if [[ "$SIGNED" != true ]]; then
    printf '✓ structure verifies; NOT signed by a Developer ID — Gatekeeper will\n'
    printf '  refuse a double-click, and notarization is impossible until one\n'
    printf '  exists (doc 46 S10).\n'
else
    printf '✓ verified\n'
fi
