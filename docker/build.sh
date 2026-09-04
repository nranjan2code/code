#!/usr/bin/env bash
# Build the vak container image with its build stamp filled in.
#
# `docker/Dockerfile` takes a `VAK_GIT_SHA` build arg so a running container
# can say which commit it is — `vak --version`, `/version`, and the install
# manifest all read it. Compose forwards `${VAK_GIT_SHA:-unknown}`, which
# means it is only ever set if the operator happened to export it first.
# Nobody does, so every image built the obvious way was stamped "unknown",
# and a container that cannot name its own build is one you cannot correlate
# with a ledger, an incident, or a receipt.
#
# This derives it. `docker compose build` is still fine — it just needs this
# script, or the variable, to get a real answer.
#
# Usage:
#   docker/build.sh                 # build vak:local
#   docker/build.sh --tag vak:2.0.1 # build under another tag
#   docker/build.sh --up            # build, then `docker compose up -d`

set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TAG="vak:local"
UP=false
while (($# > 0)); do
    case "$1" in
        --tag) TAG="${2:?--tag needs a value}"; shift ;;
        --up) UP=true ;;
        -h|--help) sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
    shift
done

command -v docker >/dev/null || { printf 'docker is required.\n' >&2; exit 1; }

# A dirty tree would produce an image whose stamp names a commit it does not
# actually contain. Say so rather than stamping a lie.
GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
if [[ "$GIT_SHA" != unknown && -n "$(git status --porcelain)" ]]; then
    GIT_SHA="${GIT_SHA}-dirty"
fi

printf '== building %s (%s) ==\n' "$TAG" "$GIT_SHA"
docker build \
    -f docker/Dockerfile \
    --build-arg "VAK_GIT_SHA=$GIT_SHA" \
    -t "$TAG" \
    .

printf '\n== stamp ==\n'
docker run --rm --entrypoint vak "$TAG" --version

if [[ "$UP" == true ]]; then
    printf '\n== up ==\n'
    VAK_GIT_SHA="$GIT_SHA" docker compose -f docker/compose.yaml up -d
    docker compose -f docker/compose.yaml ps
fi
