import { render } from "solid-js/web";
import ArtifactCanvas from "../src/components/ArtifactCanvas";
import * as store from "../src/store";
import { fileSubject } from "../src/canvasSubject";
import { VIEWERS } from "../src/components/canvas/viewers";
import "../src/styles.css";

// Every route the Canvas can read a file through answers with its own marker,
// and each request is logged, so a check can tell which route served a subject.
const sid = "canvas-fixture";
const requests: string[] = [];
const previewCalls: string[] = [];
/** Whether the stub server can give a page an origin of its own (`POST /previews`). */
let previews: "origin" | "none" = "none";
const page = (source: string) => `<!doctype html><html><head><title>${source}</title></head><body><h1>${source}</h1></body></html>`;
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
window.fetch = async (input, init) => {
  const url = new URL(String(input), location.origin);
  const path = url.searchParams.get("path") ?? "";
  if (url.pathname === "/fs/file") {
    requests.push(`workspace:${path}`);
    return path === "missing.html" ? json({ error: "not found" }, 404) : json({ kind: "text", content: page("workspace") });
  }
  if (url.pathname === "/previews" && (input as any) !== undefined) {
    const method = String(init?.method ?? "GET");
    if (method === "POST") {
      const body = JSON.parse(String(init?.body));
      previewCalls.push(`open:${body.kind}:${body.path}`);
      return previews === "origin"
        ? json({ id: "p1", origin: "http://localhost:9", url: `http://localhost:9/token/${body.path}` })
        : json({ error: "not local", reason: "not_local" }, 409);
    }
  }
  if (/^\/previews\/[^/]+$/.test(url.pathname) && init?.method === "DELETE") {
    previewCalls.push(`close:${url.pathname.split("/").pop()}`);
    return new Response(null, { status: 204 });
  }
  const run = /\/sandbox\/executions\/([^/]+)\/artifact$/.exec(url.pathname);
  if (run) {
    requests.push(`run:${run[1]}:${path}`);
    return json({ kind: "text", content: page(`run ${run[1]}`) });
  }
  const version = /\/sandbox\/candidates\/([^/]+)\/files$/.exec(url.pathname);
  if (version) {
    requests.push(`draft:${version[1]}:${path}`);
    return json({ kind: "text", content: page(`draft ${version[1]}`) });
  }
  return json({ comments: [], records: [] });
};

store.setActiveId(sid);
render(() => <ArtifactCanvas />, document.getElementById("root")!);

const settle = () => new Promise((resolve) => setTimeout(resolve, 250));
const frame = () => document.querySelector<HTMLIFrameElement>(".artifact-canvas iframe.artifact-canvas-frame");
const shown = () => /<h1>([^<]*)<\/h1>/.exec(frame()?.srcdoc ?? "")?.[1];
const check = (ok: unknown, message: string) => { if (!ok) throw new Error(message); return message; };
const clear = () => { while (store.canvasOpen()) store.closeArtifactCanvas(); };
const open = async (run: () => void) => { clear(); await settle(); requests.length = 0; previewCalls.length = 0; run(); await settle(); };
const run = (id: string, path: string) => ({ id, ownerSessionId: sid, tool: "bash", command: "", language: "", scratchDir: "", stdout: "", stderr: "", packages: [], artifacts: [{ path, mimeType: "text/html", sizeBytes: 1 }], status: "completed" as const, timestamp: "" });

