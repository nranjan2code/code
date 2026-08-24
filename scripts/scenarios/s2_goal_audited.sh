#!/usr/bin/env bash
# Scenario: goal mode reaches audited completion offline — deterministic
# verify criterion passes, judged criterion echoed PASS by the mock audit.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
start_mock
new_workspace
setup_offline
OUT=$(cd "$WORK_DIR" && $BIN exec \
  --goal "Scenario goal (offline)" \
  --criteria "judged criterion A,judged criterion B" \
  --yes "SCENARIO-GOAL proceed." 2>&1)
echo "$OUT" | grep -q "── completed" && pass "run completed" || fail "completed"
LEDGER=$(ls -t "$WORK_DIR/home/sessions/"*/*.jsonl | head -1)
echo "ledger=$LEDGER"
GOALS=$(python3 -c "
import json,sys
goals=[json.loads(l).get('status') for l in open('$LEDGER') if json.loads(l).get('kind')=='goal']
print(json.dumps(goals))
")
echo "goals=$GOALS"
[[ "$GOALS" == *done* && "$GOALS" == *audited* ]] \
  && pass "goal done{audited:true}" || fail "goal status"
grep -q '"purpose":"verify"' "$LEDGER" && pass "judge receipted as verify" || fail "verify receipt"
stop_mock
exit ${SCENARIO_FAILED:-0}
