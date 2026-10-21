import assert from "node:assert/strict";
import { HistoryPages } from "../src/historyPages.ts";
import { mergeTurnSnapshot } from "../src/presentationHydration.ts";

function page(first, last, request = undefined) {
  const entries = Array.from({ length: last - first + 1 }, (_, offset) => ({ entry_id: `entry-${first + offset}` }));
  return { session_id: "s", revision: "r1", running: false,
    transcript: { messages: entries.map(entry => ({ role: "assistant", content: [{ type: "text", text: entry.entry_id }] })), entries },
    presentation: { schema_version: 2, session_id: "s", items: entries.map(entry => ({ id: entry.entry_id, turn_id: entry.entry_id, fallback_text: "answer" })), diagnostics: [] },
    page: { request, end: `end-${last}`, older: first ? `before-${first}` : null, newer: last < 119 ? `after-${last}` : null, total_turns: 120 } };
}
const pages = new HistoryPages();
pages.add(page(80, 119), "refresh");
pages.add(page(40, 79, "before-80"), "older");
pages.add(page(0, 39, "before-40"), "older");
assert.equal(pages.merged().entries.length, 120);
assert.equal(pages.merged().entries[0].entry_id, "entry-0");
pages.add(page(40, 79, "before-80"), "older");
assert.equal(pages.merged().entries.length, 120, "repeated pages are idempotent");
pages.add(page(40, 79, "before-80"), "update");
assert.equal(pages.merged().entries.length, 120, "conditional refresh preserves adjacent pages");
const budget = JSON.stringify(page(40, 79, "before-80")).length + 50;
const bounded = new HistoryPages(budget);
bounded.add(page(80, 119), "refresh");
bounded.add(page(40, 79, "before-80"), "older");
assert.equal(bounded.size, 1);
assert.equal(bounded.newer, "after-79", "evicted newer content stays reloadable");
bounded.add(page(80, 119), "newer");
assert.equal(bounded.size, 1);
assert.equal(bounded.older, "before-80");
const current = page(0, 39).presentation;
const tail = page(39, 40).presentation;
tail.items[0].fallback_text = "changed";
const settled = mergeTurnSnapshot(current, tail);
assert.equal(settled.items.length, 41);
assert.equal(settled.items.find(item => item.id === "entry-39").fallback_text, "changed");
assert.equal(settled.items[0].id, "entry-0", "settling the live turn preserves older pages");
console.log("history pages: ordering, deduplication, eviction and turn settlement ok");

const live = { ...tail, cursor: "live:1" };
assert.equal(mergeTurnSnapshot(current, live).items[39].fallback_text, "answer", "live snapshots preserve settled output authority");
