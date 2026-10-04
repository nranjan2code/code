#!/usr/bin/env bash
# A persistent Linux install, reachable from this machine's browser.
#
# `scripts/linux-check.sh` proves the Linux path works and then throws the
# container away. This one keeps it: a full stack installed, configured,
# and serving, so the Linux surfaces can be used rather than only asserted
# about.
#
# What the container gets:
#   · the host's Ollama, at host.docker.internal
#   · outbound internet, for Telegram and the curated integrations
#   · a published gateway port, so the admin console opens on the Mac
#
# What it does NOT get: systemd. A plain container has no init, so durable
# service *units* cannot be registered or exercised here — the gateway runs
# as the container's main process instead. That is stated rather than
# papered over, because "the stack is up" and "the service manager works"
# are different claims and only one of them is being made.
#
# Usage:
#   scripts/linux-stack.sh up [--port N] [--model M]
#   scripts/linux-stack.sh status
#   scripts/linux-stack.sh logs
#   scripts/linux-stack.sh shell
#   scripts/linux-stack.sh down
#
# Credentials come from the environment and are never printed:
#   VAK_CHECK_BOT_TOKEN   creates and credentials a Telegram bot
#   VAK_CHECK_TAVILY_KEY  enables the Tavily integration

set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

NAME="vak-linux"
IMAGE="rust:1-bookworm"
PORT=8902
MODEL="gemma4:e2b-mlx"
CMD="${1:-up}"
shift || true
while (($# > 0)); do
    case "$1" in
        --port) PORT="${2:-8902}"; shift ;;
        --model) MODEL="${2:-}"; shift ;;
        --image) IMAGE="${2:-}"; shift ;;
        *) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
    shift
done

command -v docker >/dev/null || { printf 'error: docker is required\n' >&2; exit 2; }
docker info >/dev/null 2>&1 || { printf 'error: the docker daemon is not running\n' >&2; exit 2; }

case "$CMD" in
status)
    docker ps --filter "name=$NAME" --format '  {{.Names}}  {{.Status}}  {{.Ports}}' \
        || true
    # Which commit is this container actually running?
    #
    # `up` binds /src to a `git archive HEAD` SNAPSHOT, deliberately, so an
    # in-flight editing session cannot race the container's build. The cost
    # is that the snapshot is frozen at the commit `up` ran on — and
    # `docker restart` faithfully rebuilds *that*, so a restart looks like a
    # refresh and is not one. A container quietly serving day-old code while
    # reporting healthy is exactly the failure this repository keeps
    # finding; here is where it becomes visible.
    RUNNING_SHA="$(docker inspect "$NAME" \
        --format '{{range .Config.Env}}{{if eq (index (split . "=") 0) "VAK_STACK_SRC_SHA"}}{{index (split . "=") 1}}{{end}}{{end}}' \
        2>/dev/null || true)"
    HEAD_SHA="$(git -C "$ROOT_DIR" rev-parse --short HEAD 2>/dev/null || echo unknown)"
    if [[ -z "$RUNNING_SHA" ]]; then
        printf '\n  source:  unknown — this container predates source stamping.\n'
        printf '           Re-create it to find out: scripts/linux-stack.sh up\n'
    elif [[ "$RUNNING_SHA" == "$HEAD_SHA" ]]; then
        printf '\n  source:  %s (current)\n' "$RUNNING_SHA"
    else
        printf '\n  source:  %s — STALE. HEAD is %s.\n' "$RUNNING_SHA" "$HEAD_SHA"
        printf '           `docker restart` will NOT pick this up: the container builds\n'
        printf '           from a snapshot taken when `up` ran. Re-create it:\n'
        printf '             scripts/linux-stack.sh up\n'
    fi
    # The gateway token lives in the container's credential store, never a
    # file; the CLI is how a signed-in link is read out of it.
    LINK="$(docker exec "$NAME" /opt/vak/bin/vak open admin --print --port 8901 2>/dev/null || true)"
    TOKEN="$(printf '%s' "$LINK" | sed -n 's/.*[?&]token=\([^&#]*\).*/\1/p')"
    [[ -n "$TOKEN" ]] && printf '\n  http://127.0.0.1:%s/admin?token=%s#/overview\n' "$PORT" "$TOKEN"
    exit 0
    ;;
logs)   exec docker logs -f "$NAME" ;;
shell)  exec docker exec -it "$NAME" bash ;;
down)
    docker rm -f "$NAME" >/dev/null 2>&1 && printf 'removed %s\n' "$NAME" || printf 'not running\n'
    exit 0
    ;;
up) ;;
*) printf 'unknown command: %s\n' "$CMD" >&2; exit 2 ;;
esac

docker rm -f "$NAME" >/dev/null 2>&1 || true

# Build from committed HEAD, not the working tree.
#
# The container installs a *release*, so the committed tree is the honest
# input — and mounting the live working tree meant a parallel editing
# session kept invalidating the admin-console dist/src gate mid-build, with
# the container and the editor racing each other. An export settles that:
# the container gets a stable snapshot and nobody's in-flight edits are
# touched or rebuilt.
SRC_DIR="$(mktemp -d "${TMPDIR:-/tmp}/vak-stack-src.XXXXXX")"
git -C "$ROOT_DIR" archive HEAD | tar -x -C "$SRC_DIR"
SRC_SHA="$(git -C "$ROOT_DIR" rev-parse --short HEAD)"
printf 'source:    committed HEAD (%s)\n' "$SRC_SHA"

printf '== bringing up a Linux stack ==\n'
printf 'container: %s\n' "$NAME"
printf 'port:      %s (published to this machine)\n' "$PORT"
printf 'model:     ollama/%s via host.docker.internal\n\n' "$MODEL"

