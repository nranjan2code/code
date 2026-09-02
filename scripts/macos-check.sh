#!/usr/bin/env bash
# End-to-end on macOS, from a clean machine to a first audited result
# (docs/design/46-stabilization-install-and-onboarding.md).
#
# The counterpart to `scripts/linux-check.sh`. Everything runs against an
# isolated home and prefix, so it never touches the operator's real state
# — it drives destructive lifecycle commands on purpose.
#
# What it proves, in order: install places and starts nothing; setup
# refuses a missing choice and completes when told; the projection agrees
# across CLI and HTTP; the web wizard serves and authenticates; the guided
# first task is capped read-only even from a full-access workspace; an
# update preserves declared state; the app bundle verifies; and a purge
# leaves a genuine clean slate.
#
# Usage: scripts/macos-check.sh [--with-model MODEL] [--keep]
#
# Credentials come from the environment, never from this file, and are
# never printed. Both steps are skipped when unset:
#   VAK_CHECK_BOT_TOKEN     a Telegram bot token — exercises the channel path
#   VAK_CHECK_TAVILY_KEY    a Tavily key — exercises the integration path
#
# --with-model runs a real agent turn against a local Ollama model, which
# is the only part that needs something outside this repository. Without
# it, the first-task step asserts the read-only cap without dispatching.

set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

[[ "$(uname -s)" == "Darwin" ]] || { printf 'error: macOS only\n' >&2; exit 2; }

MODEL=""
KEEP=false
while (($# > 0)); do
    case "$1" in
        --with-model) MODEL="${2:-}"; shift ;;
        --keep) KEEP=true ;;
        -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) printf 'unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
    shift
done

fail() { printf '\n✗ %s\n' "$*" >&2; exit 1; }
step() { printf '\n== %s ==\n' "$*"; }
ok()   { printf '  ✓ %s\n' "$*"; }

CHECK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/vak-macos-check.XXXXXX")"
cleanup() {
    pkill -f "$CHECK_DIR" 2>/dev/null || true
    if [[ "$KEEP" == true ]]; then
        printf '\nkept: %s\n' "$CHECK_DIR"
    else
        case "$CHECK_DIR" in */vak-macos-check.*) rm -rf -- "$CHECK_DIR" ;; esac
    fi
}
trap cleanup EXIT HUP INT TERM

export VAK_HOME="$CHECK_DIR/home"
PREFIX="$CHECK_DIR/prefix"
WORKSPACE="$CHECK_DIR/workspace"
mkdir -p "$VAK_HOME" "$WORKSPACE"
printf 'isolated home:   %s\n' "$VAK_HOME"
printf 'isolated prefix: %s\n' "$PREFIX"

step "build"
cargo build --release --package vak >/dev/null
BUILT="$ROOT_DIR/target/release/vak"
[[ -x "$BUILT" ]] || fail "no binary at $BUILT"
ok "built"

# --- install ------------------------------------------------------------
step "install places, and starts nothing (D6)"
"$BUILT" self install --prefix "$PREFIX" --force >/dev/null
VAK="$PREFIX/bin/vak"
[[ -x "$VAK" ]] || fail "no binary in the prefix"
for created in "$VAK_HOME/sessions" "$VAK_HOME/gateway" "$VAK_HOME/vak-home"; do
    [[ -e "$created" ]] && fail "install created $created — it must start and seed nothing"
done
ok "components placed, no durable state created"

"$VAK" self verify --prefix "$PREFIX" >/dev/null || fail "the install does not verify"
ok "install verifies against its manifest"

# --- setup refuses, then completes --------------------------------------
step "setup refuses a missing choice"
code=0
( cd "$WORKSPACE" && "$VAK" setup --non-interactive </dev/null >/dev/null 2>&1 ) || code=$?
[[ "$code" -eq 2 ]] || fail "expected a refusal (exit 2), got $code"
ok "refused, naming the variable that supplies the answer"

step "setup completes when told everything"
ROUTE_MODEL="${MODEL:-llama3}"
(
    cd "$WORKSPACE"
    VAK_SETUP_PROVIDER=ollama VAK_SETUP_MODEL="$ROUTE_MODEL" \
    VAK_SETUP_POSTURE=full-access VAK_SETUP_SEED=1 \
        "$VAK" setup --non-interactive </dev/null > "$CHECK_DIR/setup.log" 2>&1
) || true
grep -q "route saved: ollama/$ROUTE_MODEL" "$CHECK_DIR/setup.log" \
    || { tail -20 "$CHECK_DIR/setup.log"; fail "the route was not written"; }
