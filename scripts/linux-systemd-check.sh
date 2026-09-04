#!/usr/bin/env bash
# Exercise the *service manager* on Linux, for real
# (docs/design/28-operations.md, docs/design/46 S7).
#
# `scripts/linux-check.sh` and `linux-stack.sh` prove the Linux install and
# setup paths work, but neither could touch systemd: a plain container has
# no init, so `vak self services-sync` had nothing to talk to. That left
# half of `crates/vak-ops` — every `#[cfg(not(target_os = "macos"))]`
# branch — running only in unit tests with a fake `CommandRunner`, and the
# real `systemctl --user` interaction unexercised on any machine.
#
# It turns out systemd *does* run in a container, and so does a user
# session, which is what vak actually uses. Two stages, because the
# toolchain image and the systemd image are different things:
#
#   1. build the Linux binary in the Rust image, into a host directory
#   2. boot systemd as PID 1 elsewhere, copy the binary in, and drive the
#      real lifecycle against a real service manager
#
# The systemd container runs `--privileged` with the host cgroup namespace,
# which systemd needs to manage cgroups. That is a genuine privilege grant:
# fine for a local, disposable test container, and not something to imitate
# for anything that ships.
#
# Usage: scripts/linux-systemd-check.sh [--keep]

set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

NAME="vak-systemd-check"
KEEP=false
while (($# > 0)); do
    case "$1" in
        --keep) KEEP=true ;;
        -h|--help) sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
    shift
done

command -v docker >/dev/null || { printf 'error: docker is required\n' >&2; exit 2; }
docker info >/dev/null 2>&1 || { printf 'error: the docker daemon is not running\n' >&2; exit 2; }

fail() { printf '\n✗ %s\n' "$*" >&2; exit 1; }
step() { printf '\n== %s ==\n' "$*"; }
ok()   { printf '  ✓ %s\n' "$*"; }

STAGE="$(mktemp -d "${TMPDIR:-/tmp}/vak-systemd.XXXXXX")"
cleanup() {
    if [[ "$KEEP" != true ]]; then
        docker rm -f "$NAME" >/dev/null 2>&1 || true
        case "$STAGE" in */vak-systemd.*) rm -rf -- "$STAGE" ;; esac
    else
        printf '\nkept: container %s, stage %s\n' "$NAME" "$STAGE"
    fi
}
trap cleanup EXIT HUP INT TERM

# --- stage 1: a Linux binary ---------------------------------------------
step "build a Linux binary"
SRC="$STAGE/src"
mkdir -p "$SRC" "$STAGE/out"
git -C "$ROOT_DIR" archive HEAD | tar -x -C "$SRC"
docker run --rm \
    -v "$SRC:/src" -v "$STAGE/out:/out" -w /src \
    -e CARGO_TARGET_DIR=/tmp/target \
    rust:1-bookworm \
    bash -c 'cargo build --locked --release --package vak >/dev/null && cp /tmp/target/release/vak /out/vak'
[[ -x "$STAGE/out/vak" ]] || fail "the Linux build produced no binary"
ok "built from $(git -C "$ROOT_DIR" rev-parse --short HEAD)"

# --- stage 2: a real service manager -------------------------------------
step "boot systemd as PID 1"
docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" \
    --privileged --cgroupns=host \
    -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
    --tmpfs /run --tmpfs /run/lock \
    -e container=docker \
    ubuntu:24.04 \
    bash -c 'apt-get update -qq >/dev/null 2>&1 \
        && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
             systemd systemd-sysv dbus dbus-user-session >/dev/null 2>&1 \
        && exec /lib/systemd/systemd' >/dev/null

for _ in $(seq 1 60); do
    state="$(docker exec "$NAME" systemctl is-system-running 2>/dev/null || true)"
    case "$state" in running|degraded) break ;; esac
    sleep 5
done
[[ "$state" == running || "$state" == degraded ]] || fail "systemd did not come up (state: ${state:-none})"
ok "systemd is $state"

step "start a user session"
# vak generates *user* units, so a user manager has to exist — which is the
# part that made this look impossible in a container.
docker exec "$NAME" bash -c '
    loginctl enable-linger root >/dev/null 2>&1
    systemctl start user@0.service >/dev/null 2>&1
    sleep 3
    XDG_RUNTIME_DIR=/run/user/0 systemctl --user is-system-running' >/dev/null 2>&1 \
    || fail "no user session: systemctl --user is unavailable"
ok "systemctl --user is running"

docker cp "$STAGE/out/vak" "$NAME:/usr/local/bin/vak" >/dev/null
docker exec "$NAME" chmod +x /usr/local/bin/vak

# --- the real lifecycle --------------------------------------------------
step "install, configure, and activate against real systemd"
docker exec "$NAME" bash -c '
set -eu
export XDG_RUNTIME_DIR=/run/user/0
PREFIX=/opt/vak

/usr/local/bin/vak self install --prefix "$PREFIX" --force >/dev/null
V="$PREFIX/bin/vak"

mkdir -p /root/workspace && cd /root/workspace
VAK_SETUP_PROVIDER=ollama VAK_SETUP_MODEL=llama3 \
VAK_SETUP_POSTURE=workspace-write VAK_SETUP_SEED=1 \
    "$V" setup --non-interactive </dev/null >/dev/null 2>&1 || true

echo "--- activation ---"
"$V" self services-sync --prefix "$PREFIX" 2>&1 | sed "s/^/    /"
' || fail "activation failed"

step "verify systemd actually has the units"
docker exec "$NAME" bash -c '
set -eu
export XDG_RUNTIME_DIR=/run/user/0
ls /root/.config/systemd/user/*.service >/dev/null 2>&1 \
    || { echo "FAIL: no unit files were written"; exit 1; }
echo "  unit files:"
for u in /root/.config/systemd/user/*.service; do
    n=$(basename "$u")
    printf "    %-28s %s\n" "$n" "$(systemctl --user is-active "$n" 2>/dev/null || echo inactive)"
done
systemctl --user list-unit-files "vak-*" --no-legend 2>/dev/null | sed "s/^/    registered: /" || true
' || fail "systemd does not have the units"

step "the gateway unit is genuinely managed"
docker exec "$NAME" bash -c '
set -eu
export XDG_RUNTIME_DIR=/run/user/0
systemctl --user is-enabled vak-gateway.service >/dev/null 2>&1 \
    || { echo "FAIL: vak-gateway.service is not enabled"; exit 1; }
echo "  enabled"
# Restart through the manager, not by hand: this is the interaction that
# only exists on Linux and had never run anywhere.
systemctl --user restart vak-gateway.service
sleep 3
systemctl --user show vak-gateway.service -p ExecMainStatus -p ActiveState \
    | sed "s/^/    /"
' || fail "the gateway unit is not manageable"

step "teardown removes what it registered"
docker exec "$NAME" bash -c '
set -eu
export XDG_RUNTIME_DIR=/run/user/0
/opt/vak/bin/vak self uninstall --yes --purge --prefix /opt/vak >/dev/null 2>&1 || true
if ls /root/.config/systemd/user/vak-*.service >/dev/null 2>&1; then
    echo "FAIL: unit files survived uninstall"
    exit 1
fi
echo "  no vak units remain"
' || fail "teardown left units behind"

printf '\n✓ the Linux service manager works, and is now actually exercised\n'
