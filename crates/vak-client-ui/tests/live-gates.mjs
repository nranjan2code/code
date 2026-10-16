import assert from "node:assert/strict";
import { holdsOnlyLiveGates, keepLiveGates } from "../src/liveGates.ts";

const user = { kind: "user", text: "touch a file" };
const approval = { kind: "approval", id: "a1", tool: "bash", argsJson: "{}", reason: "", resolved: null };
const question = { kind: "question", id: "q1", label: "w", question: "Which?", options: [], resolved: null };

// A gate restored before the transcript arrived survives the transcript.
assert.deepEqual(keepLiveGates([approval, question], [user]), [user, approval, question]);

// An answered gate is history, not something to carry.
assert.deepEqual(keepLiveGates([{ ...approval, resolved: "allowed" }], [user]), [user]);

// A gate the next list already holds is not doubled.
assert.deepEqual(keepLiveGates([approval], [user, approval]), [user, approval]);

// Nothing else is carried.
assert.deepEqual(keepLiveGates([{ kind: "system", text: "note" }], [user]), [user]);

// A running conversation whose list holds only restored gates still needs
// its history painted.
assert.equal(holdsOnlyLiveGates([]), true);
assert.equal(holdsOnlyLiveGates([approval, question]), true);
assert.equal(holdsOnlyLiveGates([approval, user]), false);

console.log("live-gates ok");
