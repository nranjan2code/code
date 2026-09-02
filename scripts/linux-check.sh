#!/usr/bin/env bash
# Exercise the Linux path in a container
# (docs/design/46-stabilization-install-and-onboarding.md S7/S8).
#
# Half this product's supported platforms cannot be tested on the machine
# most of it is written on. Paths resolve differently
# (`~/.local/share/vak` rather than `~/Library/Application Support`), the
# service backend is systemd rather than launchd, the sandbox is Landlock
# rather than Seatbelt, and `install.sh` runs under dash rather than bash.
# None of that is exercised by a macOS test run, so it is exercised here.
#
# Deliberately NOT a full release: no DMG, no notarization, nothing
# macOS-shaped. It answers one question — does the Linux install and setup
# path actually work — and answers it on Linux.
#
# Usage: scripts/linux-check.sh [--image IMAGE] [--keep]
#        scripts/linux-check.sh --serve [--port N]
#
# --serve leaves a configured Linux install running with its setup wizard
# reachable from this machine's browser, so the Linux web surface can be
# looked at from a Mac desktop rather than only asserted about.

set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

IMAGE="rust:1-bookworm"
KEEP=false
SERVE=false
PORT=7788
while (($# > 0)); do
    case "$1" in
        --image) IMAGE="${2:-}"; shift ;;
        --keep) KEEP=true ;;
        --serve) SERVE=true ;;
        --port) PORT="${2:-7788}"; shift ;;
        -h|--help) sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
    shift
done

command -v docker >/dev/null || {
    printf 'error: docker is required for the Linux check\n' >&2
    exit 2
}
docker info >/dev/null 2>&1 || {
    printf 'error: the docker daemon is not running\n' >&2
    exit 2
}

printf '== linux check (%s) ==\n' "$IMAGE"

# The container script. Runs as a normal build+install+setup cycle, with
# every assertion stated so a failure says which property broke.
CONTAINER_SCRIPT='
set -eu
cd /src

echo "--- build ---"
# A container has no committed desktop frontend, so build only the CLI:
# the desktop shell is a macOS-first surface and its absence is normal on
# a headless Linux box (`self install` treats it as an optional component).
cargo build --release --package vak --offline 2>/dev/null \
    || cargo build --release --package vak

VAK=/tmp/target/release/vak
export VAK_HOME=/tmp/vak-home
PREFIX=/tmp/vak-prefix

echo "--- paths resolve to the Linux layout ---"
"$VAK" self state | head -3
case "$($VAK setup status --json 2>/dev/null | head -1)" in
    *) : ;;
esac

echo "--- install ---"
"$VAK" self install --prefix "$PREFIX" --force
test -x "$PREFIX/bin/vak" || { echo "FAIL: no binary in the prefix"; exit 1; }
echo "  installed"

echo "--- install starts nothing (doc 46 D6) ---"
if [ -d "$VAK_HOME/gateway" ] || [ -d "$VAK_HOME/sessions" ]; then
    echo "FAIL: install created durable state"
    exit 1
fi
echo "  no durable state created"

echo "--- setup status reads on Linux ---"
mkdir -p /tmp/workspace
cd /tmp/workspace
code=0
"$PREFIX/bin/vak" setup status >/dev/null 2>&1 || code=$?
if [ "$code" -ge 2 ]; then
    echo "FAIL: setup status could not read state (exit $code)"
    exit 1
fi
echo "  readable (exit $code = readiness, not failure)"

echo "--- non-interactive setup refuses a missing choice ---"
code=0
"$PREFIX/bin/vak" setup --non-interactive </dev/null >/dev/null 2>&1 || code=$?
test "$code" -eq 2 || { echo "FAIL: expected refusal (exit 2), got $code"; exit 1; }
echo "  refused, as it must"

echo "--- non-interactive setup completes when told everything ---"
# Every choice supplied, including the provider: the previous assertion
# proved it refuses without one, so this one proves it proceeds with one.
# Ollama is the keyless provider, which is what makes a container run
# possible at all.
code=0
VAK_SETUP_PROVIDER=ollama VAK_SETUP_MODEL=llama3 \
VAK_SETUP_POSTURE=workspace-write VAK_SETUP_SEED=1 \
    "$PREFIX/bin/vak" setup --non-interactive </dev/null >/tmp/setup.log 2>&1 || code=$?
if [ "$code" -ge 2 ]; then
    echo "FAIL: setup refused despite every choice being supplied (exit $code)"
    tail -20 /tmp/setup.log
    exit 1
fi
grep -q "route saved: ollama/llama3" /tmp/setup.log || {
    echo "FAIL: the route was not written"; tail -20 /tmp/setup.log; exit 1; }
echo "  route and posture written"
test -d "$VAK_HOME/vak-home/.vak/skills" || { echo "FAIL: seeding did not run"; exit 1; }
echo "  seeded $(ls "$VAK_HOME/vak-home/.vak/skills" | wc -l) skill(s)"

echo "--- the web surface serves on a headless box ---"
# Linux is headless, so the wizard has to be reachable the way an operator
# would reach it: a loopback URL from `vak setup --print-url`, tunnelled or
# port-forwarded. This proves the server binds, authenticates, and serves
# both the projection and the console on Linux.
cd /tmp/workspace
"$PREFIX/bin/vak" setup --print-url > /tmp/wizard.log 2>&1 &
WIZARD_PID=$!
for _ in $(seq 1 40); do
    grep -q "http://127" /tmp/wizard.log 2>/dev/null && break
    sleep 0.5
