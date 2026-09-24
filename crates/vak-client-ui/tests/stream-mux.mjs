import assert from "node:assert/strict";
import { formatCursor, parseCursor, parseFrame, sameInterest, streamPath, unionInterest, wants } from "../src/streamMux.ts";

// Tabs' interests union into one sorted set, so equal sets compare equal
// regardless of which tab registered first.
const union = unionInterest([
  { sessions: ["b", "a"], host: true, config: false },
  { sessions: ["a", "c"], host: false, config: true },
]);
assert.deepEqual(union, { sessions: ["a", "b", "c"], host: true, config: true });
assert.ok(sameInterest(union, unionInterest([{ sessions: ["c", "b", "a"], host: true, config: true }])));

// The cursor is the agent frame's SSE id; it round-trips and ignores junk.
const cursor = parseCursor("019a-x:17,bad,:4,019a-y:3,019a-z:NaN");
assert.deepEqual([...cursor], [["019a-x", 17], ["019a-y", 3]]);
assert.equal(formatCursor(cursor), "019a-x:17,019a-y:3");

// A reopened stream resumes only the sessions it still follows.
const path = streamPath({ sessions: ["019a-x"], host: true, config: false }, cursor);
const query = new URLSearchParams(path.split("?")[1]);
assert.deepEqual(query.getAll("session"), ["019a-x"]);
assert.equal(query.get("host"), "1");
assert.equal(query.get("config"), null);
assert.equal(query.get("cursor"), "019a-x:17");

// Frames are routed by the session they name, never broadcast.
const agent = parseFrame("agent", JSON.stringify({ session: "a", event: { TurnStart: {} } }));
assert.deepEqual(agent, { kind: "agent", session: "a", event: { TurnStart: {} } });
assert.ok(wants({ sessions: ["a"], host: false, config: false }, agent));
assert.ok(!wants({ sessions: ["b"], host: true, config: true }, agent));
const host = parseFrame("host", JSON.stringify({ ready: true }));
assert.ok(wants({ sessions: [], host: true, config: false }, host));
assert.ok(!wants({ sessions: ["a"], host: false, config: false }, host));
assert.deepEqual(parseFrame("side", JSON.stringify({ session: "a", lagged: true })), { kind: "side", session: "a", event: null });
assert.equal(parseFrame("agent", "not json"), null);
assert.equal(parseFrame("agent", JSON.stringify({ event: {} })), null);

console.log("stream-mux: ok");
