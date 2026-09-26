import { render } from "solid-js/web";
// ChatPane first: the renderer and the app import each other, and the page
// has to enter that cycle where the app does.
import "../src/components/ChatPane";
import PresentationTimelineView from "../src/components/PresentationRenderer";
import * as store from "../src/store";
import type { ArtifactStatus, OutputItem, OutputTimeline } from "../src/types";
import "../src/styles.css";

// File results as the server projects them (docs/design/75 §6.1): answers
// with their files, each file's status derived from durable records. The
// older draft is still waiting, so only the newer one's Review changes is
// primary.
const sid = "result-card-fixture";
const page = (heading: string, line: string) => `<!doctype html><html><head><title>${heading}</title></head><body style="font-family: Georgia, serif; padding: 48px"><h1 style="font-size: 72px">${heading}</h1><p style="font-size: 40px">${line}</p></body></html>`;
const chart = `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="320" height="200" fill="#fcefd9"/><rect x="40" y="90" width="50" height="80" fill="#2f3c94"/><rect x="130" y="50" width="50" height="120" fill="#2f3c94"/><rect x="220" y="20" width="50" height="150" fill="#f5a400"/></svg>`;
const doc = (text: string) => ({ schema_version: 2, blocks: [{ type: "paragraph", id: text, content: [{ type: "text", text }] }], source_markdown: text, metadata: {}, diagnostics: [] });
const outcome = (result: string) => ({ result_id: result, status: "succeeded", completion: null, evidence_state: null, requirement_ids: [], evidence_receipt_ids: [], evidence: [], human_review: null });
const ago = (hours: number) => new Date(Date.now() - hours * 3600_000).toISOString();
const user = (turn: string, text: string, at: string): OutputItem => ({ id: `${turn}-user`, turn_id: turn, timestamp: at, role: "user", kind: "message", status: "succeeded", content: { type: "document", document: doc(text) }, actions: [], fallback_text: text } as unknown as OutputItem);
const answer = (turn: string, text: string, at: string): OutputItem => ({ id: `${turn}-answer`, turn_id: turn, timestamp: at, role: "assistant", kind: "outcome", status: "succeeded", outcome: outcome(`${turn}-result`), content: { type: "document", document: doc(text) }, actions: [], fallback_text: text } as unknown as OutputItem);
const file = (turn: string, id: string, name: string, path: string, at: string, status: ArtifactStatus, execution?: string, size = 1480): OutputItem => ({
  id, turn_id: turn, timestamp: at, role: "tool", kind: "artifact", status: "succeeded", outcome: outcome(`${turn}-result`),
  content: { type: "artifact", artifact: { name, path, media_type: null, description: null, size_bytes: size, status } },
  provenance: { session_id: sid, tool_call_id: execution ?? id },
  actions: [{ id: `open-${id}`, label: "Open", verb: "open_artifact", data: { path } }, ...(execution ? [{ id: `review-${execution}`, label: "Review draft", verb: "review_draft", data: { execution_id: execution } }] : [])],
  fallback_text: path,
} as unknown as OutputItem);
const timeline: OutputTimeline = { schema_version: 2, session_id: sid, diagnostics: [], items: [
  user("t1", "Draft a one-page welcome for the family reunion.", ago(30)),
  answer("t1", "Here is a first draft of the welcome page.", ago(30)),
  file("t1", "f1", "welcome.html", ".vak/scratch/vak/exec-1/welcome.html", ago(30), { state: "draft", version: 1 }, "exec-1"),
  user("t2", "Make the review page friendlier.", ago(0.01)),
  answer("t2", "The revision replaces the placeholder with one clear welcome sentence.", ago(0.01)),
  file("t2", "f2", "isolated-review.html", ".vak/scratch/vak/exec-2/isolated-review.html", ago(0.01), { state: "draft", version: 2, saved_as: { version_id: "v2", path: "isolated-review.html" } }, "exec-2"),
  user("t3", "Chart the three quarters.", ago(3)),
  answer("t3", "The chart is in your folder.", ago(3)),
  file("t3", "f3", "chart.svg", ".vak/scratch/vak/exec-3/chart.svg", ago(3), { state: "accepted", version: 3, saved_as: { version_id: "v3", path: "chart.svg" } }, "exec-3"),
  user("t4", "Write the notes as a document.", ago(50)),
  answer("t4", "I saved the notes.", ago(50)),
  file("t4", "f4", "notes.docx", "notes.docx", ago(50), { state: "in_folder" }),
  user("t5", "Export both lists.", ago(1)),
  answer("t5", "Both lists are ready for review.", ago(1)),
  file("t5", "f5a", "guests.csv", ".vak/scratch/vak/exec-5/guests.csv", ago(1), { state: "draft", version: 1 }, "exec-5"),
  file("t5", "f5b", "menu.csv", ".vak/scratch/vak/exec-5/menu.csv", ago(1), { state: "draft", version: 1 }, "exec-5"),
] } as unknown as OutputTimeline;

const originalFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const url = new URL(String(input), location.origin);
  const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
  if (url.pathname === `/sessions/${sid}/sandbox/candidates/v2/files`) return json({ path: "isolated-review.html", kind: "text", bytes: 1, editable: false, content: page("Isolated Review Draft", "Welcome to the review draft.") });
  if (url.pathname === `/sessions/${sid}/sandbox/candidates/v3/files/raw`) return new Response(chart, { headers: { "Content-Type": "image/svg+xml" } });
  if (url.pathname === "/fs/file" && url.searchParams.get("path")?.endsWith("welcome.html")) return json({ path: "welcome.html", kind: "text", bytes: 1, editable: false, content: page("Family reunion", "We are glad you are here.") });
  if (url.pathname.startsWith("/")) return json({ error: "not in fixture" }, 404);
  return originalFetch(input, init);
};

store.setActiveId(sid);
store.hydrateFromPresentation(sid, timeline);
render(() => <main style="max-width: 820px; margin: 0 auto; padding: 24px 16px"><PresentationTimelineView timeline={store.presentationOf(sid)!} sessionId={sid} /></main>, document.getElementById("root")!);

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const card = (name: string) => [...document.querySelectorAll<HTMLElement>(".result-card")].find((element) => element.querySelector(".result-card-name")?.textContent === name)!;
const assert = (condition: unknown, message: string) => { if (!condition) throw new Error(message); return message; };
(window as unknown as { setTechnical: (on: boolean) => void }).setTechnical = (on) => store.setTechnicalDetails(on);
(window as unknown as { runChecks: () => Promise<string[]> }).runChecks = async () => {
  store.setTechnicalDetails(false);
  await sleep(1200);
  const passed: string[] = [];
  const cards = [...document.querySelectorAll<HTMLElement>(".result-card")];
  passed.push(assert(cards.length === 6, `six file cards (${cards.length})`));
  passed.push(assert(card("isolated-review.html").innerText.includes("Draft, version 2, waiting for your review"), "the draft says its version and that it waits for review"));
  passed.push(assert(card("isolated-review.html").innerText.includes("your folder hasn't changed yet"), "the draft says the folder has not changed"));
  const primaries = [...document.querySelectorAll(".result-card .btn.primary")];
  passed.push(assert(primaries.length === 1 && card("isolated-review.html").contains(primaries[0]), "only the newest waiting draft's Review changes is primary"));
  passed.push(assert(card("welcome.html").querySelector(".btn")?.textContent === "Review changes", "the older draft still offers Review changes"));
  passed.push(assert(card("isolated-review.html").querySelector("iframe")?.getAttribute("srcdoc")?.includes("Welcome to the review draft."), "the preview draws the newest saved version"));
  passed.push(assert(card("isolated-review.html").querySelector("iframe")?.getAttribute("sandbox") === "allow-scripts", "the preview frame is sandboxed"));
  passed.push(assert(card("chart.svg").innerText.includes("Accepted, version 3") && card("chart.svg").querySelector("img"), "an accepted image shows its picture and status"));
  passed.push(assert(card("notes.docx").innerText.includes("Saved in your folder") && !card("notes.docx").innerText.includes("Review changes"), "a file saved straight to the folder has no review"));
  passed.push(assert(!/\d\s*(bytes|KB|MB)\b/.test(document.body.innerText), "no size shows in everyday view"));
  passed.push(assert(card("isolated-review.html").innerText.includes("Ask for changes"), "a single file's card carries Ask for changes"));
  const multi = card("guests.csv").closest(".primary-result")!;
  passed.push(assert(multi.querySelectorAll(".result-card-link").length === 0 && multi.querySelector(".primary-result-actions")?.textContent?.includes("Ask for changes"), "a result with two files asks for changes once, under both"));
  passed.push(assert(card("guests.csv").innerText.includes("Review changes") && !card("menu.csv").innerText.includes("Review changes"), "files from one run offer Review once"));
  const answerFirst = card("isolated-review.html").closest(".primary-result")!;
  passed.push(assert(answerFirst.firstElementChild?.classList.contains("primary-result-answer"), "the answer comes before its file"));
  store.setTechnicalDetails(true);
  await sleep(100);
  passed.push(assert(card("isolated-review.html").querySelector(".result-card-technical")?.textContent === "1.4 KB · .vak/scratch/vak/exec-2/isolated-review.html", "technical details add the size and path"));
  store.setTechnicalDetails(false);
  passed.push(assert(document.documentElement.scrollWidth <= document.documentElement.clientWidth, "no sideways scroll"));
  return passed;
};
