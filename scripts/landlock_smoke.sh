#!/bin/sh
# Verifies Landlock containment end-to-end against a built vakcoder binary.
# Requires Linux 5.13+. Usage: scripts/landlock_smoke.sh path/to/vakcoder
set -u
BIN="${1:?usage: landlock_smoke.sh <vakcoder-binary>}"
WS="$(mktemp -d)"
OUTSIDE="$(mktemp -d)"
trap 'rm -rf "$WS" "$OUTSIDE"' EXIT

fail() { echo "FAIL: $1" >&2; exit 1; }

# 1) basic exec inside the sandbox
"$BIN" __sandbox --rw "$WS" -- sh -c 'echo ok' | grep -q ok || fail "basic exec"

# 2) write inside the workspace is allowed
"$BIN" __sandbox --rw "$WS" -- sh -c "echo hi > '$WS/inside.txt'" || fail "workspace write"
grep -q hi "$WS/inside.txt" || fail "workspace write content"

# 3) read outside the workspace is allowed
"$BIN" __sandbox --rw "$WS" -- sh -c 'cat /etc/passwd >/dev/null' || fail "outside read"

# 4) write outside the workspace is denied
if "$BIN" __sandbox --rw "$WS" -- sh -c "echo no > '$OUTSIDE/deny.txt'" 2>/dev/null; then
  fail "outside write was ALLOWED"
fi

# 5) read-only mode denies even workspace writes
if "$BIN" __sandbox --ro -- sh -c "echo no > '$WS/ro.txt'" 2>/dev/null; then
  fail "read-only mode allowed a write"
fi

# 6) read-only mode denies outbound TCP connects (Landlock ABI v4)
if "$BIN" __sandbox --ro -- bash -c 'exec 3<>/dev/tcp/1.1.1.1/80' 2>/dev/null; then
  fail "read-only mode allowed an outbound TCP connect"
fi

# 7) workspace-write denies outbound TCP too (parity with Seatbelt)
if "$BIN" __sandbox --rw "$WS" -- bash -c 'exec 3<>/dev/tcp/1.1.1.1/80' 2>/dev/null; then
  fail "workspace-write allowed an outbound TCP connect"
fi

echo "landlock smoke: all scenarios passed ($BIN)"
