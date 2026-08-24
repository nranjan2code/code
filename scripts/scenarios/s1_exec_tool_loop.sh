#!/usr/bin/env bash
# Scenario: headless exec runs the tool loop end to end — prompt → bash
# call → result → final answer, fully audited in the session ledger.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
start_mock
new_workspace
setup_offline
OUT=$(cd "$WORK_DIR" && $BIN exec "run the smoke test" --yes 2>&1)
echo "$OUT" | grep -q "smoke-ok. Task complete." && pass "final answer streamed" || fail "final answer"
echo "$OUT" | grep -q "✓ bash\|completed" && pass "tool executed" || fail "tool executed"
LEDGER=$(ls -t "$WORK_DIR/home/sessions/"*/*.jsonl | head -1)
grep -q '"kind":"receipt"' "$LEDGER" && pass "receipts in ledger" || fail "receipts"
grep -q '"kind":"header"' "$LEDGER" && grep -q '"route_ladder"' "$LEDGER" && pass "frozen contract w/ ladder field" || fail "contract"
stop_mock
exit ${SCENARIO_FAILED:-0}