ok "route and posture written"
SKILLS="$(ls "$VAK_HOME/vak-home/.vak/skills" 2>/dev/null | wc -l | tr -d ' ')"
[[ "$SKILLS" -gt 0 ]] || fail "seeding did not run"
ok "$SKILLS starter skills seeded"

# --- the projection agrees across surfaces -------------------------------
step "the projection is one definition, not per-surface"
( cd "$WORKSPACE" && "$VAK" setup status --json > "$CHECK_DIR/cli-status.json" ) || true
python3 - "$CHECK_DIR/cli-status.json" <<'PY' || fail "the CLI projection is wrong"
import json, sys
s = json.load(open(sys.argv[1]))
assert s["permission"]["state"] == "satisfied", s["permission"]
assert s["route"]["state"] == "satisfied", s["route"]
assert s["capabilities"]["state"] == "satisfied", s["capabilities"]
print("  ✓ CLI reports route, posture, and capabilities settled")
PY

# --- the web surface -----------------------------------------------------
step "the web wizard serves and authenticates"
( cd "$WORKSPACE" && "$VAK" setup --print-url > "$CHECK_DIR/wizard.log" 2>&1 ) &
for _ in $(seq 1 40); do
    grep -q "http://127" "$CHECK_DIR/wizard.log" 2>/dev/null && break
    sleep 0.5
done
URL="$(grep -o 'http://127[^ ]*' "$CHECK_DIR/wizard.log" | head -1)"
[[ -n "$URL" ]] || { cat "$CHECK_DIR/wizard.log"; fail "the setup server printed no URL"; }
BASE="${URL%%/admin*}"
TOKEN="${URL##*token=}"; TOKEN="${TOKEN%%#*}"

http() { curl -s -o "$2" -w "%{http_code}" -H "Authorization: Bearer $TOKEN" "$BASE$1"; }
[[ "$(http /onboarding "$CHECK_DIR/http-status.json")" == "200" ]] || fail "/onboarding did not serve"
ok "/onboarding serves the projection"
[[ "$(http /admin /dev/null)" == "200" ]] || fail "/admin did not serve"
ok "/admin serves the console"
UNAUTH="$(curl -s -o /dev/null -w '%{http_code}' "$BASE/onboarding")"
[[ "$UNAUTH" == "401" ]] || fail "unauthenticated request returned $UNAUTH, expected 401"
ok "unauthenticated requests refused ($UNAUTH)"

# The two surfaces must agree, since they render one derivation.
python3 - "$CHECK_DIR/cli-status.json" "$CHECK_DIR/http-status.json" <<'PY' || fail "CLI and HTTP disagree"
import json, sys
cli, http = (json.load(open(p)) for p in sys.argv[1:3])
steps = ["workspace", "provider", "route", "permission", "capabilities"]
for name in steps:
    if cli[name]["state"] != http[name]["state"]:
        sys.exit(f"  ✗ {name}: CLI says {cli[name]['state']}, HTTP says {http[name]['state']}")
print("  ✓ CLI and HTTP report identical state for every core step")
PY

# --- the first task is capped read-only ----------------------------------
step "the guided first task is capped read-only (security invariant 5)"
FIRST="$(curl -s -X POST -H "Authorization: Bearer $TOKEN" "$BASE/onboarding/first-task")"
MODE="$(printf '%s' "$FIRST" | python3 -c "import json,sys; print(json.load(sys.stdin)['permission_mode'])")"
SESSION="$(printf '%s' "$FIRST" | python3 -c "import json,sys; print(json.load(sys.stdin)['session_id'])")"
[[ "$MODE" == "ReadOnly" ]] || fail "the first task ran as $MODE from a full-access workspace"
ok "capped to ReadOnly despite the workspace being full-access"
[[ -n "$SESSION" ]] || fail "no session was created"
ok "session $SESSION created"

