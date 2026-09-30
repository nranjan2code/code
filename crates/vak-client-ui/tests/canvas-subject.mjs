import assert from "node:assert/strict";
import {
  displayType,
  fileSubject,
  matchExecutionArtifact,
  runOrigin,
  subjectCandidateId,
  subjectExecutionId,
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

console.log("canvas subject ok");
