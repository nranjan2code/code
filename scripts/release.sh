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
PLATFORM_KEY=""
ALLOW_DIRTY=false
BUILD=true
SKIP_CHECKS=false
TEST_TIMEOUT="${VAK_TEST_TIMEOUT:-900}"

while (($# > 0)); do
    case "$1" in
        --base-url) BASE_URL="${2:-}"; shift ;;
        --platform-key) PLATFORM_KEY="${2:-}"; shift ;;
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

# The cheap refusals come first, before the clean room deletes anything and
# long before the test suite: a dirty tree, or a changelog whose newest
# section is not this version (an "Unreleased" heading is right day to day
# and wrong in a tagged release, which would ship with its entry unnamed).
if [[ "$ALLOW_DIRTY" != true ]]; then
    if [[ -n "$(git status --porcelain)" ]]; then
        printf 'error: working tree is dirty — commit or pass --allow-dirty\n' >&2
        git status --short >&2
        exit 1
    fi
    printf '  ✓ %-44s clean\n' "working tree"
fi
CHANGELOG_HEAD="$(sed -n 's/^## \([^ ]*\).*/\1/p' "$ROOT_DIR/CHANGELOG.md" | head -1)"
if [[ "$CHANGELOG_HEAD" != "$VERSION" ]]; then
    printf 'error: CHANGELOG.md starts with "## %s"; a release of %s needs its own "## %s — <date>" section first\n' \
        "$CHANGELOG_HEAD" "$VERSION" "$VERSION" >&2
    exit 1
fi
printf '  ✓ %-44s %s\n' "CHANGELOG section" "$VERSION"

# Release assembly is a clean-room operation. Never let a previous release,
# frontend bundle, or developer Cargo tree participate in this run. These are
# the only generated trees this script owns; keep the list explicit so a
# cleanup can never expand to user source or configuration.
RELEASE_CLEAN_PATHS=(
    "$ROOT_DIR/target"
    "$ROOT_DIR/dist"
    "$ROOT_DIR/crates/vak-admin-ui/dist"
    "$ROOT_DIR/crates/vak-client-ui/dist"
    "$ROOT_DIR/crates/vak-client-ui/dist-web"
    "$ROOT_DIR/crates/vak-server/site/dist"
)
for generated_path in "${RELEASE_CLEAN_PATHS[@]}"; do
    case "$generated_path" in
        "$ROOT_DIR/target"|"$ROOT_DIR/dist"|"$ROOT_DIR/crates/vak-admin-ui/dist"|\
        "$ROOT_DIR/crates/vak-client-ui/dist"|"$ROOT_DIR/crates/vak-client-ui/dist-web"|\
        "$ROOT_DIR/crates/vak-server/site/dist") ;;
        *) printf 'error: refusing to clean unexpected release path %s\n' "$generated_path" >&2; exit 2 ;;
    esac
    rm -rf -- "$generated_path"
done
printf 'release clean-room: removed prior generated outputs\n'

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
# Deliberately empty until a build sets it.
#
# This used to default to the developer's own `target/release`, and was only
# reassigned INSIDE the `--no-build` == false branch — so `--no-build`
# collected whatever binaries happened to be lying around in the working
# tree, entirely unrelated to the gates that had just passed. That is the
# "release shipped a stale binary" failure, and it was the default behaviour
# of the flag. `--no-build` now reuses the staging directory and fails if
# there is nothing in it.
ARTIFACT_BIN_DIR=""

# The platform key for the machine running this script, in the same
# `<os>/<arch>` shape the feed and `scripts/install.sh` use. One definition,
# because the provenance gate and the feed must agree on what "this host" is.
host_platform_key() {
    local platform arch
    platform="$(uname -s | tr '[:upper:]' '[:lower:]')"
    case "$platform" in
        darwin) platform="macos" ;;
    esac
    arch="$(uname -m)"
    case "$arch" in
        arm64) arch="aarch64" ;;
        amd64) arch="x86_64" ;;
    esac
    printf '%s/%s' "$platform" "$arch"
}

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

# Bound the test gate. A deadlocked test (crates/vak-server/tests/gateway.rs
# once blocked every test on one mutex) makes an unbounded
# `cargo test --workspace` never return. A release must fail loudly on
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

# The frontends are built BEFORE the gates, not after.
#
# `cargo clippy --all-targets` and `cargo test --workspace` both compile
# vak-desktop, whose tauri codegen hard-fails when crates/vak-client-ui/dist
# is missing — and that directory is gitignored. Running the gates first
# meant a release could only pass on a machine that happened to have built
# the desktop UI earlier, which is precisely the leftover-state dependency
# this script exists to eliminate. On a fresh clone it failed every time.

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
#   vak-client-ui builds TWICE from one source (docs/design/48-web-client.md):
#
#     dist/     is *not* committed (gitignored) — `self install` copies
#               whatever is currently on disk into the app bundle's
#               Resources. Without an explicit rebuild here, a release
#               could carry whatever dist/ was left over from an
#               unrelated local build, not the source this release tags.
#
#     dist-web/ *is* committed, for exactly the admin bundle's reason:
#               vak-server embeds it with include_dir! so a headless box
#               builds the server without node. Same trap, same gate — a
#               committed bundle that no longer matches its source is
#               invisible to a local build and ships broken.
command -v npm >/dev/null || {
    printf 'error: npm is required to build the admin and desktop frontends for release\n' >&2
    exit 1
}
# The clean-room step deliberately removed all manifests. They are checked
# after rebuilding below; checking before the build would make a fresh clone
# impossible to release while allowing an old bundle to participate.
printf '\n== bundle manifests ==\n'
printf '  · manifests will be validated after clean rebuilds\n'