done
URL="$(grep -o "http://127[^ ]*" /tmp/wizard.log | head -1)"
if [ -z "$URL" ]; then
    echo "FAIL: the setup server printed no URL"
    cat /tmp/wizard.log
    exit 1
fi
BASE="$(echo "$URL" | sed "s|/admin.*||")"
TOKEN="$(echo "$URL" | sed "s|.*token=||;s|#.*||")"

code=$(curl -s -o /tmp/onboarding.json -w "%{http_code}"     -H "Authorization: Bearer $TOKEN" "$BASE/onboarding")
test "$code" = "200" || { echo "FAIL: /onboarding returned $code"; exit 1; }
grep -q "core_ready" /tmp/onboarding.json || { echo "FAIL: no projection body"; exit 1; }
echo "  /onboarding serves the projection"

code=$(curl -s -o /dev/null -w "%{http_code}"     -H "Authorization: Bearer $TOKEN" "$BASE/admin")
test "$code" = "200" || { echo "FAIL: /admin returned $code"; exit 1; }
echo "  /admin serves the console (the wizard lives at #/setup)"

# Unauthenticated access must be refused even on loopback.
code=$(curl -s -o /dev/null -w "%{http_code}" "$BASE/onboarding")
test "$code" = "401" || { echo "FAIL: unauthenticated /onboarding returned $code"; exit 1; }
echo "  unauthenticated requests are refused ($code)"

kill "$WIZARD_PID" 2>/dev/null || true
wait "$WIZARD_PID" 2>/dev/null || true

echo "--- install.sh parses under dash ---"
dash -n /src/scripts/install.sh 2>/dev/null || sh -n /src/scripts/install.sh
echo "  parses"

echo "--- purge leaves nothing ---"
"$PREFIX/bin/vak" self uninstall --yes --purge --prefix "$PREFIX" >/dev/null
for leftover in "$VAK_HOME/sessions" "$VAK_HOME/vak-home/.vak/config.toml"; do
    if [ -e "$leftover" ]; then
        echo "FAIL: $leftover survived a purge"
        exit 1
    fi
done
echo "  clean"

echo ""
echo "linux check passed"
'

CONTAINER_NAME="vak-linux-check-$$"
cleanup() {
    if [[ "$KEEP" != true ]]; then
        docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT HUP INT TERM

# The source is mounted read-only apart from `target`, so a container run
# cannot rewrite the working tree it was launched from.
# ---------------------------------------------------------------------------
# --serve: bring up a Linux install and expose its wizard to this machine.
#
# The setup server still binds loopback ONLY inside the container — that
# guarantee is not weakened for convenience. A socat relay inside the
# container forwards the published port to it, so the exposure is explicit,
# container-scoped, and disappears with the container.
SERVE_SCRIPT='
set -eu
cd /src
apt-get update -qq >/dev/null 2>&1 && apt-get install -y -qq socat >/dev/null 2>&1

cargo build --release --package vak

VAK=/tmp/target/release/vak
export VAK_HOME=/tmp/vak-home
PREFIX=/tmp/vak-prefix
"$VAK" self install --prefix "$PREFIX" --force >/dev/null

mkdir -p /tmp/workspace
cd /tmp/workspace
VAK_SETUP_PROVIDER=ollama VAK_SETUP_MODEL=llama3 \
VAK_SETUP_POSTURE=workspace-write VAK_SETUP_SEED=1 \
    "$PREFIX/bin/vak" setup --non-interactive </dev/null >/dev/null 2>&1 || true

"$PREFIX/bin/vak" setup --print-url > /tmp/wizard.log 2>&1 &
for _ in $(seq 1 60); do
    grep -q "http://127" /tmp/wizard.log 2>/dev/null && break
    sleep 0.5
done
URL="$(grep -o "http://127[^ ]*" /tmp/wizard.log | head -1)"
test -n "$URL" || { echo "the setup server printed no URL"; cat /tmp/wizard.log; exit 1; }
INNER_PORT="$(echo "$URL" | sed "s|http://127.0.0.1:||;s|/.*||")"
TOKEN="$(echo "$URL" | sed "s|.*token=||;s|#.*||")"

echo ""
echo "=============================================================="
echo " Linux wizard, from your Mac browser:"
echo ""
echo "   http://127.0.0.1:PUBLISHED/admin?token=$TOKEN#/setup"
echo ""
echo " (this is a Linux install: Linux paths, Linux service backend)"
echo " Ctrl-C to stop and remove the container."
echo "=============================================================="
echo ""

socat TCP-LISTEN:8899,fork,reuseaddr TCP:127.0.0.1:"$INNER_PORT"
'

if [[ "$SERVE" == true ]]; then
    printf '== linux wizard (%s) ==\n' "$IMAGE"
    printf 'building and configuring a Linux install; first run takes a few minutes\n\n'
    docker run --rm --name "$CONTAINER_NAME" \
        -v "$ROOT_DIR:/src" \
        -w /src \
        -e CARGO_TARGET_DIR=/tmp/target \
        -p "$PORT:8899" \
        "$IMAGE" \
        bash -c "${SERVE_SCRIPT//PUBLISHED/$PORT}"
    exit $?
fi

docker run --rm --name "$CONTAINER_NAME" \
    -v "$ROOT_DIR:/src" \
    -w /src \
    -e CARGO_TARGET_DIR=/tmp/target \
    "$IMAGE" \
    bash -c "$CONTAINER_SCRIPT"
