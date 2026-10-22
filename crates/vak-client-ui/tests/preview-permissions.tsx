import { createSignal, Show } from "solid-js";
import { inlineSubject } from "../src/canvasSubject";
import { render } from "solid-js/web";
import HtmlViewer from "../src/components/canvas/HtmlViewer";
import HtmlPreview from "../src/components/HtmlPreview";
import Sheet from "../src/components/Sheet";
import OfficeChangeList from "../src/components/OfficeChangeList";
import { artifactPreviewMarkup } from "../src/artifactPreview";
import { sandboxedSrcdoc } from "../src/safeUrl";
import type { ViewerHandle } from "../src/components/canvas/types";
import "../src/styles.css";

document.documentElement.dataset.theme = new URLSearchParams(location.search).get("theme") ?? "light";
const [review, setReview] = createSignal(false);
const [view, setView] = createSignal("preview");
const [reload, setReload] = createSignal(0);
const [libraries, setLibraries] = createSignal<string[]>([]);
let handle: ViewerHandle | null = null;
const receipts: string[] = [];
window.addEventListener("message", e => { if (typeof e.data === "string") receipts.push(e.data); });
const html = `<!doctype html><h1>Daily pulse</h1><p>Interactive market chart</p><button onclick="parent.postMessage('clicked','*')">Explore scenario</button><div id="chart"></div><script>document.querySelector('#chart').textContent='Chart drawn';let isolated=false;try{parent.document.body}catch{isolated=true}parent.postMessage(isolated?'isolated':'escaped','*')</script><script src="https://example.invalid/external.js"></script><script type="application/json">{"data":true}</script>`;
const local = `<h1>Local chart</h1><script src="chart.js"></script>`;
const reader = { readFile: async (path: string) => ({ path, kind: "text" as const, bytes: 64, editable: false, content: `parent.postMessage('local:${path}','*')` }), readFileRaw: async () => { throw new Error("Unexpected media read"); } };
const previewSubject = await inlineSubject({ title: "Daily pulse", html });
render(() => <main style="height:100dvh;display:flex;flex-direction:column;min-width:0">
  <nav style="display:flex;flex-wrap:wrap;gap:8px;padding:8px">
    <button id="review" class="btn" onClick={() => setReview(true)}>Review spreadsheet</button>
    <button id="source" class="btn" onClick={() => setView(view() === "source" ? "preview" : "source")}>Source / Preview</button>
    <button id="reload" class="btn" onClick={() => setReload(x => x + 1)}>Reload</button>
  </nav>
  <section id="canvas-preview" style="flex:1;min-height:0"><HtmlViewer subject={previewSubject} view={view()} reloadKey={reload()} selection={null} onSelect={() => {}} register={value => { handle = value; }} /></section>
  <section id="local-preview" style="height:260px"><HtmlPreview html={local} page={sandboxedSrcdoc(local)} title="Local chart" sourceLabel="Synthetic selected file" frameClass="library-html-frame" localFiles libraries={libraries()} prepare={() => artifactPreviewMarkup("site/index.html", local, reader)} /></section>
  {/* The production Details container is a CSS size container. It establishes
      a containing block for fixed descendants; Review must escape it. */}
  <aside hidden={!review()} style="position:fixed;right:0;top:0;width:360px;height:100%;pointer-events:none;transform:translateX(0);container-type:inline-size;overflow:hidden">
    <div class="workbench-panel"><Show when={review()}><Sheet title="Review changes" subtitle="Nothing changes in the folder until you accept." size="wide" class="candidate-review" onClose={() => setReview(false)} footer={<><span>Applying 1 selected change to the folder · 1 viewed</span><button class="btn">Keep as draft</button><button class="btn primary" id="accept">Accept 1 selected file</button></>}>
      <section class="candidate-review-section"><div class="candidate-review-section-head"><h3>indian-markets-today-2026-10-21.xlsx</h3><button class="btn">Focus on this file</button><button class="btn">Open saved version in Canvas</button></div><p>Open the saved version to see the whole document. The changes are listed below.</p></section>
      <section class="candidate-review-section"><h3>Changes</h3><OfficeChangeList review={{ path: "market.xlsx", compared_with: "nothing (new file)", summary: [], flags: [], changes: Array.from({length:40}, (_, i) => ({section:"Index Data", anchor:`Index Data!A${i+1}`, kind:"added", before:null, after:`Index ${i+1}`})) }} /></section>
    </Sheet></Show></div>
  </aside>
</main>, document.getElementById("root")!);
const wait = () => new Promise(r => setTimeout(r, 200));
const check = (ok: unknown, label: string) => { if (!ok) throw new Error(label); return label; };
const button = (root: Element, text: string) => [...root.querySelectorAll<HTMLButtonElement>("button")].find(x => x.textContent === text)!;
const within = (el: Element) => { const r = el.getBoundingClientRect(); return r.left >= -1 && r.top >= -1 && r.right <= innerWidth+1 && r.bottom <= innerHeight+1; };
Object.assign(window, { runChecks: async () => {
  const passed: string[] = [];
  await wait();
  const canvas = document.querySelector("#canvas-preview")!;
  passed.push(check(receipts.length === 0, "No document code executes automatically"));
  canvas.querySelector<HTMLDetailsElement>("details")!.open = true;
  passed.push(check(canvas.textContent?.includes("1 embedded scripts · 1 inline actions · 0 local script files · 1 external script references"), "Script report separates embedded code, event handlers, external references and inert data"));
  passed.push(check(canvas.textContent?.includes("WebRTC") && canvas.textContent?.includes("network connections"), "Network limitation disclosed before execution"));
  button(canvas,"Run page code").click(); await wait();
  passed.push(check(receipts.includes("isolated") && !receipts.includes("escaped"), "Opted-in code draws the chart without parent access"));
  passed.push(check(handle?.safetyLabel?.includes("network access possible"), "Canvas footer reports effective permission"));
  passed.push(check(canvas.querySelector("iframe")?.getAttribute("sandbox") === "allow-scripts allow-forms", "Interactive frame has no same-origin, popups or native privilege"));
  const interactive = canvas.querySelector<HTMLIFrameElement>("iframe")!.srcdoc;
  passed.push(check(interactive.includes("script-src 'unsafe-inline';") && interactive.includes("connect-src 'none'"), "External scripts and fetch stay denied"));
  document.querySelector<HTMLButtonElement>("#source")!.click(); await wait();
  document.querySelector<HTMLButtonElement>("#source")!.click(); await wait();
  passed.push(check(receipts.filter(x=>x==='isolated').length === 1, "Source navigation preserves the running document"));
  button(canvas,"Stop page code").click(); await wait();
  passed.push(check(canvas.querySelector("iframe")?.getAttribute("sandbox") === "allow-forms" && !handle?.safetyLabel?.includes("possible"), "Stop destroys the active frame and restores offline status"));
  button(canvas,"Run page code").click(); await wait();
  document.querySelector<HTMLButtonElement>("#reload")!.click(); await wait();
  passed.push(check(canvas.querySelector("iframe")?.getAttribute("sandbox") === "allow-forms", "Reload revokes permission"));
  const local = document.querySelector("#local-preview")!;
  local.querySelector<HTMLDetailsElement>("details")!.open = true;
  button(local,"Run page code").click(); await wait();
  passed.push(check(receipts.includes("local:site/chart.js"), "Local script files run only through the selected file reader"));
  setLibraries(["diagram"]); await wait();
  passed.push(check(local.querySelector("iframe")?.getAttribute("sandbox") === "allow-forms", "Changing requested libraries revokes running permission"));
  document.querySelector<HTMLButtonElement>("#review")!.focus(); document.querySelector<HTMLButtonElement>("#review")!.click(); await wait();
  const dialog = document.querySelector(".candidate-review")!;
  passed.push(check(!dialog.closest(".workbench-panel") && within(dialog), "Review escapes the containing Details panel and fits the viewport"));
  passed.push(check(within(dialog.querySelector(".sheet-close")!) && within(dialog.querySelector("#accept")!) && within(button(dialog,"Keep as draft")), "Close and decision controls remain inside the window"));
  const body = dialog.querySelector(".sheet-body")!;
  passed.push(check(body.scrollHeight > body.clientHeight && body.scrollWidth <= body.clientWidth + 1, "Review body scrolls vertically without horizontal overflow"));
  passed.push(check(dialog.contains(document.activeElement), "Review receives focus"));
  const accept = dialog.querySelector<HTMLButtonElement>("#accept")!; accept.focus(); accept.dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",bubbles:true,cancelable:true}));
  passed.push(check(document.activeElement === dialog.querySelector(".sheet-close"), "Tab stays inside Review"));
  window.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})); await wait();
  passed.push(check(!document.querySelector(".candidate-review") && document.activeElement === document.querySelector("#review"), "Escape closes Review and restores focus"));
  passed.push(check(document.documentElement.scrollWidth <= innerWidth + 1, "Page fits the viewport"));
  return passed;
}, showReview: () => setReview(true) });
