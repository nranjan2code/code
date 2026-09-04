#!/usr/bin/env sh
# The one-command install (docs/design/46-stabilization-install-and-onboarding.md
# D1, S7):
#
#   curl -fsSL https://get.vak.dev/install.sh | sh
#
# Detects the platform, downloads that platform's components, verifies each
# digest, and then hands off to `vak self install`. It never writes the
# install prefix itself: one writer owns the install root, so status,
# verify, update, and uninstall can all see the result (doc 32 invariant
# 11). A script that copied files into place behind the installer produced
# an install the installer could not manage.
#
# Deliberately POSIX sh, not bash: this runs on whatever a fresh machine
# has, which on many Linux images is dash.
#
# Environment:
#   VAK_FEED_URL   release feed to install from
#   VAK_PREFIX     install location (default: the platform's own)
#   VAK_VERSION    pin a version instead of taking the feed's latest

set -eu

FEED_URL="${VAK_FEED_URL:-https://get.vak.dev/release.json}"
STAGING=""

say() { printf '%s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

cleanup() {
    [ -n "$STAGING" ] && [ -d "$STAGING" ] && rm -rf "$STAGING"
}
trap cleanup EXIT HUP INT TERM

need() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 is required and was not found"
}

# --- platform -----------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Darwin) os="macos" ;;
    Linux) os="linux" ;;
    *) fail "unsupported platform '$os' — vak ships for macOS and Linux" ;;
esac
case "$arch" in
    arm64|aarch64) arch="aarch64" ;;
    x86_64|amd64) arch="x86_64" ;;
    *) fail "unsupported architecture '$arch'" ;;
esac
PLATFORM="$os/$arch"
say "vak installer — $PLATFORM"

need curl
need shasum || need sha256sum

sha256_of() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        sha256sum "$1" | awk '{print $1}'
    fi
}

# --- feed ---------------------------------------------------------------
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/vak-install.XXXXXX")"
FEED="$STAGING/release.json"
say "fetching $FEED_URL"
curl -fsSL "$FEED_URL" -o "$FEED" || fail "could not fetch the release feed at $FEED_URL"

# The feed is JSON; parse it with whatever the machine has rather than
# adding a dependency to the one command that has to work on a bare box.
if command -v python3 >/dev/null 2>&1; then
    parse() { python3 -c "
import json,sys
feed=json.load(open(sys.argv[1]))
platform=feed.get('platforms',{}).get(sys.argv[2])
if not platform:
    sys.exit('this release ships nothing for ' + sys.argv[2])
print(feed['version'])
for c in platform['components']:
    print('%s\t%s\t%s' % (c['name'], c['url'], c['sha256']))
" "$FEED" "$PLATFORM"; }
else
    fail "python3 is required to read the release feed"
fi

PARSED="$(parse)" || fail "$PARSED"
VERSION="$(printf '%s\n' "$PARSED" | head -1)"

# VAK_VERSION pins the install to a known-good release. This was documented
# in the header from the start and never implemented, so an operator asking
# for a specific version silently got whatever the feed was serving — the
# worst outcome for the one flag whose entire purpose is not moving.
if [ -n "${VAK_VERSION:-}" ] && [ "$VAK_VERSION" != "$VERSION" ]; then
    fail "the feed at $FEED_URL serves $VERSION, but VAK_VERSION asked for $VAK_VERSION.
       Point VAK_FEED_URL at that version's own release.json to install it."
fi
say "installing $VERSION"

# --- download and verify ------------------------------------------------
mkdir -p "$STAGING/bin"
printf '%s\n' "$PARSED" | tail -n +2 | while IFS="$(printf '\t')" read -r name url want; do
    [ -n "$name" ] || continue
    say "  fetching $name"
    curl -fsSL "$url" -o "$STAGING/bin/$name" || fail "download failed: $url"
    got="$(sha256_of "$STAGING/bin/$name")"
    if [ "$got" != "$want" ]; then
        fail "$name failed its integrity check (expected $want, got $got). Nothing was installed."
    fi
    chmod +x "$STAGING/bin/$name"
done

[ -x "$STAGING/bin/vak" ] || fail "the release did not include the vak binary"

# --- hand off to the installer -----------------------------------------
# The freshly downloaded binary installs itself and its siblings. This
# script never writes the prefix.
say "placing components"
if [ -n "${VAK_PREFIX:-}" ]; then
    "$STAGING/bin/vak" self install --prefix "$VAK_PREFIX" --force
else
    "$STAGING/bin/vak" self install --force
fi

say ""
say "installed. nothing is running yet — setup is what activates:"
say ""
say "    vak setup"
say ""
