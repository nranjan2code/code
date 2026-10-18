import { render } from "solid-js/web";
import CanvasSaveNotice from "../src/components/canvas/CanvasSaveNotice";
import { canvasSaveState, hasUnsavedCanvasChanges, retryCanvas, startCanvasSync } from "../src/canvasSync";
import { canvasStacks, changeCanvas } from "../src/store";
import { fileSubject } from "../src/canvasSubject";
import "../src/styles.css";

document.documentElement.dataset.theme = new URLSearchParams(location.search).get("theme") === "dark" ? "dark" : "light";
const session = "canvas-save-fixture";
let responseStatus = 401;
let revision = 0;
let puts = 0;
window.fetch = async (_input, init) => {
  if (init?.method !== "PUT") return new Response(JSON.stringify({ revision, entries: [], active: null }), { headers: { "Content-Type": "application/json" } });
  puts += 1;
  const body = JSON.parse(String(init.body));
  return new Response(JSON.stringify(responseStatus === 200 ? { ...body, revision: ++revision } : { error: "fixture refusal" }), {
    status: responseStatus, headers: { "Content-Type": "application/json" },
  });
};
startCanvasSync();
render(() => <main style="padding:16px;max-width:640px;margin:auto">
  <h1>Canvas saving</h1>
  <CanvasSaveNotice status={canvasSaveState(session)} onRetry={() => retryCanvas(session)} />
</main>, document.getElementById("root")!);
const tick = () => new Promise((resolve) => setTimeout(resolve, 350));
const check = (value: unknown, message: string) => { if (!value) throw new Error(message); };
(window as any).runChecks = async () => {
  changeCanvas(session, { op: "open", subject: fileSubject("note.txt", { sessionId: session }, "Notes") });
  await tick();
  const key = canvasStacks()[session].active;
  changeCanvas(session, { op: "update", key, patch: { draft: "Keep this unsent note" } });
  await tick();
  check(canvasSaveState(session).state === "error", "A refused write remains unsaved");
  check(hasUnsavedCanvasChanges(), "Unacknowledged changes protect unload");
  check(document.querySelector('[role="status"]')?.textContent?.includes("Sign in"), "Authentication failure has a visible recovery message");
  check(canvasStacks()[session].entries[0].draft === "Keep this unsent note", "The note survives refusal");
  const attempts = puts;
  responseStatus = 200;
  document.querySelector<HTMLButtonElement>(".artifact-canvas-notice button")!.click();
  await tick();
  check(puts > attempts && canvasSaveState(session).state === "saved", "Retry saves the retained operations");
  check(!hasUnsavedCanvasChanges() && !document.querySelector('[role="status"]'), "Successful receipt clears the unsaved notice and unload protection");
  return ["refusal retention", "visible recovery", "retry", "acknowledged cleanup"].map((name) => `PASS: ${name}`);
};
