import assert from "node:assert/strict";
import { hasSettledProjection, serverTurnFor } from "../src/turnPairing.ts";

const user = { role: "user", turn_id: "turn-1", kind: "message", content: { type: "document" }, provenance: { entry_id: "user-1" } };
const artifact = { role: "tool", turn_id: "turn-1", kind: "artifact", content: { type: "artifact" } };
const card = { role: "tool", turn_id: "turn-1", kind: "card", content: { type: "structured" } };
const failed = { role: "assistant", turn_id: "turn-1", kind: "error", content: { type: "error" } };

assert.equal(serverTurnFor("user-1", [user, artifact, card, failed]), "turn-1");
assert.equal(hasSettledProjection([user, artifact, card, failed]), true);
assert.equal(hasSettledProjection([user, failed]), true);
assert.equal(hasSettledProjection([user, { role: "tool", kind: "progress", content: { type: "information" } }]), false);

// A paged transcript's position differs from the server's canonical turn.
const { turnPlanLabel } = await import('../src/turnPairing.ts');
const timeline = { items: [user], answers: [
  { turn_id: 'turn-9', sections: [{ name: 'Unrelated work' }] },
  { turn_id: 'turn-1', sections: [{ name: ' Noida weather ' }, { name: 'Air quality' }] },
] };
assert.equal(turnPlanLabel('user-1', timeline), 'Noida weather · Air quality');
assert.equal(turnPlanLabel('not-loaded', timeline), '');
assert.equal(turnPlanLabel(undefined, timeline), '');
assert.equal(turnPlanLabel('user-1', { items: [user] }), '');
const { latestWorkLabel } = await import('../src/turnPairing.ts');
assert.equal(latestWorkLabel(timeline), 'Noida weather · Air quality');
assert.equal(latestWorkLabel({...timeline, items: [...timeline.items, {role:'user',turn_id:'turn-10',fallback_text:'New request without a plan'}]}), 'New request without a plan');
assert.equal(latestWorkLabel(undefined), '');
