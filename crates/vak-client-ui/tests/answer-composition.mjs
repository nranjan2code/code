import assert from "node:assert/strict";
import { answerDocument, answerSegments, groupAnswerEntries } from "../src/answerComposition.ts";
const document = text => ({ source_markdown: text, metadata: {} });
const prose = (id, text = id, result = "r1") => ({ item: { id, role: "assistant", kind: "outcome", content: { type: "document", document: document(text) }, outcome: { result_id: result } } });
const card = (id, result = "r1", type = "structured") => ({ item: { id, role: "tool", kind: "card", content: { type }, outcome: { result_id: result } } });
const ids = entries => entries.map(entry => entry.item.id);
const source = [prose("before"), card("first"), prose("between", "Note: this is meaningful."), card("second"), prose("after")];
const groups = groupAnswerEntries(source);
assert.equal(groups.length, 1);
assert.deepEqual(ids(groups[0].entries), ids(source));
assert.deepEqual(answerSegments(groups[0].entries).map(ids), [["before"], ["first"], ["between"], ["second"], ["after"]]);
assert.equal(answerDocument(source[2].item).source_markdown, "Note: this is meaningful.");
// Text-only multipart answers and transitional phrases also belong to the
// projected answer; wording never grants permission to discard a paragraph.
assert.deepEqual(ids(groupAnswerEntries([prose("a", "Let me check the files."), prose("b")])[0].entries), ["a", "b"]);
// Typed non-result entries and distinct known result ids split groups.
const user = { item: { id: "user", role: "user", kind: "message", content: { type: "document", document: document("request") } } };
const error = { item: { id: "error", role: "tool", kind: "error", content: { type: "error" } } };
assert.deepEqual(groupAnswerEntries([user, ...source, error, card("other", "r2")]).map(group => group.kind), ["item", "result", "item", "result"]);
assert.equal(groupAnswerEntries([card("a", "r1"), card("unknown", null), card("b", "r2")]).length, 2);
const scoped = groupAnswerEntries([card("known", "r1"), prose("unknown-tail", "tail", null), card("next", "r2")]);
assert.equal(scoped[0].resultId, "r1");
assert.equal(scoped[1].resultId, "r2");
// Layout can group adjacent structured/adaptive cards, never across prose.
assert.deepEqual(answerSegments([card("a"), card("b", "r1", "adaptive"), prose("c"), card("d")]).map(ids), [["a", "b"], ["c"], ["d"]]);
const file = { item: { id: "file", role: "tool", kind: "artifact", content: { type: "artifact" } } };
assert.deepEqual(answerSegments([prose("a"), file]).map(ids), [["a"]]);
assert.equal(groupAnswerEntries([file])[0].entries[0], file);
assert.equal(answerDocument({role:"assistant",content:{type:"outcome",document:document("outcome")}}).source_markdown,"outcome");
assert.equal(answerDocument(user.item), undefined);
assert.deepEqual(groupAnswerEntries([]), []);
console.log("answer composition order and boundaries ok");
