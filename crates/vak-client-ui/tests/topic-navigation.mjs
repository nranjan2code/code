import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import ts from "typescript";

const source = fs.readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
const start = source.indexOf("async function focusTopicTurn(");
const end = source.indexOf("\nexport async function openTopic", start);
assert(start >= 0 && end > start);
const actual = ts.transpile(source.slice(start, end), { target: ts.ScriptTarget.ES2022 });
const turnId = "018f0000-0000-7000-8000-000000001000";
let around;
let loaded;
const context = {
  readableSnapshot: async (_session, options) => {
    around = options.around;
    // Mock a topic assignment whose source turn is roughly 1,000 turns away
    // from the latest page. The server returns a focused page directly.
    return {
      page: { older: "before-target", newer: "after-target", end: "focused", total_turns: 1000, first_turn: 1000 },
      presentation: { items: [] },
      revision: "revision-1000",
    };
  },
  batch: (callback) => callback(),
  hydrateHistoryPage: (session, snapshot, direction) => { loaded = { session, snapshot, direction }; },
  api: { sandboxExecutions: async () => ({ events: [] }) },
  hydrateWorkbenchExecutions() {},
  isRunning: () => false,
  setNotice: () => assert.fail("the mocked around page should be accepted"),
};
vm.createContext(context);
vm.runInContext(actual, context);
await context.focusTopicTurn("source-session", turnId);
assert.equal(around, turnId, "navigation fetches around the topic's source turn directly");
assert.equal(loaded.session, "source-session");
assert.equal(loaded.snapshot.page.first_turn, 1000, "a distant source turn loads without paging through intervening turns");
assert.equal(loaded.direction, "refresh", "the focused page becomes the reader's current history window");
console.log("topic navigation: directly opens a source turn across a 1,000-turn gap");
