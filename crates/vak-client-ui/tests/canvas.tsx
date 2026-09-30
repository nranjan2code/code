import { render } from "solid-js/web";
import ArtifactCanvas from "../src/components/ArtifactCanvas";
import * as store from "../src/store";
import { fileSubject } from "../src/canvasSubject";
import "../src/styles.css";

// Every route the Canvas can read a file through answers with its own marker,
// and each request is logged, so a check can tell which route served a subject.
const sid = "canvas-fixture";
const requests: string[] = [];
const page = (source: string) => `<!doctype html><html><head><title>${source}</title></head><body><h1>${source}</h1></body></html>`;
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
window.fetch = async (input) => {
  const url = new URL(String(input), location.origin);
  const path = url.searchParams.get("path") ?? "";
  if (url.pathname === "/fs/file") {
    requests.push(`workspace:${path}`);
    return path === "missing.html" ? json({ error: "not found" }, 404) : json({ kind: "text", content: page("workspace") });
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
const open = async (run: () => void) => { store.closeArtifactCanvas(); requests.length = 0; run(); await settle(); };
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

  store.closeArtifactCanvas();
  return passed;
};
