import { createSignal, onCleanup, onMount } from "solid-js";
import { render } from "solid-js/web";
import ArtifactCanvas from "../src/components/ArtifactCanvas";
import { VIEWERS } from "../src/components/canvas/viewers";
import { useCanvasWorkspace } from "../src/components/canvas/WorkspaceContext";
import { createCanvasDrawingSurface } from "../src/canvasDrawing";
import * as store from "../src/store";
import "../src/styles.css";

const params = new URLSearchParams(location.search);
document.documentElement.dataset.theme = params.get("theme") === "dark" ? "dark" : "light";
const [report, setReport] = createSignal("Synthetic 2D workload in the production Canvas host");
const intervals: number[] = [], paints: number[] = [], latencies: number[] = [];
let sampling = false, inputAt = 0, mounts = 0, frames = 0;
const sid = "drawing-acceptance";
store.setActiveId(sid);
window.fetch = async () => new Response(JSON.stringify({ records: [] }), { headers: { "Content-Type": "application/json" } });
function DrawingViewer() {
  const workspace = useCanvasWorkspace()!;
  const prior = workspace.read("probe/edits");
  const [text, setText] = createSignal(typeof prior?.data === "string" ? prior.data : "");
  const [zoom, setZoom] = createSignal(1);
  let canvas!: HTMLCanvasElement, invalidate = () => {};
  let pan = { x: 0, y: 0 };
  mounts++;
  onMount(() => {
    const context = canvas.getContext("2d");
    if (!context) throw new Error("No 2D context");
    const style = getComputedStyle(document.documentElement);
    const color = (token: string) => style.getPropertyValue(token).trim();
    const surface = createCanvasDrawingSurface(canvas, frame => {
      frames++;
      context.setTransform(frame.pixelRatio, 0, 0, frame.pixelRatio, 0, 0);
      context.fillStyle = color("--surface"); context.fillRect(0, 0, frame.width, frame.height);
      context.save(); context.translate(frame.width / 2 + pan.x, frame.height / 2 + pan.y); context.scale(zoom(), zoom());
      context.strokeStyle = color("--border"); context.lineWidth = 1 / frame.pixelRatio; context.beginPath();
      for (let i = -40; i <= 40; i++) {
        context.moveTo(i * 24, -960); context.lineTo(i * 24, 960);
        context.moveTo(-960, i * 24); context.lineTo(960, i * 24);
      }
      context.stroke(); context.strokeStyle = color("--accent"); context.lineWidth = 2;
      for (let ring = 1; ring <= 10; ring++) { context.beginPath(); context.ellipse(0, 0, ring * 17, ring * 9, Math.PI / 6, 0, Math.PI * 2); context.stroke(); }
      context.restore(); context.fillStyle = color("--text"); context.font = "16px system-ui";
      context.fillText("Drawing surface · vector paths + text", 20, 32); context.font = "12px system-ui";
      context.fillText(`${Math.round(frame.width)} × ${Math.round(frame.height)} CSS pixels`, 20, 56);
      context.fillText(`${frame.pixelWidth} × ${frame.pixelHeight} backing pixels · scale ${frame.pixelRatio}`, 20, 76);
      if (inputAt) { latencies.push(performance.now() - inputAt); inputAt = 0; }
    }, { continuous: true, onError: error => setReport(`FAILED: ${String(error)}`), onFrame: timing => {
      if (sampling && timing.interval > 0) { intervals.push(timing.interval); paints.push(timing.paint); }
    } });
    invalidate = surface.invalidate; onCleanup(surface.dispose);
  });
  return <section style="height:100%;min-height:0;display:flex;flex-direction:column">
    <div style="display:flex;flex-wrap:wrap;gap:8px;padding:8px">
      <label>Workspace note <input aria-label="Workspace note" value={text()} onInput={event => {
        inputAt = performance.now(); setText(event.currentTarget.value);
        workspace.checkpoint("probe/edits", { schema: 1, summary: text(), data: text() }); invalidate();
      }} /></label>
      <button class="btn" onClick={() => workspace.clear("probe/edits")}>Mark saved</button>
      <button class="btn" onClick={() => void sample()}>Measure drawing</button>
      <button class="btn" onClick={() => store.openArtifactFile("other.txt", { sessionId: sid })}>Other tab</button>
    </div>
    <canvas ref={canvas} aria-label="Synthetic drawing probe" style="display:block;width:100%;height:100%;min-height:0;flex:1;touch-action:none"
      onPointerMove={event => { inputAt = performance.now(); const rect = canvas.getBoundingClientRect(); pan = { x: (event.clientX - rect.left - rect.width / 2) / 4, y: (event.clientY - rect.top - rect.height / 2) / 4 }; invalidate(); }}
      onWheel={event => { event.preventDefault(); inputAt = performance.now(); setZoom(value => Math.max(0.1, Math.min(16, value * Math.exp(-event.deltaY * 0.001)))); invalidate(); }} />
  </section>;
}
VIEWERS.code = DrawingViewer;
const open = () => store.openArtifactFile("drawing.txt", { sessionId: sid });
render(() => <main class="app" style={`display:block;--canvas-split-width:${store.canvasSplitWidth() === null ? "65%" : `${store.canvasSplitWidth()}px`}`}>
  <div style="padding:20px"><h1>Canvas drawing acceptance</h1><p>Production host · synthetic drawing viewer</p><button class="btn" onClick={open}>Open drawing</button><output style="display:block;white-space:pre-wrap;max-width:320px">{report()}</output></div>
  <ArtifactCanvas />
</main>, document.getElementById("root")!);
const tick = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
const settle = async () => { for (let i = 0; i < 6; i++) await tick(); };
const percentile = (values: number[], part: number) => [...values].sort((a, b) => a - b)[Math.floor((values.length - 1) * part)] ?? 0;
async function sample() {
  intervals.length = 0; paints.length = 0; sampling = true;
  for (let i = 0; i < 120; i++) await tick();
  sampling = false;
  const canvas = document.querySelector<HTMLCanvasElement>("canvas")!;
  const result = { viewport: [innerWidth, innerHeight], dpr: devicePixelRatio, visibility: document.visibilityState,
    backing: [canvas.width, canvas.height], css: [canvas.clientWidth, canvas.clientHeight], samples: intervals.length,
    medianIntervalMs: percentile(intervals, 0.5), p95IntervalMs: percentile(intervals, 0.95), observedRafHz: Math.round(1000 / percentile(intervals, 0.5)),
    p95PaintMs: percentile(paints, 0.95), inputToDrawP95Ms: latencies.length ? percentile(latencies, 0.95) : null, frames, mounts,
    noPageOverflow: document.documentElement.scrollWidth <= innerWidth,
    fidelity: canvas.width === Math.round(canvas.getBoundingClientRect().width * devicePixelRatio) && canvas.height === Math.round(canvas.getBoundingClientRect().height * devicePixelRatio) };
  setReport(JSON.stringify(result, null, 2)); (window as any).__drawingReport = result; return result;
}
(window as any).runChecks = async () => {
  open(); await settle();
  const input = document.querySelector<HTMLInputElement>('[aria-label="Workspace note"]')!;
  input.value = "Keep my drawing edits"; input.dispatchEvent(new Event("input", { bubbles: true }));
  store.openArtifactFile("other.txt", { sessionId: sid }); await settle(); open(); await settle();
  if (document.querySelector<HTMLInputElement>('[aria-label="Workspace note"]')?.value !== "Keep my drawing edits") throw new Error("Edits lost on remount");
  document.querySelector<HTMLButtonElement>('[aria-label="Close Canvas"]')!.click(); await settle();
  if (!document.body.textContent?.includes("Discard edits and close")) throw new Error("Missing dirty close choice");
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Keep working")!.click(); await settle();
  const tab = document.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]')!;
  tab.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true })); await settle();
  if (store.canvasEntry()?.subject.title === "drawing.txt") throw new Error("Keyboard tabs did not move");
  open(); await settle();
  if (matchMedia("(min-width:1101px)").matches) {
    document.querySelector<HTMLElement>('[aria-label="Resize Canvas"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true })); await settle();
    if (store.canvasSplitWidth() === null) throw new Error("Keyboard resize did not apply");
  }
  if (store.canvasEntry()?.subject.kind !== "file" || store.canvasEntry()?.subject.title !== "drawing.txt") throw new Error("Returning to the drawing failed");
  const result = await sample();
  if (!result.fidelity || !result.noPageOverflow || !result.samples) throw new Error("Drawing geometry or sampling failed");
  document.documentElement.dataset.checks = "PASS: checkpoint remount, dirty close, keyboard tabs, responsive sizing, backing pixels, frame sampling";
};
if (params.has("run")) void (window as any).runChecks().catch((error: unknown) => { setReport(`FAILED: ${String(error)}`); document.documentElement.dataset.checks = `FAILED: ${String(error)}`; });
else open();
