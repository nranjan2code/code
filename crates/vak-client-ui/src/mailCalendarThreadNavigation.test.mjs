import test from "node:test";
import assert from "node:assert/strict";
import { appendUniqueConversationMessages, loadConversationCitation, MAX_CITATION_PAGES } from "./mailCalendarThreadNavigation.mjs";

const page = (ids, next_cursor = null) => ({
  messages: ids.map((provider_id) => ({ provider_id })),
  next_cursor,
});

test("citation already on the first page does not fetch more conversation data", async () => {
  let calls = 0;
  const result = await loadConversationCitation(page(["first", "target"], "next"), "target", async () => {
    calls += 1;
    return page([], null);
  });
  assert.equal(result.found, true);
  assert.equal(result.additionalPages, 0);
  assert.equal(calls, 0);
  assert.equal(result.nextCursor, "next");
});

test("provider page overlap does not duplicate messages in the conversation workspace", () => {
  const existing = [{ provider_id: "new", body_text: "current version" }];
  const appended = appendUniqueConversationMessages(existing, [
    { provider_id: "new", body_text: "duplicate page copy" },
    { provider_id: "old", body_text: "older message" },
  ]);
  assert.deepEqual(appended.map((message) => message.provider_id), ["new", "old"]);
  assert.equal(appended[0].body_text, "current version");
});

test("citation navigation follows only pages from the selected conversation", async () => {
  const visited = [];
  const result = await loadConversationCitation(page(["first"], "cursor-1"), "target", async (cursor) => {
    visited.push(cursor);
    return cursor === "cursor-1" ? page(["second"], "cursor-2") : page(["target", "target"], "cursor-3");
  });
  assert.deepEqual(visited, ["cursor-1", "cursor-2"]);
  assert.deepEqual(result.messages.map((message) => message.provider_id), ["first", "second", "target"]);
  assert.equal(result.found, true);
  assert.equal(result.nextCursor, "cursor-3");
});

test("missing target stops when the provider conversation is exhausted", async () => {
  const result = await loadConversationCitation(page(["first"], "cursor-1"), "missing", async () => page(["second"], null));
  assert.equal(result.found, false);
  assert.equal(result.additionalPages, 1);
  assert.equal(result.nextCursor, null);
});

test("citation auto-fetch is capped and a repeated cursor cannot loop", async () => {
  let calls = 0;
  const result = await loadConversationCitation(page(["first"], "cursor"), "missing", async () => {
    calls += 1;
    return page([`item-${calls}`], "cursor");
  }, MAX_CITATION_PAGES + 100);
  assert.equal(calls, 1);
  assert.equal(result.found, false);
  assert.equal(result.nextCursor, "cursor");
  assert.equal(result.additionalPages, 1);
});

test("citation auto-fetch never exceeds the shared twenty-page ceiling", async () => {
  let calls = 0;
  const result = await loadConversationCitation(page(["first"], "0"), "missing", async () => {
    calls += 1;
    return page([`item-${calls}`], String(calls));
  }, MAX_CITATION_PAGES + 1);
  assert.equal(calls, MAX_CITATION_PAGES);
  assert.equal(result.additionalPages, MAX_CITATION_PAGES);
  assert.notEqual(result.nextCursor, null);
});