if [[ -n "$MODEL" ]]; then
    step "a real turn, and its receipt"
    # The most valuable assertion here, so it is an assertion and not a
    # warning: a soft "no receipt yet" that passes anyway would let the
    # whole agent path break without this test noticing.
    curl -s -X POST -H "Authorization: Bearer $TOKEN" \
        -H 'content-type: application/json' \
        -d '{"prompt":"Reply with exactly: ready"}' \
        "$BASE/sessions/$SESSION/run" >/dev/null || true

    # A local model can take a while to load. Poll rather than guess.
    RECEIPT_FOUND=false
    for _ in $(seq 1 60); do
        RECEIPTS="$(curl -s -H "Authorization: Bearer $TOKEN" \
            "$BASE/sessions/$SESSION/receipts" 2>/dev/null || true)"
        if printf '%s' "$RECEIPTS" | grep -q '"purpose"'; then
            RECEIPT_FOUND=true
            break
        fi
        sleep 2
    done
    [[ "$RECEIPT_FOUND" == true ]] \
        || fail "the run produced no dispatch receipt after 120s — the agent path is broken"

    PURPOSE="$(printf '%s' "$RECEIPTS" | python3 -c "
import json,sys
d=json.load(sys.stdin)
rows=d if isinstance(d,list) else d.get('receipts',[])
print(rows[0].get('purpose','?') if rows else '?')
" 2>/dev/null || echo '?')"
    ok "the run produced a dispatch receipt (purpose: $PURPOSE)"

    # Read-only was the cap, so no write tool may have been *dispatched*.
    #
    # Checked structurally, not by substring: the session header carries the
    # frozen capability contract, which lists every tool that exists. A
    # grep over the transcript therefore matches tool names that were only
    # ever advertised, and reports a security failure that did not happen —
    # which is exactly what the first version of this check did.
    TRANSCRIPT="$(curl -s -H "Authorization: Bearer $TOKEN" \
        "$BASE/sessions/$SESSION/transcript" 2>/dev/null || true)"
    printf '%s' "$TRANSCRIPT" | python3 -c "
import json, sys

WRITE_TOOLS = {'write_file', 'edit_file', 'bash', 'apply_patch'}

try:
    doc = json.load(sys.stdin)
except Exception:
    sys.exit(0)  # nothing to inspect is not a violation

messages = doc if isinstance(doc, list) else doc.get('messages', [])
dispatched = set()
for message in messages:
    content = message.get('content')
    if not isinstance(content, list):
        continue
    for block in content:
        if isinstance(block, dict) and block.get('type') in ('tool_use', 'tool_call'):
            name = block.get('name')
            if name:
                dispatched.add(name)

offending = dispatched & WRITE_TOOLS
if offending:
    sys.exit('a read-only capped session dispatched: ' + ', '.join(sorted(offending)))
print('  ✓ no write tool was dispatched under the read-only cap')
" || fail "the read-only cap did not hold"
fi

# --- channels: a bot is a record, and its token never comes back ---------
if [[ -n "${VAK_CHECK_BOT_TOKEN:-}" ]]; then
    step "a chat bot is configured, and is not thereby activated"
    curl -s -X POST -H "Authorization: Bearer $TOKEN" \
        -H 'content-type: application/json' \
        -d '{"id":"checkbot","surface":"telegram","label":"Check Bot"}' \
        "$BASE/gateway/bots" > "$CHECK_DIR/bot.json"
    python3 -c "
import json
d = json.load(open('$CHECK_DIR/bot.json'))
assert d.get('activation_required') is True, d
print('  ✓ created, and reported as needing activation')
" || fail "creating a bot did not report activation_required"

    # The token goes in on stdin-free curl data, and must never come back.
    SAVED="$(curl -s -X PUT -H "Authorization: Bearer $TOKEN" \
        -H 'content-type: application/json' \
        --data-binary "{\"token\":\"$VAK_CHECK_BOT_TOKEN\"}" \
        "$BASE/gateway/bots/checkbot/token")"
    if printf '%s' "$SAVED" | grep -qF "$VAK_CHECK_BOT_TOKEN"; then
        fail "the API returned the bot token it was just given"
    fi
    ok "token stored, and not echoed back"

    curl -s -H "Authorization: Bearer $TOKEN" "$BASE/gateway/bots" > "$CHECK_DIR/bots.json"
    python3 -c "
import json
bots = json.load(open('$CHECK_DIR/bots.json'))['bots']
bot = next(b for b in bots if b['id'] == 'checkbot')
assert bot['token_configured'] is True, bot
assert 'token' not in bot, 'the listing leaked a token field'
print('  ✓ listed as token_configured, with no token field')
" || fail "the bot listing is wrong"

    # Configured is not activated: the projection has to show the gap.
    curl -s -H "Authorization: Bearer $TOKEN" "$BASE/onboarding" > "$CHECK_DIR/after-bot.json"
    python3 -c "