printf '\n== frontends ==\n'
( cd "$ROOT_DIR/crates/vak-admin-ui" && npm ci --silent && npm rebuild --silent && npm run build --silent >/dev/null )
if [[ -n "$(git status --porcelain -- crates/vak-admin-ui/dist)" ]]; then
    printf 'error: crates/vak-admin-ui/dist does not match crates/vak-admin-ui/src.\n' >&2
    printf 'Rebuild locally (npm run build in crates/vak-admin-ui), review the diff,\n' >&2
    printf 'and commit dist/ before releasing -- otherwise the compiled server embeds\n' >&2
    printf 'a frontend that does not match what this release claims to ship.\n' >&2
    git status --short -- crates/vak-admin-ui/dist >&2
    exit 1
fi
printf '  ✓ %-44s matches source\n' "vak-admin-ui/dist"
( cd "$ROOT_DIR/crates/vak-client-ui" && npm ci --silent && npm rebuild --silent && npm run build --silent >/dev/null )
if [[ -n "$(git status --porcelain -- crates/vak-client-ui/dist-web)" ]]; then
    printf 'error: crates/vak-client-ui/dist-web does not match crates/vak-client-ui/src.\n' >&2
    printf 'Rebuild locally (npm run build in crates/vak-client-ui), review the diff,\n' >&2
    printf 'and commit dist-web/ before releasing -- otherwise the compiled server\n' >&2
    printf 'serves a web client that does not match what this release claims to ship.\n' >&2
    git status --short -- crates/vak-client-ui/dist-web >&2
    exit 1
fi
printf '  ✓ %-44s matches source\n' "vak-client-ui/dist-web"
printf '  ✓ %-44s rebuilt\n' "vak-client-ui/dist"

# The public site at `/` is the third embedded bundle, and the only one
# that needs no npm — which is why it is checked here rather than inside
# the frontend block above. Same trap as the other two: site/dist is
# committed and embedded with include_dir!, so a src/ edit that was never
# rebuilt ships the previous pages, invisibly.
( cd "$ROOT_DIR" && python3 crates/vak-server/site/build.py >/dev/null )
if [[ -n "$(git status --porcelain -- crates/vak-server/site/dist)" ]]; then
    printf 'error: crates/vak-server/site/dist does not match crates/vak-server/site/src.\n' >&2
    printf 'Rebuild locally (python3 crates/vak-server/site/build.py), review the diff,\n' >&2
    printf 'and commit site/dist/ before releasing -- otherwise the compiled server\n' >&2
    printf 'serves a public site that does not match what this release claims to ship.\n' >&2
    git status --short -- crates/vak-server/site/dist >&2
    exit 1
fi
printf '  ✓ %-44s matches source\n' "vak-server/site/dist"

for manifest in \
    crates/vak-admin-ui/dist/.src-manifest \
    crates/vak-client-ui/dist-web/.src-manifest \
    crates/vak-server/site/dist/.src-manifest; do
    if [[ ! -s "$ROOT_DIR/$manifest" ]]; then
        printf 'error: clean rebuild did not produce %s\n' "$manifest" >&2
        exit 1
    fi
    printf '  ✓ %-44s present after rebuild\n' "$manifest"
done

if [[ "$SKIP_CHECKS" != true ]]; then
    cargo fmt --all -- --check
    printf '  ✓ %-44s clean\n' "cargo fmt"
    cargo clippy --locked --workspace --all-targets -- -D warnings
    printf '  ✓ %-44s clean\n' "cargo clippy"
    run_bounded "$TEST_TIMEOUT" "cargo test --workspace" \
        cargo test --locked --workspace --quiet
    printf '  ✓ %-44s passing\n' "cargo test"
fi


GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
printf '  ✓ %-44s %s\n' "commit" "$GIT_SHA"

# A tag that already exists means this version was released before;
# re-releasing it silently would leave two different binaries claiming
# the same version.
#
# A tag at this commit is a repeatable local build; a tag at a different
# commit means the version has already been released from different code.
if git rev-parse -q --verify "refs/tags/v$VERSION" >/dev/null 2>&1; then
    tagged="$(git rev-parse "v$VERSION^{commit}")"
    head_commit="$(git rev-parse "HEAD^{commit}")"
    if [[ "$tagged" != "$head_commit" ]]; then
        printf 'error: tag v%s already exists and points at %s, not this commit (%s).\n' \
            "$VERSION" "$(git rev-parse --short "$tagged")" "$(git rev-parse --short "$head_commit")" >&2
        printf '       That version already shipped from different code — bump the version.\n' >&2
        exit 1
    fi
    printf '  ✓ %-44s this commit\n' "tag v$VERSION"
