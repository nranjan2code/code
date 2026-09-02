#!/usr/bin/env bash
# Build a release and emit the feed `vak self update` consumes.
#
# Produces, under dist/<version>/:
#   <component>                 the release binaries
#   SHA256SUMS                  checksums for manual verification
#   release.json                the update feed (schema 2)
#
# Usage: scripts/release.sh [--base-url URL] [--allow-dirty] [--no-build]
#                          [--skip-checks] [--test-timeout SECONDS]
#
# --base-url is where the artifacts will be served from; the feed records
# <base-url>/<version>/<component>. It defaults to a localhost URL so the
# update path can be exercised end to end by serving dist/ locally --
# `self update` speaks http/https only, so a file:// feed cannot be used.

set -Eeuo pipefail
# shellcheck source=scripts/version.sh
source "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/version.sh"

BASE_URL=""
ALLOW_DIRTY=false
BUILD=true
SKIP_CHECKS=false
TEST_TIMEOUT="${VAK_TEST_TIMEOUT:-900}"

while (($# > 0)); do
    case "$1" in
        --base-url) BASE_URL="${2:-}"; shift ;;
        --allow-dirty) ALLOW_DIRTY=true ;;
        --no-build) BUILD=false ;;
        --skip-checks) SKIP_CHECKS=true ;;
        --test-timeout) TEST_TIMEOUT="${2:-900}"; shift ;;
        -h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
    shift
done

cd "$ROOT_DIR"
VERSION="$(workspace_version)"

# A release changes the identity of every workspace crate. Keeping those
# one-shot test and release artifacts in the developer target directory made
# each version accumulate indefinitely. Build in an isolated directory shared
# by all gates in this run, then remove it on every normal exit.
RELEASE_TARGET_DIR="$(mktemp -d "${TMPDIR:-/tmp}/vak-release.XXXXXX")"
cleanup_release_target() {
    case "$RELEASE_TARGET_DIR" in
        */vak-release.*) rm -rf -- "$RELEASE_TARGET_DIR" ;;
        *) printf 'warning: refusing to remove unexpected release target %s\n' \
               "$RELEASE_TARGET_DIR" >&2 ;;
    esac
}
trap cleanup_release_target EXIT HUP INT TERM
export CARGO_TARGET_DIR="$RELEASE_TARGET_DIR"
ARTIFACT_BIN_DIR="$ROOT_DIR/target/release"

MIN_FREE_GB="${VAK_RELEASE_MIN_FREE_GB:-15}"
if [[ ! "$MIN_FREE_GB" =~ ^[0-9]+$ ]]; then
    printf 'error: VAK_RELEASE_MIN_FREE_GB must be a non-negative integer\n' >&2
    exit 2
fi
AVAILABLE_KB="$(df -Pk "$ROOT_DIR" | awk 'NR == 2 { print $4 }')"
REQUIRED_KB=$((MIN_FREE_GB * 1024 * 1024))
if ((AVAILABLE_KB < REQUIRED_KB)); then
    printf 'error: release needs at least %s GiB free for bounded staging; only %s GiB is available\n' \
        "$MIN_FREE_GB" "$((AVAILABLE_KB / 1024 / 1024))" >&2
    exit 1
fi
printf 'release staging: %s (removed on exit)\n' "$RELEASE_TARGET_DIR"

# ---- gates ----------------------------------------------------------
# docs/design/32-release-engineering.md: gate before build, so a release
# is never assembled from a tree that would not pass review.
printf '== gates ==\n'
"$ROOT_DIR/scripts/check-version.sh"

# Bound the test gate. crates/vak-server/tests/gateway.rs currently
# deadlocks (every test in it blocks on one mutex), so an unbounded
# `cargo test --workspace` never returns. A release must fail loudly on
# that rather than hang a terminal overnight.
run_bounded() {
    local seconds="$1" label="$2"; shift 2
    "$@" &
    local pid=$! waited=0
    while kill -0 "$pid" 2>/dev/null; do
        if ((waited >= seconds)); then
            kill -9 "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
            printf '\nerror: %s exceeded %ss and was killed.\n' "$label" "$seconds" >&2
            printf 'If it hung rather than ran slow, that is a defect to fix, not to wait out.\n' >&2
            return 1
        fi
        sleep 2
        waited=$((waited + 2))
    done
    wait "$pid"
}