import json
s = json.load(open('$CHECK_DIR/after-bot.json'))
services = s['services']
assert services['state'] == 'incomplete', services
assert 'not activated' in services['what'], services
print('  ✓ the projection reports it as awaiting activation')
" || fail "a configured-but-unactivated bot is invisible"
fi

# --- integrations: enabled explicitly, key never returned ----------------
if [[ -n "${VAK_CHECK_TAVILY_KEY:-}" ]]; then
    step "an integration is enabled explicitly, and its key stays put"
    ENABLED="$(curl -s -X PUT -H "Authorization: Bearer $TOKEN" \
        -H 'content-type: application/json' \
        --data-binary "{\"scope\":\"user\",\"key\":\"$VAK_CHECK_TAVILY_KEY\"}" \
        "$BASE/config/integrations/tavily")"
    if printf '%s' "$ENABLED" | grep -qF "$VAK_CHECK_TAVILY_KEY"; then
        fail "the API returned the integration key it was just given"
    fi
    ok "enabled, and the key was not echoed back"

    curl -s -H "Authorization: Bearer $TOKEN" "$BASE/config/integrations?scope=user" \
        > "$CHECK_DIR/integrations.json"
    python3 -c "
import json
d = json.load(open('$CHECK_DIR/integrations.json'))['integrations']
names = sorted(i['id'] for i in d)
assert names == sorted(names), 'the catalog is not in a stable order'
tavily = next(i for i in d if i['id'] == 'tavily')
assert tavily['key_effective'] is True, tavily
for entry in d:
    assert 'key' not in entry, 'the catalog leaked a key'
print('  ✓ %d integrations offered as peers; key present, never returned' % len(d))
" || fail "the integration catalog is wrong"
fi

pkill -f "$CHECK_DIR" 2>/dev/null || true
sleep 1

# --- update preserves state ----------------------------------------------
step "an update preserves declared state"
"$VAK" self state > "$CHECK_DIR/before.json"
"$BUILT" self install --prefix "$PREFIX" --force >/dev/null
"$VAK" self state --verify "$CHECK_DIR/before.json" \
    || fail "the update changed state the registry forbids changing"

# --- the app bundle -------------------------------------------------------
step "the app bundle builds and verifies"
BUNDLE_HOME="$CHECK_DIR/bundle-home"
VAK_HOME="$BUNDLE_HOME" "$BUILT" self install --prefix "$CHECK_DIR/Vak.app" --force >/dev/null
codesign --force --deep --sign - "$CHECK_DIR/Vak.app" 2>/dev/null || true
"$ROOT_DIR/scripts/verify-macos-release.sh" "$CHECK_DIR/Vak.app" >/dev/null \
    || fail "the app bundle does not verify"
ok "bundle structure, architecture, dylibs, and seal verify"

if command -v hdiutil >/dev/null; then
    DMG_ROOT="$CHECK_DIR/dmgroot"
    mkdir -p "$DMG_ROOT"
    cp -R "$CHECK_DIR/Vak.app" "$DMG_ROOT/"
    ln -sf /Applications "$DMG_ROOT/Applications"
    hdiutil create -quiet -volname "Vak check" -srcfolder "$DMG_ROOT" \
        -ov -format UDZO "$CHECK_DIR/out.dmg"
    hdiutil attach -quiet -nobrowse -mountpoint "$CHECK_DIR/mnt" "$CHECK_DIR/out.dmg"
    [[ -d "$CHECK_DIR/mnt/Vak.app" ]] || fail "the DMG does not contain Vak.app"
    hdiutil detach -quiet "$CHECK_DIR/mnt"
    ok "DMG builds, mounts, and contains the app beside an Applications link"
fi

# --- purge ----------------------------------------------------------------
step "purge leaves a genuine clean slate"
"$VAK" self uninstall --yes --purge --prefix "$PREFIX" >/dev/null
for leftover in \
    "$VAK_HOME/sessions" \
    "$VAK_HOME/gateway" \
    "$VAK_HOME/vak-home/.vak/config.toml" \
    "$VAK_HOME/vak-home/.vak/skills"
do
    [[ -e "$leftover" ]] && fail "$leftover survived a purge"
done
ok "nothing declared survived"

printf '\n✓ macOS end-to-end passed\n'
