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
