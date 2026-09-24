import { render } from "solid-js/web";
import WorkbenchPanel from "../src/components/WorkbenchPanel";
import * as store from "../src/store";
import type { SandboxRecord } from "../src/api";
import "../src/styles.css";

// A page opened after the person accepted: nothing in memory, only the
// session's durable sandbox records, shaped like the server's. The session's
// latest acceptance belongs to another execution, so the offer for the first
// one can only come from deriving it per execution.
const sid = "undo-fixture";
const receipt = (paths: string[]) => ({
  verification: paths.map((path) => ({ path, status: "observed", evidence: "destination hash verified" })),
  integration: { applied_state_digest: "sha256:4f1c2a9be07d55e2a1", workspace_state_status: "observed", target_checks_status: "passed", evidence: "ooxml verifier passed", target_checks: [] },
});
const candidate = (id: string, execution: string, path: string) => ({ kind: "Candidate", record: { record_id: `candidate-${id}`, session_id: sid, turn_id: "turn-1", result_id: "result-1", execution_id: execution, environment_id: ".vak/scratch/vak", candidate: { candidate_id: id, destination_root: "/tmp/ws", files: [{ path }] }, updated_at: "2026-09-24T10:00:00Z" } });
const promotion = (id: string, path: string) => ({ kind: "Promotion", record: { record_id: `promotion-${id}`, session_id: sid, result_id: "result-1", candidate_digest: "sha256:0", candidate_id: id, receipt: receipt([path]), updated_at: "2026-09-24T10:05:00Z" } });
const records: unknown[] = [
  candidate("c-report", "exec-1", "report.docx"),
  promotion("c-report", "report.docx"),
  candidate("c-budget", "exec-2", "budget.xlsx"),
  promotion("c-budget", "budget.xlsx"),
];
const undoCalls: string[] = [];
const originalFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const url = String(input);
  const json = (body: unknown) => new Response(JSON.stringify(body), { headers: { "Content-Type": "application/json" } });
  if (url === `/sessions/${sid}/sandbox/records`) return json({ records });
  const undo = url.match(/\/sandbox\/promotions\/([^/]+)\/undo$/);
  if (undo && init?.method === "POST") {
    const candidateId = decodeURIComponent(undo[1]);
    undoCalls.push(candidateId);
    const record = { kind: "PromotionUndo", record: { record_id: `promotion-undo-${candidateId}`, session_id: sid, candidate_id: candidateId, receipt: { restored: ["report.docx"], verification: [] }, updated_at: "2026-09-24T10:10:00Z" } };
    records.push(record);
    return json(record);
  }
  if (url.startsWith("/")) return json({});
  return originalFetch(input, init);
};

const started = (id: string, file: string) => [
  { kind: "ExecutionStarted", execution_id: id, tool: "bash", code_preview: `python edit.py ${file}`, language: "bash", scratch_dir: `.vak/scratch/vak/${id}` },
  { kind: "ArtifactGenerated", execution_id: id, path: file, mime_type: "application/octet-stream", size_bytes: 2048 },
  { kind: "ExecutionFinished", execution_id: id, exit_code: 0, duration_ms: 900 },
];
store.setActiveExecutionId(null);
store.setActiveId(sid);
store.hydrateWorkbenchExecutions(sid, [...started("exec-1", "report.docx"), ...started("exec-2", "budget.xlsx")]);
render(() => <div style="height:100vh;width:640px"><WorkbenchPanel /></div>, document.getElementById("root")!);

const settle = () => new Promise((resolve) => setTimeout(resolve, 50));
const undoButton = () => [...document.querySelectorAll("button")].find((button) => button.textContent?.trim() === "Undo acceptance");
const text = () => document.querySelector(".workbench-panel")?.textContent ?? "";
const check = (ok: unknown, message: string) => { if (!ok) throw new Error(message); return message; };

(window as any).runChecks = async () => {
  const passed: string[] = [];
  await settle();
  store.setWorkbenchTab("execution");
  await settle();
  passed.push(check(undoButton(), "the latest execution offers Undo from the records"));
  passed.push(check(text().includes("Applied 1 change(s). Exact workspace state verified; target checks passed."), "its acceptance message is shown"));
  store.setActiveExecutionId("exec-1");
  await settle();
  passed.push(check(undoButton(), "an earlier execution still offers its own Undo"));
  undoButton()!.click();
  await settle();
  passed.push(check(undoCalls.join() === "c-report", "Undo reverses that execution's acceptance"));
  passed.push(check(!undoButton(), "the offer is gone once undone"));
  passed.push(check(text().includes("Restored 1 file(s) to their pre-acceptance state."), "the restore is reported"));
  store.setActiveExecutionId("exec-2");
  await settle();
  passed.push(check(undoButton(), "the other execution's offer is unaffected"));
  return passed;
};
