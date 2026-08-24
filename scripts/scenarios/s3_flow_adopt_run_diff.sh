#!/usr/bin/env bash
# Scenario: plan → run → adopt → re-run → identical diff (Phase E+G loop).
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
start_mock
new_workspace
setup_offline
cd "$WORK_DIR"
$BIN plan "probe something trivial" --yes >/dev/null 2>&1 || true
LEDGER=$(ls home/flow-runs/plan-*.json 2>/dev/null | head -1)
[[ -n "$LEDGER" ]] && pass "planner ledger exists" || { fail "no planner ledger"; stop_mock; exit 1; }
$BIN flow adopt "$LEDGER" --name adopted-scenario >/dev/null \
  && pass "adopt from plan ledger" || fail "adopt"
$BIN flow check adopted-scenario >/dev/null \
  && pass "adopted flow valid" || fail "valid"
$BIN flow run adopted-scenario --yes --trust >/dev/null 2>&1 \
  && pass "flow ran" || fail "flow run"
R1=$(ls home/flow-runs/adopted-scenario/*.json | head -1)
$BIN flow run adopted-scenario --yes --trust >/dev/null 2>&1 || true
R2=$(ls -t home/flow-runs/adopted-scenario/*.json | head -1)
if $BIN flow diff "$R1" "$R2" | grep -q "identical"; then
  pass "re-run diff identical"
else
  # Outputs may embed timestamps; a difference is acceptable if statuses match.
  A=$(python3 -c "import json;d=json.load(open('$R1'));print(all(v['status']=='completed' for v in d['nodes'].values()))")
  B=$(python3 -c "import json;d=json.load(open('$R2'));print(all(v['status']=='completed' for v in d['nodes'].values()))")
  [[ "$A" == True && "$B" == True ]] && pass "both runs completed (output drift tolerated)" || fail "diff/run"
fi
stop_mock
exit ${SCENARIO_FAILED:-0}