(window as any).runChecks = async () => {
  const passed: string[] = [];

  await open(() => store.openArtifactCanvas({ kind: "inline", title: "Probe", html: "<h1>inline</h1><script>1</script>" }));
  passed.push(check(shown() === "inline" && requests.length === 0, "Inline markup renders without reading any file"));
  passed.push(check(frame()?.getAttribute("sandbox") === "allow-scripts allow-forms", "A static preview runs without same-origin access"));
  passed.push(check(/^<!doctype html><meta http-equiv="Content-Security-Policy"/.test(frame()?.srcdoc ?? "") && /connect-src 'none'/.test(frame()!.srcdoc), "The no-network policy comes first in the frame"));

  await open(() => store.openArtifactFile("site/index.html", { sessionId: sid }));
  passed.push(check(shown() === "workspace" && requests.join() === "workspace:site/index.html", "A workspace file is read through the workspace route only"));

  await open(() => store.openArtifactCanvas(fileSubject("site/index.html", { sessionId: sid, executionId: "e1" }, "index.html")));
  passed.push(check(shown() === "run e1" && requests.join() === "run:e1:site/index.html", "A run's file is read through that run only"));

  await open(() => store.openArtifactCanvas(fileSubject("site/index.html", { sessionId: sid, candidateId: "c1", executionId: "e1" }, "index.html")));
  passed.push(check(shown() === "draft c1" && requests.join() === "draft:c1:site/index.html", "A saved version is read through its candidate only"));

  await open(() => store.openArtifactFile("missing.html", { sessionId: sid }));
  passed.push(check(!frame() && !!document.querySelector(".artifact-canvas-error"), "An unreadable file shows an error and no stand-in page"));

  store.setWorkbenchExecutions([run("e7", "out/one.html")]);
  await open(() => store.openArtifactFile("out/one.html"));
  passed.push(check(shown() === "run e7" && requests.join() === "run:e7:out/one.html", "A bare path made by exactly one run opens that run's file"));

  await open(() => store.openArtifactFile("one.html"));
  passed.push(check(requests.join() === "workspace:one.html", "A bare file name that only shares a suffix is not taken for the run's file"));

  store.setWorkbenchExecutions([run("e7", "report.html"), run("e8", "report.html")]);
  store.setNotice(null);
  await open(() => store.openArtifactFile("report.html"));
  passed.push(check(!store.canvasOpen() && store.notices().some((notice) => /more than one run/.test(notice.text)), "A path made by several runs asks which one instead of choosing"));

  // With a preview origin, a page is framed from it and the files it loads come from there.
  previews = "origin";
  await open(() => store.openArtifactFile("site/index.html", { sessionId: sid }));
  passed.push(check(frame()?.getAttribute("src") === "http://localhost:9/token/site/index.html" && !frame()?.getAttribute("srcdoc") && previewCalls.join() === "open:workspace:site/index.html", "A page with files behind it is framed from a preview origin"));
  passed.push(check(frame()?.getAttribute("sandbox") === "allow-scripts allow-same-origin allow-forms allow-popups", "A preview origin frame keeps its own storage and opens windows"));
  await open(() => store.openArtifactCanvas(fileSubject("site/index.html", { sessionId: sid, candidateId: "c1" }, "index.html")));
  passed.push(check(previewCalls.join() === "open:candidate:site/index.html", "A saved version opens its preview through its candidate"));
  await open(() => store.openArtifactCanvas({ kind: "inline", title: "Probe", html: "<h1>inline</h1>" }));
  passed.push(check(previewCalls.length === 0 && shown() === "inline", "Markup from the conversation never gets an origin"));
  await open(() => store.openArtifactFile("site/index.html", { sessionId: sid }));
  previewCalls.length = 0;
  store.closeArtifactCanvas();
  await settle();
  passed.push(check(previewCalls.join() === "close:p1", "Closing the Canvas ends the preview origin"));
  previews = "none";

  // Several subjects share one Canvas as tabs, and each keeps what was done in it.
  await open(() => store.openArtifactFile("site/index.html", { sessionId: sid }));
  store.openArtifactFile("data/table.csv", { sessionId: sid });
  await settle();
  const tabs = () => [...document.querySelectorAll<HTMLElement>(".artifact-canvas-tab-label")];
  passed.push(check(tabs().length === 2 && tabs()[1].getAttribute("aria-selected") === "true", "A second subject opens as a tab in front of the first"));
  tabs()[0].click();
  await settle();
  const seg = (label: string) => [...document.querySelectorAll<HTMLElement>(".artifact-canvas-seg-btn")].find((button) => button.textContent?.trim() === label);
  seg("Code")!.click();
  await settle();
  const note = () => document.querySelector<HTMLTextAreaElement>(".artifact-canvas-feedback textarea")!;
  note().value = "make the heading larger";
  note().dispatchEvent(new Event("input", { bubbles: true }));
  await settle();
  tabs()[1].click();
  await settle();
  passed.push(check(!!document.querySelector(".artifact-canvas-table-preview"), "Another tab shows its own viewer"));
  tabs()[0].click();
  await settle();
  passed.push(check(!!document.querySelector(".artifact-canvas-source") && note().value === "make the heading larger", "Coming back to a tab keeps its view and unsent note"));

  // Each conversation has its own Canvas: leaving hides it, returning finds it as it was.
  store.setActiveId("another-conversation");
  await settle();
  passed.push(check(!store.canvasOpen() && !document.querySelector(".artifact-canvas"), "Another conversation has no Canvas showing"));
  store.setActiveId(sid);
  await settle();
  passed.push(check(!!document.querySelector(".artifact-canvas") && tabs().length === 2 && note().value === "make the heading larger", "Returning to a conversation finds its Canvas as it was"));

  // Escape closes the one in front and leaves the other.
  document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
  await settle();
  passed.push(check(tabs().length === 0 && store.canvasEntries().length === 1 && store.canvasOpen(), "Escape closes only the tab in front"));

  // A document opens on the whole viewport and can still be put beside the conversation.
  await open(() => store.openArtifactFile("report.docx", { sessionId: sid }));
  const panel = () => document.querySelector(".artifact-canvas")!;
  passed.push(check(panel().classList.contains("canvas-focused"), "A document opens on the whole viewport"));
  const layout = document.querySelector<HTMLElement>('.artifact-canvas-btn[aria-label="Show beside conversation"]');
  passed.push(check(!!layout, "A document offers to sit beside the conversation"));
  layout!.click();
  await settle();
  passed.push(check(panel().classList.contains("canvas-split"), "A document can be shown beside the conversation"));

  // A viewer that fails is contained: the frame stays, and offers another go.
  const original = VIEWERS.code;
  let broken = true;
  VIEWERS.code = ((props: any) => { if (broken) throw new Error("boom"); return original(props); }) as typeof original;
  try {
    await open(() => store.openArtifactFile("src/main.rs", { sessionId: sid }));
    passed.push(check(/This view stopped working: boom/.test(document.querySelector(".artifact-canvas-error")?.textContent ?? "") && !!document.querySelector(".artifact-canvas-header"), "A failing viewer shows why and leaves the frame usable"));
    broken = false;
    [...document.querySelectorAll<HTMLElement>(".artifact-canvas-error button")].find((button) => button.textContent === "Try again")!.click();
    await settle();
    passed.push(check(!!document.querySelector(".artifact-canvas-source"), "Trying again draws the viewer"));
  } finally {
    VIEWERS.code = original;
  }

  clear();
  return passed;
};
