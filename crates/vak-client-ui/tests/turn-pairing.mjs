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
const { turnDisplayLabel } = await import('../src/turnPairing.ts');
const timeline = { items: [user] };
assert.equal(turnDisplayLabel('not-loaded', timeline), '');
assert.equal(turnDisplayLabel(undefined, timeline), '');
assert.equal(turnDisplayLabel('user-1', timeline), '');
const { latestWorkLabel } = await import('../src/turnPairing.ts');
assert.equal(latestWorkLabel({...timeline, items: [...timeline.items, {role:'user',turn_id:'turn-10',fallback_text:'New untitled request'}]}), 'New untitled request');
assert.equal(latestWorkLabel(undefined), '');

const heading = { role: 'assistant', turn_id: 'turn-1', kind: 'outcome', content: { type: 'document', document: { blocks: [
  { type: 'heading', content: [{type:'text',text:'Observed '},{type:'strong',content:[{type:'text',text:'changes'}]}] }
] } } };
const titledCard = { role: 'tool', turn_id: 'turn-1', kind: 'card', content: { type: 'structured', output: { payload: { title: 'Recorded result' } } } };
assert.equal(turnDisplayLabel('user-1', {items: [user, titledCard, heading]}), 'Observed changes');
assert.equal(turnDisplayLabel('user-1', {items: [user, titledCard]}), 'Recorded result');
assert.equal(turnDisplayLabel('user-1', {items: [user, {...titledCard,turn_id:'turn-2'}]}), '');
assert.equal(turnDisplayLabel('user-1', {items: [{...user,fallback_text:'Long raw request'}, {...titledCard,provenance:{source:'derived'}}]}), '');

assert.equal(latestWorkLabel({items: [user, titledCard, heading]}), 'Observed changes');
