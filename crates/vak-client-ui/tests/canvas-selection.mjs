import assert from "node:assert/strict";
import { changeRequest, commentPlace, sameSelection, selectionForPrompt, selectionInvalid, selectionLabel, selectionOfComment } from "../src/canvasSelection.ts";
import { changedFiles, draftActivity, newerVersions, versionNumber, versionsOf } from "../src/draftVersions.ts";

// A selection says where in everyday words, and sends only what it can name.
assert.equal(selectionLabel({ kind: "lines", start: 4 }), "Line 4");
assert.equal(selectionLabel({ kind: "lines", start: 4, end: 4 }), "Line 4");
assert.equal(selectionLabel({ kind: "lines", start: 4, end: 9 }), "Lines 4–9");
assert.equal(selectionLabel({ kind: "anchor", anchor: "Budget!B4" }), "Selected place");
assert.equal(selectionLabel({ kind: "anchor", anchor: "Budget!B4" }, true), "Selected place · Budget!B4");
assert.deepEqual(commentPlace(null), {});
assert.deepEqual(commentPlace({ kind: "lines", start: 4, end: 9 }), { lineStart: 4, lineEnd: 9 });
assert.deepEqual(commentPlace({ kind: "lines", start: 4 }), { lineStart: 4, lineEnd: undefined });
assert.deepEqual(commentPlace({ kind: "anchor", anchor: "p:1A2B" }), { anchor: "p:1A2B" });
assert.equal(selectionInvalid({ kind: "lines", start: 5, end: 3 }), true);
assert.equal(selectionInvalid({ kind: "lines", start: 0 }), true);
assert.equal(selectionInvalid({ kind: "lines", start: 5, end: 5 }), false);
assert.equal(selectionInvalid({ kind: "anchor", anchor: "x" }), false);
assert.equal(selectionInvalid(null), false);

// A saved comment puts the reader back where it was written; one with no place is about the whole file.
assert.deepEqual(selectionOfComment({ line_start: 3, line_end: 5 }), { kind: "lines", start: 3, end: 5 });
assert.deepEqual(selectionOfComment({ line_start: 3, line_end: 3 }), { kind: "lines", start: 3, end: undefined });
assert.deepEqual(selectionOfComment({ anchor: "Budget!B4", line_start: 3 }), { kind: "anchor", anchor: "Budget!B4" });
assert.equal(selectionOfComment({}), null);
assert.equal(sameSelection({ kind: "lines", start: 3 }, { kind: "lines", start: 3, end: 3 }), true);
assert.equal(sameSelection({ kind: "lines", start: 3 }, { kind: "anchor", anchor: "3" }), false);
assert.equal(sameSelection(null, null), true);
assert.equal(selectionForPrompt(null), "");
assert.match(selectionForPrompt({ kind: "lines", start: 2, end: 4 }), /lines 2–4/);
assert.match(selectionForPrompt({ kind: "anchor", anchor: "Budget!B4" }), /Budget!B4/);

// Versions of one result, in the order they were saved; other results' are not among them.
const candidate = (id, execution, files = [], updated = "2026-09-30T10:00:00Z") => ({ kind: "Candidate", record: { execution_id: execution, updated_at: updated, candidate: { candidate_id: id, files } } });
const records = [
  candidate("c1", "e1", [{ path: "a.html", base_hash: "h" }, { path: "b.html" }, { path: "gone.html", operation: "Delete" }], "2026-09-30T10:00:00Z"),
  candidate("x1", "other"),
  candidate("c2", "e1", [], "2026-09-30T11:00:00Z"),
  { kind: "CandidateRevision", record: { parent_candidate_id: "c1", status: "Completed", updated_at: "2026-09-30T10:30:00Z" } },
  candidate("c3", "e1", [], "2026-09-30T12:00:00Z"),
];
const versions = versionsOf(records, "e1");
assert.deepEqual(versions.map((v) => v.candidate.candidate_id), ["c1", "c2", "c3"]);
assert.deepEqual(versionsOf(records, undefined), []);
assert.equal(versionNumber(versions, "c2"), 2);
assert.equal(versionNumber(versions, "nope"), null);
assert.deepEqual(newerVersions(versions, "c1").map((v) => v.candidate.candidate_id), ["c2", "c3"]);
assert.deepEqual(newerVersions(versions, "c3"), []);
assert.deepEqual(newerVersions(versions, "nope"), []);
assert.deepEqual(changedFiles(versions[0]), [
  { path: "a.html", change: "changed" },
  { path: "b.html", change: "new" },
  { path: "gone.html", change: "removed" },
]);

// Activity is what the records and comments show, newest first, and nothing else.
const activity = draftActivity(records, versions, [{ actor_id: "operator", created_at: "2026-09-30T10:45:00Z", text: "x" }, { actor_id: "u2", actor_name: "Asha", created_at: "2026-09-30T13:00:00Z", text: "y" }, { actor_id: "u3", text: "no time" }]);
assert.deepEqual(activity.map((item) => item.text), ["Asha commented", "Version 3 saved", "Version 2 saved", "You commented", "The Agent finished revising version 1", "Version 1 saved"]);

// A request reads as its sender would write it: their own message in the conversation.
assert.equal(changeRequest("site/index.html", "make it green", { kind: "lines", start: 3, end: 5 }), "Change site/index.html (lines 3–5): make it green");
assert.equal(changeRequest("report.docx", "fix the date", { kind: "anchor", anchor: "p:4" }), "Change report.docx (at p:4): fix the date");
assert.equal(changeRequest("a.csv", "sort it", null), "Change a.csv: sort it");

console.log("canvas selection ok");