# The container's own script. Installs, configures, then serves in the
# foreground so the container's lifetime IS the gateway's lifetime.
STACK='
set -eu
cd /src
# socat relays the published port to the loopback the server binds. The
# app is never asked to bind a public interface: the local plane stays
# loopback-only (docs/design/31-network-resilience.md), and the exposure
# is an explicit, container-scoped forward that dies with the container.
apt-get update -qq >/dev/null 2>&1 && apt-get install -y -qq socat >/dev/null 2>&1

echo "--- build ---"
cargo build --locked --release --package vak

VAK=/tmp/target/release/vak
PREFIX=/opt/vak
# No VAK_HOME: let paths resolve the way they do on a real Linux box
# (~/.local/share/vak for data, ~/vak-home for the Shared layer). Setting
# it would nest the Shared layer inside the data home, which is the
# self-contained-root behaviour, not a normal install.

echo "--- install ---"
"$VAK" self install --prefix "$PREFIX" --force
V="$PREFIX/bin/vak"

echo "--- setup ---"
mkdir -p /root/workspace && cd /root/workspace
VAK_SETUP_PROVIDER=ollama VAK_SETUP_MODEL="$STACK_MODEL" \
VAK_SETUP_POSTURE=workspace-write VAK_SETUP_SEED=1 \
    "$V" setup --non-interactive </dev/null 2>&1 | grep -E "route saved|posture|starter skills" || true

echo "--- activation (pins the bearer token; systemd is absent here) ---"
# --prefix matters: without it services-sync resolves the platform
# default, finds no manifest there, and bails before pinning anything.
"$V" self services-sync --prefix "$PREFIX" 2>&1 | sed "s/^/    /" || true
LINK=$("$V" open admin --print --port 8901)
TOKEN=${LINK#*token=}
TOKEN=${TOKEN%%[&#]*}
test -n "$TOKEN" || { echo "FAIL: no gateway token was pinned"; exit 1; }
echo "  token pinned"

if [ -n "${STACK_TAVILY_KEY:-}" ]; then
    echo "--- integration ---"
    "$V" serve --port 8901 >/tmp/tmp-serve.log 2>&1 &
    TMP=$!
    sleep 4
    curl -s -X PUT -H "Authorization: Bearer $TOKEN" -H "content-type: application/json" \
        --data-binary "{\"scope\":\"user\",\"key\":\"$STACK_TAVILY_KEY\"}" \
        http://127.0.0.1:8901/config/integrations/tavily >/dev/null 2>&1 || true
    if [ -n "${STACK_BOT_TOKEN:-}" ]; then
        curl -s -X POST -H "Authorization: Bearer $TOKEN" -H "content-type: application/json" \
            -d "{\"id\":\"linuxbot\",\"surface\":\"telegram\",\"label\":\"Linux Bot\"}" \
            http://127.0.0.1:8901/gateway/bots >/dev/null 2>&1 || true
        curl -s -X PUT -H "Authorization: Bearer $TOKEN" -H "content-type: application/json" \
            --data-binary "{\"token\":\"$STACK_BOT_TOKEN\"}" \
            http://127.0.0.1:8901/gateway/bots/linuxbot/token >/dev/null 2>&1 || true
        echo "  bot configured (not activated: no service manager in a container)"
    fi
    kill $TMP 2>/dev/null || true
    wait $TMP 2>/dev/null || true
    echo "  tavily enabled"
fi

echo ""
echo "=============================================================="
echo " Linux stack is up. From your Mac browser:"
echo ""
echo "   http://127.0.0.1:PUBLISHED/admin?token=$TOKEN#/overview"
echo ""
echo " This is a Linux install: Linux paths, Landlock sandbox."
echo "=============================================================="
echo ""

cd /root/workspace
# The relay listens on the published port and forwards to the loopback
# address the gateway binds. Started first so it is ready when the
# gateway comes up; the gateway runs in the foreground, so if it dies the
# container exits rather than sitting there looking healthy.
socat TCP-LISTEN:8900,fork,reuseaddr TCP:127.0.0.1:8901 &
exec "$V" serve --gateway --trust --port 8901
'

docker run -d --name "$NAME" \
    -v "$SRC_DIR:/src" \
    -w /src \
    -e CARGO_TARGET_DIR=/tmp/target \
    -e VAK_STACK_SRC_SHA="$SRC_SHA" \
    -e VAK_GIT_SHA="$SRC_SHA" \
    -e STACK_MODEL="$MODEL" \
    -e STACK_BOT_TOKEN="${VAK_CHECK_BOT_TOKEN:-}" \
    -e STACK_TAVILY_KEY="${VAK_CHECK_TAVILY_KEY:-}" \
    -e VAK_OLLAMA_BASE_URL="http://host.docker.internal:11434/v1" \
    -p "$PORT:8900" \
    --add-host=host.docker.internal:host-gateway \
    "$IMAGE" \
    bash -c "${STACK//PUBLISHED/$PORT}" >/dev/null

printf 'building and configuring; this takes a few minutes on first run\n'
printf 'follow along with:  scripts/linux-stack.sh logs\n\n'

# Wait for the stack to announce itself.
for _ in $(seq 1 180); do
    if docker logs "$NAME" 2>&1 | grep -q "Linux stack is up"; then
        docker logs "$NAME" 2>&1 | sed -n '/====/,/====/p'
        exit 0
    fi
    if ! docker ps --filter "name=$NAME" --format '{{.Names}}' | grep -q "$NAME"; then
        printf 'the container exited; last output:\n' >&2
        docker logs --tail 30 "$NAME" >&2
        exit 1
    fi
    sleep 5
done
printf 'still building — check `scripts/linux-stack.sh logs`\n'