if [[ "$SKIP_CHECKS" != true ]]; then
    cargo fmt --all -- --check
    printf '  ✓ %-44s clean\n' "cargo fmt"
    cargo clippy --workspace --all-targets -- -D warnings
    printf '  ✓ %-44s clean\n' "cargo clippy"
    run_bounded "$TEST_TIMEOUT" "cargo test --workspace" \
        cargo test --workspace --quiet
    printf '  ✓ %-44s passing\n' "cargo test"
fi

# Both frontends must be rebuilt from their current source before this
# gate can mean anything, for two different reasons:
#
#   vak-admin-ui/dist is *committed* — vak-server embeds it at Cargo
#   compile time (crates/vak-server/src/admin_ui.rs, include_dir!), so
#   only what is on disk in dist/ at build time ends up in the binary.
#   That is exactly how v0.8.1 shipped a blank admin console: the source
#   had a real fix, dist/ was never regenerated from it, and nothing
#   caught the mismatch before the release went out. Rebuilding here and
#   failing on a diff makes that class of miss impossible to ship again.
#   Since then vak-server's build.rs also refuses to compile against a
#   dist/ whose .src-manifest no longer matches src/, so an ordinary
#   `cargo build` fails first; this gate stays as the backstop that also
#   catches a *committed* dist/ diff, which a local build cannot see.
#
#   vak-desktop/ui/dist is *not* committed (gitignored) — self install
#   copies whatever is currently on disk into the bundle's Resources.
#   Without an explicit rebuild here, a release could carry whatever
#   dist/ happened to be left over from an unrelated local build, not
#   the source this release actually gates and tags.
command -v npm >/dev/null || {
    printf 'error: npm is required to build the admin and desktop frontends for release\n' >&2
    exit 1
}
printf '\n== frontends ==\n'
( cd "$ROOT_DIR/crates/vak-admin-ui" && npm ci --silent && npm run build --silent >/dev/null )
if [[ -n "$(git status --porcelain -- crates/vak-admin-ui/dist)" ]]; then
    printf 'error: crates/vak-admin-ui/dist does not match crates/vak-admin-ui/src.\n' >&2
    printf 'Rebuild locally (npm run build in crates/vak-admin-ui), review the diff,\n' >&2
    printf 'and commit dist/ before releasing -- otherwise the compiled server embeds\n' >&2
    printf 'a frontend that does not match what this release claims to ship.\n' >&2
    git status --short -- crates/vak-admin-ui/dist >&2
    exit 1
fi
printf '  ✓ %-44s matches source\n' "vak-admin-ui/dist"
( cd "$ROOT_DIR/crates/vak-desktop/ui" && npm ci --silent && npm run build --silent >/dev/null )
printf '  ✓ %-44s rebuilt\n' "vak-desktop/ui/dist"

if [[ "$ALLOW_DIRTY" != true ]]; then
    if [[ -n "$(git status --porcelain)" ]]; then
        printf 'error: working tree is dirty — commit or pass --allow-dirty\n' >&2
        git status --short >&2
        exit 1
    fi
    printf '  ✓ %-44s clean\n' "working tree"
fi

GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
printf '  ✓ %-44s %s\n' "commit" "$GIT_SHA"

# A tag that already exists means this version was released before;
# re-releasing it silently would leave two different binaries claiming
# the same version.
if git rev-parse "v$VERSION" >/dev/null 2>&1; then
    printf 'error: tag v%s already exists — bump the version first\n' "$VERSION" >&2
    exit 1
fi
printf '  ✓ %-44s free\n' "tag v$VERSION"

