import { render } from "solid-js/web";
import { Show } from "solid-js";
import ChatPane from "../src/components/ChatPane";
import WorkbenchPanel from "../src/components/WorkbenchPanel";
import * as store from "../src/store";
import { artifactPreviewHtml } from "../src/artifactPreview";
import { assistantParts } from "../src/structured";
import "../src/styles.css";

const sid = "render-fixture";
const card = { semantic_type: "metric", schema_version: 2, skill_id: "core", skill_version: "1.0.0", payload: { label: "Files reviewed", value: 12, unit: "files" } };
const answer = `Here is the result.\n\n\`\`\`vak\n${JSON.stringify(card)}\n\`\`\`\n\nAll twelve files are ready for review.`;
const originalFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const url = String(input);
  if (url.startsWith("/fs/file?")) {
    const path = new URL(url, location.origin).searchParams.get("path")!;
    const content = path.endsWith("style.css") ? "body { background: rgb(240, 248, 242); color: #243c2a; font-family: system-ui; padding: 24px }" : path.endsWith("app.js") ? "document.querySelector('h1').textContent = 'Preview is interactive';" : '<html><head><link rel="stylesheet" href="style.css"></head><body><h1>Preview</h1><script src="app.js"></script></body></html>';
    return new Response(JSON.stringify({ kind: "text", content }), { headers: { "Content-Type": "application/json" } });
  }
  if (url.includes("/control")) return new Response(JSON.stringify({ paused: false, revision: 1 }));
  if (url.startsWith("/")) return new Response("{}", { headers: { "Content-Type": "application/json" } });
  return originalFetch(input, init);
};
const messages = (text = answer) => [
  { role: "user", content: [{ type: "text", text: "Review the files and show me a clean result." }] },
  { role: "assistant", content: [{ type: "text", text }] },
];
const doc = (text: string) => ({ schema_version: 2, blocks: [{ type: "paragraph", id: text, content: [{ type: "text", text }] }], source_markdown: text, metadata: {}, diagnostics: [] });
const snapshot = () => ({ schema_version: 2, session_id: sid, diagnostics: [], items: [
  { id: "user-1", turn_id: "turn-1", role: "user", kind: "message", status: "succeeded", content: { type: "document", document: doc("Review the files and show me a clean result.") }, actions: [], fallback_text: "Review the files and show me a clean result." },
  { id: "assistant-1", turn_id: "turn-1", role: "assistant", kind: "outcome", status: "succeeded", content: { type: "document", document: { ...doc(answer), blocks: [{ type: "structured", id: "metric-1", output: card, fallback_markdown: "Files reviewed: 12 files" }, ...doc("All twelve files are ready for review.").blocks] } }, actions: [], fallback_text: answer },
] });
store.setDockTab(null);
store.setActiveId(sid);
store.setPresentationMode("everyday");
store.hydrateFromTranscript(sid, messages() as any);
render(() => <div style="display:flex;height:100vh;width:100%"><main style="flex:1;min-width:0;display:flex;flex-direction:column"><header style="padding:16px;border-bottom:1px solid var(--border)"><strong>Vak</strong><button onClick={() => store.setPresentationMode(store.presentationMode() === "everyday" ? "advanced" : "everyday")}>{store.presentationMode()}</button></header><ChatPane /></main><Show when={store.dockTab() === "workbench"}><aside style="width:42%;min-width:280px"><WorkbenchPanel /></aside></Show></div>, document.getElementById("root")!);
const tick = () => new Promise((resolve) => setTimeout(resolve, 80));
const assert = (condition: unknown, message: string) => { if (!condition) throw new Error(message); };
(window as any).runChecks = async () => {
  const passed: string[] = [];
  const check = (condition: unknown, message: string) => { assert(condition, message); passed.push(message); };
  await tick();
  check(document.querySelector(".chat")!.textContent!.includes("Files reviewed"), "Saved transcript renders a card");
  check(!document.querySelector(".chat")!.textContent!.includes("```"), "Transport fence delimiters are absent");
  check(!document.querySelector(".chat")!.textContent!.includes("semantic_type"), "Transport JSON is absent from chat");
  store.setHydratingId(sid); await tick();
  check(!document.querySelector(".transcript-skeleton"), "Completion refresh preserves visible output");
  store.hydrateFromPresentation(sid, snapshot() as any); await tick();
  check(document.querySelector(".chat")!.textContent!.includes("Files reviewed"), "Durable presentation renders the same result");
  store.setHydratingId(null);
  store.setPresentationMode("advanced"); await tick();
  check(!document.querySelector(".semantic-render-audit, .semantic-diagnostic, .presentation-feedback"), "Advanced has no automatic debug chrome");
  check(!store.dockTab(), "Advanced does not automatically open a dock");
  const oldCard = document.querySelector(".semantic-turn");
  store.appendUser(sid, "Please add a summary."); store.markRunning(sid, true);
  store.applyEvent(sid, { Stream: { TextDelta: { delta: "One more result.", partial: {} } } } as any, {}); await tick();
  check(document.querySelector(".semantic-turn") === oldCard, "Previous turn remains mounted during next turn");
  const bubble = document.querySelector(".msg.assistant");
  store.applyEvent(sid, { Stream: { TextDelta: { delta: " With more detail.", partial: {} } } } as any, {}); await tick();
  check(document.querySelector(".msg.assistant") === bubble, "Streaming keeps the assistant bubble mounted");
  store.applyEvent("background-task", { Sandbox: { kind: "ExecutionStarted", execution_id: "other", tool: "bash", language: "sh", scratch_dir: ".vak/scratch/other", code_preview: "echo other" } } as any, {});
  check(!store.workbenchExecutions().some((item) => item.id === "other"), "Background execution cannot contaminate focused Workbench");
  const preview = await artifactPreviewHtml(".vak/scratch/result/index.html", '<head><link rel="stylesheet" href="style.css"></head><body><h1>Preview</h1><script src="app.js"></script></body>');
  check(preview.includes("rgb(240, 248, 242)") && preview.includes("Preview is interactive") && !preview.includes('src="app.js"'), "Preview resolves relative CSS and JavaScript through authenticated reads");
  check(preview.includes("connect-src 'none'") && preview.includes("base-uri 'none'"), "Preview retains network and origin isolation");
  check(assistantParts('```vak\n{"semantic_type":', true).length === 0, "Incomplete transport is held while streaming");
  check(assistantParts('```json\n{"ordinary":true}\n```').some((part) => part.type === "text"), "Ordinary JSON remains ordinary content");
  let refused = false;
  try { await artifactPreviewHtml(".vak/scratch/result/index.html", '<img src="../private.png">'); } catch { refused = true; }
  check(refused, "Artifact assets cannot traverse outside the preview directory");
  const previewRan = await new Promise<boolean>((resolve) => {
    const frame = document.createElement("iframe");
    frame.setAttribute("sandbox", "allow-scripts");
    const listener = (event: MessageEvent) => {
      if (event.source !== frame.contentWindow || event.data?.check !== "preview-ran") return;
      clearTimeout(timer); window.removeEventListener("message", listener); frame.remove();
      resolve(event.data.color === "rgb(240, 248, 242)" && event.data.text === "Preview is interactive");
    };
    const timer = window.setTimeout(() => { window.removeEventListener("message", listener); frame.remove(); resolve(false); }, 2000);
    window.addEventListener("message", listener);
    frame.srcdoc = preview.replace("</body>", '<script>parent.postMessage({check:"preview-ran",color:getComputedStyle(document.body).backgroundColor,text:document.querySelector("h1").textContent},"*")</script></body>');
    document.body.append(frame);
  });
  check(previewRan, "Sandboxed iframe executes the relative script and applies its stylesheet");
  store.markRunning(sid, false);
  store.hydrateFromTranscript(sid, messages() as any);
  store.hydrateFromPresentation(sid, snapshot() as any);
  store.setPresentationMode("everyday");
  store.clearPresentation(sid);
  store.hydrateFromTranscript(sid, messages(Array.from({length: 80}, (_, i) => `Paragraph ${i}: a longer answer to verify reading position.`).join("\n\n")) as any);
  await tick();
  const scroller = document.querySelector<HTMLDivElement>(".chat")!;
  scroller.scrollTop = 100; scroller.dispatchEvent(new Event("scroll"));
  store.appendUser(sid, "Continue the explanation."); await tick();
  check(scroller.scrollTop < 200, "New output respects the reader's scroll position");
  store.hydrateFromTranscript(sid, messages() as any);
  store.hydrateFromPresentation(sid, snapshot() as any);
  return passed;
};
(window as any).showArtifact = () => store.openWorkbenchArtifact(".vak/scratch/result/index.html");
