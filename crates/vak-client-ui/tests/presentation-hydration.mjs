import assert from "node:assert/strict";
import { ConversationHydration, applyPresentationDelta, mergePresentationSnapshot } from "../src/presentationHydration.ts";

const settled = {
  schema_version: 2,
  session_id: "session-1",
  cursor: "durable:1",
  diagnostics: [],
  items: [
    {
      id: "weather-card",
      timestamp: "2026-09-24T00:00:00Z",
      turn_id: "weather-turn",
      role: "tool",
      kind: "card",
      status: "succeeded",
      content: { type: "structured", data: {} },
      provenance: { source: "session_projection" },
      actions: [],
      fallback_text: "31 C",
    },
  ],
};

const nextRunLive = {
  schema_version: 2,
  session_id: "session-1",
  cursor: "live:42",
  diagnostics: [],
  items: [
    {
      id: "live-assistant-42",
      timestamp: "2026-09-24T00:01:00Z",
      turn_id: "live-turn-42",
      role: "assistant",
      kind: "message",
      status: "streaming",
      content: { type: "document", document: {} },
      provenance: { source: "live_event" },
      actions: [],
      fallback_text: "Fetching news…",
    },
  ],
};

const merged = mergePresentationSnapshot(settled, nextRunLive);
assert.deepEqual(
  merged.items.map((item) => item.id),
  ["weather-card", "live-assistant-42"],
  "a new run's live frame must not dehydrate a settled card",
);

const conflictingLive = {
  ...nextRunLive,
  items: [
    { ...settled.items[0], kind: "message", fallback_text: "plain text" },
    nextRunLive.items[0],
  ],
};
assert.equal(
  mergePresentationSnapshot(settled, conflictingLive).items[0].kind,
  "card",
  "a live re-projection must not turn a settled card into prose",
);

const durable = { ...nextRunLive, cursor: "durable:2", items: [] };
assert.deepEqual(
  mergePresentationSnapshot(merged, durable).items,
  [],
  "a durable snapshot remains an authoritative full replacement",
);

let streaming = { ...nextRunLive, items: [{ ...nextRunLive.items[0], fallback_text: "", content: { type: "document", document: { source_markdown: "", blocks: [] } } }] };
for (const delta of ["Literal ", "[recovery] ", "completed"]) streaming = applyPresentationDelta(streaming, { type: "text_delta", item_id: "live-assistant-42", delta });
assert.equal(streaming.items[0].fallback_text, "Literal [recovery] completed");
assert.equal(streaming.items[0].content.document.source_markdown, "Literal [recovery] completed");
// An older server's accumulated text still replaces, rather than duplicates.
streaming = applyPresentationDelta(streaming, { type: "text_delta", item_id: "live-assistant-42", delta: "!", text: "Exact snapshot!" });
assert.equal(streaming.items[0].fallback_text, "Exact snapshot!");
assert.equal(mergePresentationSnapshot(streaming, durable).items.length, 0);
console.log("presentation delta hydration ok");

// The same durable read first arrives while the stream owns the view, then
// after settlement. Deferred reads must not poison the applied-state cache.
const coordinator = new ConversationHydration();
let applied = [];
const apply = (value) => () => applied.push(value);
coordinator.commit("s", coordinator.begin("s"), "saved-cards", false, apply("running"));
coordinator.commit("s", coordinator.begin("s"), "saved-cards", true, apply("settled"));
coordinator.commit("s", coordinator.begin("s"), "saved-cards", true, apply("duplicate"));
assert.deepEqual(applied, ["settled"]);
// A slow pre-admission response cannot overwrite a newly sent message.
const beforeAdmission = coordinator.begin("s");
coordinator.invalidate("s");
assert.equal(coordinator.commit("s", beforeAdmission, "old", true, apply("stale")), false);
const slow = coordinator.begin("s");
const fast = coordinator.begin("s");
coordinator.commit("s", fast, "new", true, apply("new"));
coordinator.commit("s", slow, "old", true, apply("old"));
assert.deepEqual(applied, ["settled", "new"]);
// An optimistic mutation requires applying the same server content again.
coordinator.invalidate("s");
coordinator.commit("s", coordinator.begin("s"), "new", true, apply("reconciled"));
assert.deepEqual(applied, ["settled", "new", "reconciled"]);
// Exceptions never acknowledge a failed view commit.
const broken = coordinator.begin("other");
assert.throws(() => coordinator.commit("other", broken, "data", true, () => { throw Error("apply failed"); }));
assert.equal(coordinator.commit("other", coordinator.begin("other"), "data", true, apply("recovered")), true);

const beforeReplacement = {...settled,items:[{...settled.items[0],id:'anchor',provenance:{presentation_id:'pres-anchor'}},{...settled.items[0],id:'beside',placement:{beside:'pres-anchor'}}]};
const afterReplacement = applyPresentationDelta(beforeReplacement,{type:'item_started',item:{...settled.items[0],id:'replacement',placement:{replaces:'pres-anchor'}}});
assert.deepEqual(afterReplacement.items.map(item=>item.id),['beside','replacement']);
assert.equal(afterReplacement.items[0].placement.beside,'pres-anchor');
console.log('live replacement removes only its addressed target and retains original anchors');

const durableCard = {...settled.items[0],id:'shared-card',provenance:{source:'emit_table_card',presentation_id:'pres-1'}};
const durableCards = {...settled,items:[durableCard]};
const lateCard = {...durableCard,turn_id:'directive-id',provenance:{...durableCard.provenance,source:'live_event'}};
assert.equal(applyPresentationDelta(durableCards,{type:'item_started',item:lateCard}),durableCards);
console.log('late live cards never replace their authoritative settled item');

const liveNamedSettled = { ...settled, items: [{ ...settled.items[0], turn_id: "live-sounding-recorded-id" }] };
assert.equal(mergePresentationSnapshot(liveNamedSettled, nextRunLive).items[0], liveNamedSettled.items[0], "recorded authority comes from provenance, never an ID prefix");
