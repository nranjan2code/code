import assert from "node:assert/strict";
import { actionText, automationState, canSendAgain, deliveryStatusLabel, sentAgainWords, nextRunWords, runStatusLabel } from "../src/automationWords.ts";



assert.equal(runStatusLabel("completed"), "Finished");
assert.equal(runStatusLabel("failed"), "Couldn’t finish");
assert.equal(runStatusLabel("abandoned"), "Interrupted");
assert.equal(runStatusLabel("skipped"), "Didn’t run");
assert.equal(deliveryStatusLabel("inbox"), "Saved in Inbox");
assert.equal(deliveryStatusLabel("something new"), "something new");
assert.equal(deliveryStatusLabel("unknown"), "Not sure it was sent");
assert.equal(canSendAgain({ delivery_effect: "eff_1" }), true);
assert.equal(canSendAgain({ delivery_effect: null }), false);
assert.equal(canSendAgain({}), false);
assert.equal(sentAgainWords({ new: true, status: "sent" }), "Sent again as a new message");
assert.equal(sentAgainWords({ new: false, status: "unknown" }), "Tried again; it hasn’t arrived yet");

// A paused automation is paused whatever it is doing; otherwise it is working while a run is open.
assert.equal(automationState({ enabled: false, running: true }), "paused");
assert.equal(automationState({ enabled: true, running: true }), "running");
assert.equal(automationState({ enabled: true, running: false }), "scheduled");

assert.equal(actionText({ action: { kind: "prompt", text: "summarise" } }), "summarise");
assert.equal(actionText({ action: { kind: "script", command: "df -h" } }), "df -h");
// The next run is an instant, shown with the reader's zone named.
assert.match(nextRunWords("2026-10-03T09:00:00Z"), /\d/);
assert.ok(/[A-Z]{2,}|GMT|UTC/.test(nextRunWords("2026-10-03T09:00:00Z")), nextRunWords("2026-10-03T09:00:00Z"));
assert.equal(nextRunWords(null), "Not scheduled");
assert.equal(nextRunWords("not a time"), "Not scheduled");

console.log("automation words ok");
