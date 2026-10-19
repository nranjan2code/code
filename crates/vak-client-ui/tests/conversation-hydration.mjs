import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import ts from "typescript";
import { ConversationHydration } from "../src/presentationHydration.ts";

// Execute App's actual hydrate function, including its API wiring and commit
// guards. The browser fixture separately exercises Solid and the real ChatPane.
const source = fs.readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
const start = source.indexOf("async function hydrate(id:");
const end = source.indexOf("\nconst resyncingSessions", start);
assert(start >= 0 && end > start);
const actual = ts.transpile(source.slice(start, end), { target: ts.ScriptTarget.ES2022 });
const coordinator = new ConversationHydration();
let running = true;
let identity = false;
let cards = [];
let reads = [];
let notes = [];
const snapshot = (label = "answer") => ({
  session_id: "s", running: false,
  transcript: { messages: [{ label }], entries: [{ entry_id: "user" }], usage: {}, set_aside: [] },
  presentation: { session_id: "s", items: [{ id: label }] },
});
const context = {
  conversationHydration: coordinator,
  readableSnapshot: () => new Promise((resolve, reject) => reads.push({ resolve, reject })),
  api: { sandboxExecutions: async () => ({ events: [] }) },
  isRunning: () => running, holdsOnlyLiveGates: () => false,
  itemsOf: () => [{ kind: "user", entryId: identity ? "user" : undefined }],
  batch: apply => apply(), hydrateFromTranscript: () => { identity = true; },
  hydrateSetAside() {}, hydrateFromPresentation: (_id, value) => { cards = value.items; },
  setUsageFor() {}, activeId: () => "s", setWorkbenchLoadError() {},
  hydrateWorkbenchExecutions() {}, setHydratingId() {},
  appendSystem: (_id, message) => notes.push(message), Error,
};
vm.createContext(context);
vm.runInContext(actual, context);
let pending = context.hydrate("s", true);
reads.shift().resolve(snapshot());
await pending;
assert.equal(identity, false, "live ownership defers identity");
running = false;
pending = context.hydrate("s", true);
reads.shift().resolve(snapshot());
await pending;
assert.equal(identity, true, "identical settled read commits identity");
assert.equal(cards[0].id, "answer", "the same commit includes the saved card");

const older = context.hydrate("s", true);
const oldRead = reads.shift();
const newer = context.hydrate("s", true);
reads.shift().resolve(snapshot("newer"));
await newer;
oldRead.resolve(snapshot("older"));
await older;
assert.equal(cards[0].id, "newer", "slower older response cannot roll the view back");

const staleFailure = context.hydrate("s", true);
const failure = reads.shift();
coordinator.invalidate("s");
failure.reject(new Error("outdated failure"));
await staleFailure;
assert.deepEqual(notes, [], "invalidated reads cannot publish stale failure notes");
console.log("actual conversation hydration: deferred identity, atomic card commit, read ordering and stale failure checks passed");

// Invalid identity/output envelopes fail before any view mutation.
const readStart = source.indexOf("async function readableSnapshot(");
const readEnd = source.indexOf("\n/** What `hydrate`", readStart);
let returned;
const validation = { api: { conversationSnapshot: async () => returned }, Error };
vm.createContext(validation);
vm.runInContext(ts.transpile(source.slice(readStart, readEnd), { target: ts.ScriptTarget.ES2022 }), validation);
const valid = { session_id: "s", running: false, cursor: "revision", transcript: { messages: [], entries: [] }, presentation: { schema_version: 2, session_id: "s", cursor: "revision", items: [] } };
returned = valid;
await validation.readableSnapshot("s");
for (const invalid of [
  { ...valid, session_id: "other" },
  { ...valid, cursor: "different-revision" },
  { ...valid, transcript: { messages: [{}], entries: [] } },
  { ...valid, transcript: { messages: [{}], entries: [{}] } },
  { ...valid, presentation: { ...valid.presentation, schema_version: 999 } },
]) {
  returned = invalid;
  await assert.rejects(validation.readableSnapshot("s"), /snapshot is invalid/);
}
console.log("conversation snapshot validation: mismatched revisions, missing identity and unsupported schemas refused");
