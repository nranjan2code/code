#!/usr/bin/env bash
# The upgrade gate (docs/design/46-stabilization-install-and-onboarding.md
# VII.5): prove that updating preserves everything the durable state
# registry says it must.
#
# Everything else in doc 46 Part VII is a promise. This is the machine that
# keeps it: install a prior artifact, configure it for real, snapshot every
# declared file, update to this build, and assert per-entry survival — then
# repeat downgrading, because the leg people forget is whether the OLDER
# binary can still read what the newer one wrote.
#
# The comparison rules are NOT here. They live in `vak_core::state` beside
# the registry, and `vak self state --verify` applies them, so a shell
# script and a library cannot drift apart about what an update may do.
#
# Usage: scripts/upgrade-gate.sh [--previous PATH_TO_VAK_BINARY]
#
# --previous defaults to this build. That is not a tautology: with both
# legs the same binary it still proves that install and update themselves
# do not disturb state, which is the regression most likely to happen and
# the one nothing else catches. Point it at a real prior release once one
# is published and the same script becomes a true cross-version gate.

set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

PREVIOUS=""
while (($# > 0)); do
    case "$1" in
        --previous) PREVIOUS="${2:-}"; shift ;;
        -h|--help) sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
    shift
done

cd "$ROOT_DIR"

fail() { printf '\n✗ %s\n' "$*" >&2; exit 1; }
step() { printf '\n== %s ==\n' "$*"; }

step "build this revision"
cargo build --release --package vak >/dev/null
HEAD_BIN="$ROOT_DIR/target/release/vak"
[[ -x "$HEAD_BIN" ]] || fail "no vak binary at $HEAD_BIN"

if [[ -z "$PREVIOUS" ]]; then
    PREVIOUS="$HEAD_BIN"
    printf '  · no --previous given; using this build for both legs\n'
    printf '    (still proves install/update do not disturb state)\n'
fi
[[ -x "$PREVIOUS" ]] || fail "previous artifact is not executable: $PREVIOUS"

# An isolated home and prefix. The gate must never touch the operator's
# real state -- it deliberately drives destructive lifecycle commands.
GATE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/vak-upgrade-gate.XXXXXX")"
cleanup() {
    case "$GATE_DIR" in
        */vak-upgrade-gate.*) rm -rf -- "$GATE_DIR" ;;
    esac
}
trap cleanup EXIT HUP INT TERM

export VAK_HOME="$GATE_DIR/home"
PREFIX="$GATE_DIR/prefix"
WORKSPACE="$GATE_DIR/workspace"
mkdir -p "$VAK_HOME" "$WORKSPACE"
printf 'gate home:   %s\n' "$VAK_HOME"
printf 'gate prefix: %s\n' "$PREFIX"

step "install the prior artifact"
"$PREVIOUS" self install --prefix "$PREFIX" --force >/dev/null
INSTALLED="$PREFIX/bin/vak"
[[ -x "$INSTALLED" ]] || fail "prior install produced no binary at $INSTALLED"

step "configure it, for real"
# A fixture with something in every kind of durable state: a route and a
# posture (config), starter capabilities (the Shared layer), and a session
# ledger. An empty home would let an update that destroys data pass.
(
    cd "$WORKSPACE"
    VAK_SETUP_POSTURE=workspace-write VAK_SETUP_SEED=1 \
        "$INSTALLED" setup --non-interactive </dev/null >/dev/null 2>&1 || true
)
"$INSTALLED" setup seed >/dev/null 2>&1 || true

# A ledger row that must survive byte-for-byte.
mkdir -p "$VAK_HOME/sessions/gate"
printf '{"kind":"header","session_id":"gate-fixture"}\n' \
    > "$VAK_HOME/sessions/gate/fixture.jsonl"
printf '{"at":"gate","kind":"config_change"}\n' \
    > "$VAK_HOME/security-events.jsonl"

# An edited seed skill: an update must never overwrite a file the operator
# has changed (doc 46 VII.4).
EDITED_SKILL="$VAK_HOME/vak-home/.vak/skills/debugging/SKILL.md"
if [[ -f "$EDITED_SKILL" ]]; then
    printf '\nEDITED BY THE OPERATOR\n' >> "$EDITED_SKILL"
    EDITED_SUM="$(shasum -a 256 "$EDITED_SKILL" | awk '{print $1}')"
else
    EDITED_SUM=""
    printf '  · no seeded skill to edit; skipping that assertion\n'
fi

step "snapshot before"
BEFORE="$GATE_DIR/before.json"
"$INSTALLED" self state > "$BEFORE"
FILES_BEFORE="$(python3 -c "
import json,sys
d=json.load(open('$BEFORE'))
print(sum(len(e['files']) for e in d['entries']))
")"
printf '  %s declared file(s) captured\n' "$FILES_BEFORE"
[[ "$FILES_BEFORE" -gt 0 ]] || fail "the fixture produced no durable state to check"

step "update to this build"
"$HEAD_BIN" self install --prefix "$PREFIX" --force >/dev/null

step "verify the upgrade contract"
"$INSTALLED" self state --verify "$BEFORE" \
    || fail "the update changed state the registry forbids changing"

step "setup re-runs nothing"
# Configuration must survive an update: the review screen, not the wizard
# (doc 46 VII.6).
(
    cd "$WORKSPACE"
    "$INSTALLED" setup status --json > "$GATE_DIR/after-status.json" || true
)
python3 - "$GATE_DIR/after-status.json" <<'PY' || fail "setup lost a settled answer across the update"
import json, sys
state = json.load(open(sys.argv[1]))
for step in ("permission", "capabilities"):
    if state[step]["state"] == "incomplete":
        print(f"  ✗ {step} was configured before the update and is owed again after it")
        sys.exit(1)
print("  ✓ posture and capabilities are still settled")
PY

if [[ -n "$EDITED_SUM" ]]; then
    step "an edited seed is never overwritten"
    NOW_SUM="$(shasum -a 256 "$EDITED_SKILL" | awk '{print $1}')"
    [[ "$NOW_SUM" == "$EDITED_SUM" ]] \
        || fail "the update overwrote a skill the operator had edited"
    printf '  ✓ the edited skill is byte-identical\n'
fi

step "downgrade leg"
# The leg people forget: the OLDER binary must still read what the newer
# one wrote (doc 46 VII.3 rule 2).
"$PREVIOUS" self install --prefix "$PREFIX" --force >/dev/null
# `setup status` exits 1 for "read fine, not ready" and 2 for "could not
# read at all". Only the latter means the older build choked on newer
# state; treating any non-zero as failure would fail this gate on a
# fixture that simply has no provider key.
code=0
( cd "$WORKSPACE" && "$INSTALLED" setup status >/dev/null 2>&1 ) || code=$?
if ((code >= 2)); then
    fail "the previous build could not read state this build wrote (exit $code)"
fi
"$INSTALLED" self state --verify "$BEFORE" \
    || fail "downgrading changed state the registry forbids changing"
printf '  ✓ the previous build reads this build'"'"'s state\n'

printf '\n✓ upgrade gate passed\n'
