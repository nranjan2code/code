import assert from "node:assert/strict";
import { mergePresentationSnapshot } from "../src/presentationHydration.ts";

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