# ---- build ----------------------------------------------------------
OUT="$ROOT_DIR/dist/$VERSION"
if [[ "$BUILD" == true ]]; then
    printf '\n== build ==\n'
    # The binary stamps this into its own manifest, so an installed
    # build can be traced back to a commit.
    VAK_GIT_SHA="$GIT_SHA" cargo build --release \
        --package vak --package vak-desktop --package vak-delivery
    ARTIFACT_BIN_DIR="$RELEASE_TARGET_DIR/release"
else
    printf '\n== build skipped ==\n'
fi

rm -rf "$OUT"
mkdir -p "$OUT"

# Components, mirroring COMPONENTS in crates/vak/src/install/mod.rs.
# Only vak is required; the rest ship when the build produced them.
REQUIRED=("vak")
OPTIONAL=("vak-desktop" "vak-delivery-worker")

collected=()
for name in "${REQUIRED[@]}"; do
    src="$ARTIFACT_BIN_DIR/$name"
    if [[ ! -x "$src" ]]; then
        printf 'error: required component %s missing at %s\n' "$name" "$src" >&2
        exit 1
    fi
    cp "$src" "$OUT/$name"
    collected+=("$name:true")
done
for name in "${OPTIONAL[@]}"; do
    src="$ARTIFACT_BIN_DIR/$name"
    if [[ -x "$src" ]]; then
        cp "$src" "$OUT/$name"
        collected+=("$name:false")
    else
        printf '  · %s not built — omitted from this release\n' "$name"
    fi
done

# ---- checksums + feed ----------------------------------------------
printf '\n== artifacts ==\n'
( cd "$OUT" && shasum -a 256 "${collected[@]%%:*}" > SHA256SUMS )

PLATFORM="$(uname -s | tr '[:upper:]' '[:lower:]')"
case "$PLATFORM" in
    darwin) PLATFORM="macos" ;;
esac
ARCH="$(uname -m)"
case "$ARCH" in
    arm64) ARCH="aarch64" ;;
esac
KEY="$PLATFORM/$ARCH"

if [[ -z "$BASE_URL" ]]; then
    BASE_URL="http://127.0.0.1:8899"
    printf '  · no --base-url; feed points at %s for local testing\n' "$BASE_URL"
fi

{
    printf '{\n'
    printf '  "schema": 2,\n'
    printf '  "version": "%s",\n' "$VERSION"
    printf '  "released_at": "%s",\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf '  "platforms": {\n'
    printf '    "%s": {\n' "$KEY"
    printf '      "components": [\n'
    first=true
    for entry in "${collected[@]}"; do
        name="${entry%%:*}"
        required="${entry##*:}"
        sum="$(shasum -a 256 "$OUT/$name" | awk '{print $1}')"
        [[ "$first" == true ]] || printf ',\n'
        first=false
        printf '        {"name": "%s", "url": "%s/%s/%s", "sha256": "%s", "required": %s}' \
            "$name" "${BASE_URL%/}" "$VERSION" "$name" "$sum" "$required"
    done
    printf '\n      ]\n'
    printf '    }\n'
    printf '  }\n'
    printf '}\n'
} > "$OUT/release.json"

for entry in "${collected[@]}"; do
    name="${entry%%:*}"
    printf '  ✓ %-24s %s\n' "$name" "$(shasum -a 256 "$OUT/$name" | awk '{print $1}' | cut -c1-16)…"
done

printf '\n== done ==\n'
printf 'version   %s (%s)\n' "$VERSION" "$GIT_SHA"
printf 'platform  %s\n' "$KEY"
printf 'output    %s\n' "$OUT"
printf '\nexercise the update path against these artifacts:\n'
printf '  (cd %s && python3 -m http.server 8899) &\n' "$ROOT_DIR/dist"
printf '  vak self update --url %s/%s/release.json --dry-run\n' "${BASE_URL%/}" "$VERSION"
printf '\npublish, then tag:\n'
printf '  git tag -a v%s -m "release %s" && git push origin v%s\n' "$VERSION" "$VERSION" "$VERSION"