else
    printf '  ✓ %-44s free\n' "tag v$VERSION"
fi

# ---- build ----------------------------------------------------------
OUT="$ROOT_DIR/dist/$VERSION"
if [[ "$BUILD" == true ]]; then
    printf '\n== build ==\n'
    # The binary stamps this into its own manifest, so an installed
    # build can be traced back to a commit.
    # `--locked` so a release can never resolve a dependency graph other
    # than the one Cargo.lock records and the gates above just tested.
    # Without it the released binaries and the Docker image (which has
    # always used --locked) could be built from different dependency
    # versions, with nothing anywhere to say so.
    VAK_GIT_SHA="$GIT_SHA" cargo build --locked --release \
        --package vak --package vak-desktop --package vak-delivery
    ARTIFACT_BIN_DIR="$RELEASE_TARGET_DIR/release"
else
    printf '\n== build skipped ==\n'
    # The staging directory is fresh per run (and removed on exit), so
    # `--no-build` only makes sense with an explicit override pointing at a
    # tree the caller vouches for. Anything else would collect binaries this
    # run never produced and never checked.
    ARTIFACT_BIN_DIR="${VAK_RELEASE_BIN_DIR:-}"
    if [[ -z "$ARTIFACT_BIN_DIR" ]]; then
        printf 'error: --no-build needs VAK_RELEASE_BIN_DIR pointing at the binaries to publish.\n' >&2
        printf '       Without it this would collect whatever is in the working tree, which is\n' >&2
        printf '       how a release ships a binary nobody built for it.\n' >&2
        exit 1
    fi
    printf '  · publishing pre-built binaries from %s\n' "$ARTIFACT_BIN_DIR"
    printf '  · the provenance gate below still applies\n'
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

# Feed ingestion is a Python subprocess runtime, not source-only tooling.
# Ship it beside the binaries so managed installs and the Linux image work
# even when the user's workspace is not a vak checkout.
cp -R "$ROOT_DIR/scripts/feeds" "$OUT/feeds"

# ---- provenance ------------------------------------------------------
# Prove the artifact IS the build this run gated.
#
# Everything above verifies the *tree*: versions agree, bundles match their
# sources, tests pass. Nothing verified the *binary*, so `release.json` could
# assert a version the binary had never been asked about, and did — a stale
# or mismatched binary produced a feed that confidently named the wrong
# build, which is worse than no feed at all because `self update` trusts it.
#
# The binary is asked directly. `vak --version` prints "<version> (<sha>)"
# when VAK_GIT_SHA was stamped (crates/vak/build.rs), so this catches a
# binary built from another commit as well as one built from another version.
printf '\n== provenance ==\n'
if [[ -n "$PLATFORM_KEY" && "$PLATFORM_KEY" != "$(host_platform_key)" ]]; then
    # A cross-built artifact cannot run here. Every leg of the release
    # matrix builds natively, so this is only reachable when someone
    # deliberately cross-publishes.
    printf '  · %s is not this host — cannot execute the artifact to verify it\n' "$PLATFORM_KEY" >&2
    printf '  · publishing unverified; build natively to get this gate\n' >&2
else
    reported="$("$OUT/vak" --version 2>/dev/null | head -1 || true)"
    if [[ -z "$reported" ]]; then
        printf 'error: the collected vak binary does not run on this host.\n' >&2
        printf '       A release must never publish a binary it could not execute once.\n' >&2
        exit 1
    fi
    if [[ "$reported" != *"$VERSION"* ]]; then
        printf 'error: the collected binary reports %q but this release claims %s.\n' \
            "$reported" "$VERSION" >&2
        printf '       The artifact is not the build this run gated.\n' >&2
        exit 1
    fi
    if [[ "$GIT_SHA" != unknown && "$reported" != *"$GIT_SHA"* ]]; then
        printf 'error: the collected binary reports %q, which does not carry this commit (%s).\n' \
            "$reported" "$GIT_SHA" >&2
        printf '       A stale binary in the staging tree is the usual cause.\n' >&2
        exit 1
    fi
    printf '  ✓ %-44s %s\n' "vak --version" "$reported"
fi

# ---- checksums + feed ----------------------------------------------
printf '\n== artifacts ==\n'
( cd "$OUT" && shasum -a 256 "${collected[@]%%:*}" > SHA256SUMS )

# A release is a matrix build: each leg declares what it produced rather
# than inferring it from the machine that happened to run the script, which
# is why a laptop release could only ever describe one platform.
KEY="${PLATFORM_KEY:-$(host_platform_key)}"

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
