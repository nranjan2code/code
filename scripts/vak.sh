#!/usr/bin/env bash
# One entry point for the vak lifecycle: build, release, install, update,
# reinstall, uninstall, doctor. Dispatches to the real owner of each verb —
# scripts/build.sh, scripts/release.sh, or `vak self <verb>` — rather than
# reimplementing any of it, per docs/design/32-release-engineering.md's
# "one writer" rule.
#
# Usage: scripts/vak.sh <verb> [args...]
#
#   build        scripts/build.sh [--no-install] [--no-desktop] [--clean] ...
#   release      scripts/release.sh [--base-url URL] [--allow-dirty] ...
#   install      vak self install [--prefix DIR] [--force]
#   reinstall    vak self reinstall [--prefix DIR] [--yes]
#   verify       vak self verify [--prefix DIR]
#   status       vak self status [--prefix DIR]
#   update       vak self update [--prefix DIR] [--url URL] [--yes] [--dry-run]
#   uninstall    vak self uninstall [--prefix DIR] [--yes] [--purge]
#   services-sync  vak self services-sync [--prefix DIR] [NAME...]
#   doctor       vak doctor [--trust] [--repair]
#
# `install`/`reinstall`/`verify`/`status`/`update`/`uninstall`/
# `services-sync`/`doctor` run against an already-installed `vak` if one is
# on PATH, else fall back to the most recently built binary in target/
# (release preferred over debug). `build` and `release` always run in the
# repo, not against an installed binary.

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
    sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'
}

verb="${1:-}"
[[ -n "$verb" ]] || { usage; exit 2; }
shift || true

resolve_vak() {
    if command -v vak >/dev/null 2>&1; then
        command -v vak
    elif [[ -x "$ROOT_DIR/target/release/vak" ]]; then
        echo "$ROOT_DIR/target/release/vak"
    elif [[ -x "$ROOT_DIR/target/debug/vak" ]]; then
        echo "$ROOT_DIR/target/debug/vak"
    else
        printf 'error: no vak binary found on PATH or in target/ — run `scripts/vak.sh build` first\n' >&2
        exit 1
    fi
}

case "$verb" in
    build)
        exec "$ROOT_DIR/scripts/build.sh" "$@"
        ;;
    release)
        exec "$ROOT_DIR/scripts/release.sh" "$@"
        ;;
    install)
        exec "$(resolve_vak)" self install "$@"
        ;;
    reinstall)
        exec "$(resolve_vak)" self reinstall "$@"
        ;;
    verify)
        exec "$(resolve_vak)" self verify "$@"
        ;;
    status)
        exec "$(resolve_vak)" self status "$@"
        ;;
    update)
        exec "$(resolve_vak)" self update "$@"
        ;;
    uninstall)
        exec "$(resolve_vak)" self uninstall "$@"
        ;;
    services-sync)
        exec "$(resolve_vak)" self services-sync "$@"
        ;;
    doctor)
        exec "$(resolve_vak)" doctor "$@"
        ;;
    -h|--help|help)
        usage
        exit 0
        ;;
    *)
        printf 'unknown verb: %s\n\n' "$verb" >&2
        usage
        exit 2
        ;;
esac
