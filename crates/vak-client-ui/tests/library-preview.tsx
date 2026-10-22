import { render } from "solid-js/web";
import "../src/components/ChatPane";
import LibraryPage from "../src/components/LibraryPage";
import * as store from "../src/store";
import "../src/styles.css";

const theme = new URLSearchParams(location.search).get("theme") ?? "light";
document.documentElement.dataset.theme = theme;
const at = new Date().toISOString();
const files = [
  ["html", "Inference report", "report.html", "file"],
  ["markdown", "Reading notes", "notes.md", "file"],
  ["table", "Quarterly data", "data.csv", "file"],
  ["bad-table", "Malformed data", "malformed.csv", "file"],
  ["image", "Architecture diagram", "diagram.svg", "file"],
  ["audio", "Recorded briefing", "briefing.wav", "file"],
  ["video", "Product walkthrough", "walkthrough.mp4", "file"],
  ["code", "Analysis script", "analysis.py", "file"],
  ["unknown", "Project archive", "project.zip", "file"],
  ["page", "Saved answer", "answer.json", "page"],
  ["error", "Unavailable report", "unavailable.html", "file"],
  ["doc", "Board document", "board.docx", "document"],
  ["sheet", "Financial workbook", "finance.xlsx", "document"],
  ["slides", "Strategy slides", "strategy.pptx", "document"],
  ["pdf", "Published report", "report.pdf", "document"],
];
const artifacts = files.map(([id, name, path, kind]) => ({ id, name, path, kind, agent: "vak", updated_at: at, summary: "A saved result with its own versions, actions and conversation.", starred: false, archived: false, held: false, head: "v2", versions: 2, siblings: 1, size: 1000, needs: { waiting: 0, behind: 0, missing: false, changed_outside: false }, history: [
  { id: "v1", at, from: "call", saved: true, promoted: true, digest: "a", size: 100, now: { is: "written" } },
  { id: "v2", parent: "v1", at, from: "call", saved: true, promoted: true, digest: "b", size: 200, now: { is: "in_folder" } },
], comments: [] as unknown[] }));
const html = (version: string) => `<!doctype html><html><head><style>body{font:16px/1.6 system-ui;padding:32px;max-width:900px;margin:auto;background:var(--vak-bg);color:var(--vak-fg)}h1{font-size:32px}button{font:inherit;padding:12px;border-radius:8px}section{border:1px solid var(--vak-rule);padding:24px;border-radius:12px}</style></head><body><h1>High-speed AI inference</h1><p>Enterprise impact and upgrade roadmap · ${version}</p><section><h2>A readable, interactive report</h2><p>Systems, infrastructure, people and risk, with practical next steps.</p><button onclick="this.textContent='Scenario updated'">Explore scenario</button></section><script>parent.postMessage({type:'library-test', version:'${version}', isolated: (()=>{try{return !parent.document}catch{return true}})()},'*')</script></body></html>`;
const requests: string[] = [];
const receipts: { version: string; isolated: boolean }[] = [];
window.addEventListener("message", e => { if (e.data?.type === "library-test") receipts.push(e.data); });
window.fetch = async (input, init) => {
  const url = new URL(String(input), location.origin);
  requests.push(`${init?.method ?? "GET"} ${url.pathname}`);
  const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { "Content-Type": "application/json" } });
  if (url.pathname === "/library") return json({ artifacts });
  if (url.pathname === "/agents") return json({ agents: [] });
  if (url.pathname.includes("abilities")) return json({ abilities: [] });
  const match = /^\/library\/([^/]+)(.*)$/.exec(url.pathname);
  if (!match) return json({ error: "Not in fixture" }, 404);
  const [, id, suffix] = match;
  const artifact = artifacts.find(a => a.id === id)!;
  if (!suffix) return json(artifact);
  if (suffix.endsWith("/document")) {
    const vocabulary = id === "sheet" ? "excel" : id === "slides" ? "power_point" : id === "pdf" ? "pdf" : "word";
    const anchor = id === "sheet" ? "sheet:1!A1" : id === "slides" ? "slide:1/shape:2" : "p1";
    const units = [{ anchor, kind: id === "slides" ? "shape" : "paragraph", level: 0, text: `Saved ${id} content`, labels: [] }];
    return json({ path: artifact.path, sha256: "fixture", vocabulary, kind: "Document", extension: artifact.path.split(".").pop(), macro_enabled: false, strict: false, title: artifact.name, stats: [], flags: [], sensitivity_labels: [], outline: [], total_units: units.length, from: 0, next: null, units, media: [], not_read: [] });
  }
  if (suffix === "/shares") return json({ shares: [] });
  if (suffix.endsWith("/compare")) return json({ folder: "same", kind: "text" });
  if (suffix.endsWith("/comments") && init?.method === "POST") { artifact.comments.push({ author_name: "You", version: "v1", at, text: JSON.parse(String(init.body)).text }); return json({ comment_id: "comment" }); }
  if (suffix === "/versions" && init?.method === "POST") return json({ version: "v2", sibling: false });
  if (/^\/versions\/v[12]$/.test(suffix)) {
    const version = suffix.split("/").pop()!;
    if (id === "error") return json({ error: "Version unavailable" }, 404);
    if (id === "html") return new Response(html(version));
    if (id === "markdown") return new Response("# Readable notes\n\nA **clear report** with useful next steps.\n\n- Review systems\n- Train people");
    if (id === "bad-table") return new Response('Name,Value\n"Unclosed');
    if (id === "table") return new Response('Quarter,Revenue,Notes\nQ1,100,"Includes, services"\nQ2,220,Growth');
    if (id === "image") return new Response('<svg xmlns="http://www.w3.org/2000/svg" width="640" height="240"><rect width="640" height="240" fill="indigo"/><text x="40" y="130" fill="white" font-size="32">Architecture overview</text></svg>');
    if (id === "code") return new Response('def analyze():\n    return "Analysis ready"');
    if (id === "page") {
      const item = (id: string, block: string, payload: unknown) => ({ id, turn_id: "turn", role: "tool", kind: "card", status: "succeeded", timestamp: at, content: { type: "structured", output: { semantic_type: block, block, schema_version: 2, payload } }, provenance: { presentation_id: id }, actions: [], fallback_text: "Saved interactive card" });
      return json({ format: "vak-page", title: "Saved answer", text: "A saved answer", items: [
        item("control", "controls", { title: "Scenario", inputs: [{ name: "amount", label: "Amount", kind: "number", value: 10 }], outputs: [{ name: "double", label: "Double", expression: "amount * 2" }] }),
        item("result", "stats", { readings: [{ label: "Double", value: 5 }], bound: [{ path: ["readings", 0, "value"], expression: "double", from: ["control"] }] }),
      ] });
    }
    return new Response(new Uint8Array(44));
  }
  return json({ error: "Not in fixture" }, 404);
};
store.setLibraryOpen(true);
render(() => <main style="display:flex;flex-direction:column;height:100vh"><LibraryPage /></main>, document.getElementById("root")!);
const sleep = (ms = 80) => new Promise(resolve => setTimeout(resolve, ms));
const click = (text: string, selector = "button") => {
  const element = [...document.querySelectorAll<HTMLElement>(selector)].find(e => e.textContent?.trim().startsWith(text));
  if (!element) throw new Error(`Missing control: ${text}`);
  element.click();
};
async function open(id: string) {
  if (document.querySelector(".library-artifact")) { click("Library"); await sleep(); }
  const title = artifacts.find(a => a.id === id)!.name;
  [...document.querySelectorAll<HTMLButtonElement>(".library-card")].find(e => e.querySelector("strong")?.textContent === title)!.click();
  await sleep(250);
}
Object.assign(window, { openLibraryFixture: open, runChecks: async () => {
  const passed: string[] = [];
  const assert = (ok: unknown, label: string) => { if (!ok) throw new Error(label); passed.push(label); };
  await sleep(300);
  assert(document.querySelector('.library-list.is-list') && document.querySelectorAll('.library-card').length === artifacts.length, "the compact list retains every artifact");
  click("Grid"); await sleep();
  assert(!document.querySelector('.library-list.is-list') && document.querySelectorAll('.library-card').length === artifacts.length, "grid and list show the same artifacts");
  click("List"); await sleep();
  await open("html");
  assert(!document.querySelector<HTMLDetailsElement>('.artifact-summary')?.open && document.querySelector('.artifact-summary')?.textContent?.includes(artifacts[0].summary), "secondary description is available without crowding the preview");
  assert(document.querySelector(".library-html-frame")?.getAttribute("sandbox") === "allow-forms", "HTML opens paused in an opaque sandbox by default");
  await sleep(150);
  assert(receipts.length === 0, "saved HTML does not run before permission");
  document.querySelector<HTMLDetailsElement>(".html-preview-permissions details")!.open = true;
  click("Run page code"); await sleep(200);
  assert(receipts.some(r => r.version === "v2" && r.isolated), "saved HTML runs only after permission without accessing the parent");
  assert(!document.querySelector(".library-history") && !document.querySelector(".library-comments-block"), "content gets the full reading area");
  click("Source"); await sleep();
  assert(document.querySelector("pre")?.textContent?.includes("<!doctype html>"), "source is available on request");
  click("Preview"); await sleep();
  const picker = document.querySelector<HTMLSelectElement>('[aria-label="Version to view"]')!;
  picker.value = "v1"; picker.dispatchEvent(new Event("change", { bubbles: true })); await sleep(200);
  assert(document.querySelector<HTMLIFrameElement>(".library-html-frame")!.srcdoc.includes("v1") && !receipts.some(r => r.version === "v1"), "version picker shows the older version with permission revoked");
  click("History"); await sleep();
  assert(document.querySelectorAll(".file-node").length === 2, "history preserves both versions and provenance");
  click("Compare with your folder"); await sleep();
  assert(document.body.textContent?.includes("Your folder holds this version"), "compare remains available");
  click("Comments"); await sleep();
  const note = document.querySelector<HTMLInputElement>('[aria-label="Comment"]')!;
  note.value = "Review this scenario"; note.dispatchEvent(new Event("input", { bubbles: true }));
  document.querySelector<HTMLFormElement>(".library-comments-block form")!.requestSubmit(); await sleep(200);
  assert(document.body.textContent?.includes("Review this scenario"), "comment flow remains available on the selected version");
  click("Share", ".artifact-toolbar button"); await sleep();
  assert(document.querySelector(".library-share"), "share flow remains available");
  await open("markdown"); assert(document.querySelector(".library-reading h2, .library-reading h1"), "Markdown opens as formatted prose");
  await open("table"); assert(document.querySelectorAll("tbody tr").length === 2 && document.body.textContent?.includes("Includes, services"), "CSV opens as a table preserving quoted values");
  await open("bad-table"); assert(document.body.textContent?.includes("ends inside a quoted value"), "malformed tables show the parsing problem"); click("Source"); await sleep(); assert(document.querySelector("pre")?.textContent?.includes('"Unclosed'), "malformed table source remains available");
  await open("image"); assert(document.querySelector<HTMLImageElement>(".library-media img")?.naturalWidth === 640, "saved image is displayed");
  await open("audio"); assert(document.querySelector("audio[controls]"), "audio has playback controls");
  await open("video"); assert(document.querySelector("video[controls]"), "video has playback controls");
  await open("code"); assert(document.querySelector("pre")?.textContent?.includes("def analyze"), "code stays readable as source");
  await open("unknown"); assert(document.body.textContent?.includes("Download this version"), "unsupported formats explain how to open them");
  await open("error"); assert(document.querySelector('[role="alert"]') && document.body.textContent?.includes("Try again"), "unreadable versions show an error and retry");
  await open("page"); assert(document.querySelector(".library-page-view .answer-in-canvas"), "saved pages keep their presentation renderer");
  const amount = document.querySelector<HTMLInputElement>('input[type="number"]')!;
  assert(amount, "saved pages retain interactive controls");
  amount.value = "25"; amount.dispatchEvent(new Event("input", { bubbles: true })); await sleep();
  assert(document.querySelector(".stat-value")?.textContent?.trim() === "50", "saved page controls update linked results");
  click("Comments"); await sleep(); click("Content"); await sleep();
  assert(amount.value === "25" && document.querySelector(".stat-value")?.textContent?.trim() === "50", "page interaction survives switching to comments and back");
  for (const id of ["doc", "sheet", "slides", "pdf"]) { await open(id); assert(document.querySelector(".library-document .office-view"), `${id} opens in the existing document viewer`); }
  assert(!requests.some(r => r.includes("/fs/") || r.includes("/sessions/")), "previews never substitute mutable workspace or session files");
  await open("html");
  return passed;
} });
