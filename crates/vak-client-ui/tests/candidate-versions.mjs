import assert from "node:assert/strict";
import { acceptanceSummary, pendingVersions, undoablePromotion } from "../src/candidateVersions.ts";

const candidate = (id, execution = "exec-1", extra = {}) => ({ kind: "Candidate", record: { execution_id: execution, candidate: { candidate_id: id }, ...extra } });
const promotion = (id, receipt = {}) => ({ kind: "Promotion", record: { candidate_id: id, receipt } });
const undo = (id) => ({ kind: "PromotionUndo", record: { candidate_id: id } });
const ids = (records) => pendingVersions(records, "exec-1").map((record) => record.candidate.candidate_id);
const undoable = (records, execution = "exec-1") => undoablePromotion(records, execution)?.candidate_id ?? null;

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

// Undo is offered from the records alone: nothing accepted, nothing to undo.
assert.equal(undoable([candidate("full")]), null);
// The acceptance of this execution's result is undoable…
assert.equal(undoable(accepted), "kept");
// …until it is undone, when its round reopens and Undo goes away together.
assert.equal(undoable([...accepted, undo("kept")]), null);
assert.deepEqual(ids([...accepted, undo("kept")]), ["full"]);
// A later round's acceptance is the one offered; undoing it offers the one
// before, as the round rule reopens the later round and not the earlier one.
// An undone version itself is not pending again: the server accepts a
// candidate once.
const twice = [...accepted, candidate("again"), candidate("again-kept"), promotion("again-kept")];
assert.equal(undoable(twice), "again-kept");
assert.equal(undoable([...twice, undo("again-kept")]), "kept");
assert.deepEqual(ids([...twice, undo("again-kept")]), ["again"]);
assert.equal(undoable([...twice, undo("again-kept"), undo("kept")]), null);
assert.deepEqual(ids([...twice, undo("again-kept"), undo("kept")]), ["full", "again"]);
// Another execution's acceptance is never offered here, even when it is the
// session's latest, and this one's stays offered.
const mixed = [...accepted, candidate("other", "exec-2"), promotion("other")];
assert.equal(undoable(mixed), "kept");
assert.equal(undoable(mixed, "exec-2"), "other");
assert.equal(undoable([candidate("other", "exec-2"), promotion("other")]), null);
// An agent revision keeps its execution, so accepting it is undoable here.
assert.equal(undoable([candidate("full"), candidate("rev", "exec-1", { parent_candidate_id: "full" }), promotion("rev")]), "rev");

// The message beside Undo is the one shown right after Accept.
assert.equal(acceptanceSummary(promotion("kept", { verification: [{}, {}] }).record), "Applied 2 change(s).");
assert.equal(acceptanceSummary(promotion("kept", { verification: [{}], integration: { workspace_state_status: "observed", target_checks_status: "passed" } }).record), "Applied 1 change(s). Exact workspace state verified; target checks passed.");
console.log("candidate-versions: ok");
