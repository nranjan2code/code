import assert from "node:assert/strict";
import { pendingVersions } from "../src/candidateVersions.ts";

const candidate = (id, execution = "exec-1", extra = {}) => ({ kind: "Candidate", record: { execution_id: execution, candidate: { candidate_id: id }, ...extra } });
const promotion = (id) => ({ kind: "Promotion", record: { candidate_id: id } });
const undo = (id) => ({ kind: "PromotionUndo", record: { candidate_id: id } });
const ids = (records) => pendingVersions(records, "exec-1").map((record) => record.candidate.candidate_id);

// A full draft and a version keeping some of it are alternatives.
assert.deepEqual(ids([candidate("full"), candidate("kept", "exec-1", { parent_candidate_id: "full", narrowed: { path: "b.xlsx", keep: ["1"] } })]), ["full", "kept"]);
// Accepting the narrower version settles the full draft too.
const accepted = [candidate("full"), candidate("kept"), promotion("kept")];
assert.deepEqual(ids(accepted), []);
// Reviewing again after that starts a new round; its version is pending.
assert.deepEqual(ids([...accepted, candidate("again")]), ["again"]);
// Undoing the acceptance reopens the round it closed.
assert.deepEqual(ids([...accepted, undo("kept")]), ["full"]);
// Another execution's promotion settles nothing here.
assert.deepEqual(ids([candidate("full"), candidate("other", "exec-2"), promotion("other")]), ["full"]);
console.log("candidate-versions: ok");
