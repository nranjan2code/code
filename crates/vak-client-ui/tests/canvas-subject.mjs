import assert from "node:assert/strict";
import {
  displayType,
  fileSubject,
  inlineTitle,
  isCanvasSubject,
  sameSubject,
  liveServerOrigin,
  matchExecutionArtifact,
  previewSource,
  runOrigin,
  subjectCandidateId,
  subjectExecutionId,
  subjectKey,
  subjectPath,
  subjectSessionId,
} from "../src/canvasSubject.ts";

// The origin picks the route: a saved version is never read as a workspace file.
const draft = fileSubject("site/index.html", { sessionId: "s1", candidateId: "c1", executionId: "e1", resultId: "r1" }, "index.html");
assert.equal(draft.kind, "draft_file");
assert.equal(subjectCandidateId(draft), "c1");
assert.equal(subjectExecutionId(draft), "e1");
assert.equal(draft.resultId, "r1");

const run = fileSubject("out/report.csv", { sessionId: "s1", executionId: "e2" }, "report.csv");
assert.equal(run.kind, "execution_artifact");
assert.equal(subjectCandidateId(run), undefined);

const plain = fileSubject("notes.md", { sessionId: "s1" }, "notes.md");
assert.equal(plain.kind, "file");
assert.equal(subjectSessionId(plain), "s1");

// A run without its conversation cannot be read as a run, so it is a plain file.
assert.equal(fileSubject("a.txt", runOrigin({ executionId: "e9" }), "a.txt").kind, "file");
assert.equal(fileSubject("a.txt", runOrigin({ sessionId: "s", executionId: "e9" }), "a.txt").kind, "execution_artifact");

// Inline markup has no file and no conversation of its own.
const inline = { kind: "inline", title: "HTML Preview", html: "<p>x</p>" };
assert.equal(subjectPath(inline), "");
assert.equal(subjectSessionId(inline), undefined);
assert.equal(displayType(inline), "html");
assert.equal(subjectPath({ ...inline, basePath: "site/a.html" }), "site/a.html");

// How a subject is drawn comes from its kind, then its extension.
const type = (path) => displayType(fileSubject(path, {}, path));
assert.deepEqual(
  ["a.pdf", "a.CSV", "a.tsv", "a.docx", "a.PNG", "a.svg", "a.html", "a.xhtml", "a.rs", "Makefile"].map(type),
  ["pdf", "table", "table", "office", "image", "image", "html", "html", "code", "code"],
);
assert.equal(displayType({ kind: "live_server", title: "app", serverName: "web", sessionId: "s" }), "server");


// A bare reference matches a run only by identical path.
const runs = [
  { id: "e1", artifacts: [{ path: "report.html" }, { path: "data/a.csv" }] },
  { id: "e2", artifacts: [{ path: "report.html" }] },
  { id: "e3", artifacts: [{ path: "other/chart.png" }] },
];
assert.deepEqual(matchExecutionArtifact("data/a.csv", runs), { kind: "one", executionId: "e1" });
assert.deepEqual(matchExecutionArtifact("./data/a.csv", runs), { kind: "one", executionId: "e1" });
assert.deepEqual(matchExecutionArtifact("report.html", runs), { kind: "many", count: 2 });
// A shared file name or suffix is not a match: it would open another run's file.
assert.deepEqual(matchExecutionArtifact("chart.png", runs), { kind: "none" });
assert.deepEqual(matchExecutionArtifact("a.csv", runs), { kind: "none" });
assert.deepEqual(matchExecutionArtifact("x/data/a.csv", runs), { kind: "none" });

// Which route serves a page's files follows its identity; markup and servers have no files to serve.
// A page in the folder is read from its conversation's folder, an Agent's own.
assert.deepEqual(previewSource(plain), { kind: "workspace", path: "notes.md", session_id: "s1" });
assert.deepEqual(previewSource(run), { kind: "execution", session_id: "s1", execution_id: "e2", path: "out/report.csv" });
assert.deepEqual(previewSource(draft), { kind: "candidate", session_id: "s1", candidate_id: "c1", path: "site/index.html" });
assert.equal(previewSource(inline), null);
assert.equal(previewSource({ kind: "live_server", title: "app", serverName: "web", sessionId: "s" }), null);

// A dev server is framed from the other loopback name than the app's, and never from anywhere else.
assert.equal(liveServerOrigin("127.0.0.1", 5173), "http://localhost:5173");
assert.equal(liveServerOrigin("[::1]", 5173), "http://localhost:5173");
assert.equal(liveServerOrigin("localhost", 5173), "http://127.0.0.1:5173");
assert.equal(liveServerOrigin("LOCALHOST", 5173), "http://127.0.0.1:5173");
assert.equal(liveServerOrigin("vak.example.com", 5173), null);
assert.equal(liveServerOrigin("192.168.1.20", 5173), null);

// A routine is a subject with no file, no conversation and nothing to serve.
const routine = { kind: "automation", title: "Morning summary", taskId: "t1" };
assert.equal(displayType(routine), "automation");
assert.equal(subjectPath(routine), "");
assert.equal(subjectSessionId(routine), undefined);
assert.equal(previewSource(routine), null);
assert.equal(subjectKey(routine), "task:t1");
assert.notEqual(subjectKey(routine), subjectKey({ ...routine, taskId: "t2" }));

// Reopening the same thing is the same subject; another place in it is not.
const cited = { ...plain, anchor: "p:4" };
assert.ok(sameSubject(plain, { ...plain }));
assert.ok(!sameSubject(plain, cited));

// What another surface wrote is checked before it is drawn.
assert.ok(isCanvasSubject(plain) && isCanvasSubject(run) && isCanvasSubject(draft) && isCanvasSubject(inline) && isCanvasSubject(routine));
for (const hostile of [null, "x", { kind: "file" }, { kind: "file", title: "a", path: 3 }, { kind: "unknown", title: "a" }, { kind: "draft_file", title: "a", path: "p", sessionId: "s" }]) {
  assert.ok(!isCanvasSubject(hostile), JSON.stringify(hostile));
}

// Markup is named for what it is, not "HTML Preview".
assert.equal(inlineTitle("<!doctype html><title>Little &amp; counter</title><p>x</p>"), "Little & counter");
assert.equal(inlineTitle("<p>x</p>", "Given name"), "Given name");
assert.equal(inlineTitle("<p>x</p>"), "Web page");
assert.equal(inlineTitle("<svg viewBox='0 0 1 1'></svg>"), "Picture");

console.log("canvas subject ok");
