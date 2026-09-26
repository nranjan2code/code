import assert from "node:assert/strict";
import { fileKind, newestWaitingDraft, statusWords } from "../src/resultCard.ts";

// The card says where the file stands in words, and nothing when the server
// could not establish it.
assert.deepEqual(statusWords({ state: "draft", version: 2 }), { headline: "Draft, version 2, waiting for your review", folder: "your folder hasn't changed yet", waiting: true });
assert.deepEqual(statusWords({ state: "accepted", version: 3 }), { headline: "Accepted, version 3", folder: "now in your folder", waiting: false });
assert.deepEqual(statusWords({ state: "in_folder" }), { headline: "Saved in your folder", folder: "", waiting: false });
assert.equal(statusWords(null), null);
assert.equal(statusWords(undefined), null);
// No status line ever carries a size.
for (const status of [{ state: "draft", version: 1 }, { state: "accepted", version: 1 }, { state: "in_folder" }]) {
  const words = statusWords(status);
  assert.doesNotMatch(`${words.headline} ${words.folder}`, /byte|KB|MB/);
}

assert.equal(fileKind("isolated-review.html", "text/html"), "Web page");
assert.equal(fileKind("Budget.XLSX"), "Spreadsheet");
assert.equal(fileKind("photo", "image/png"), "Image");
assert.equal(fileKind("notes.md"), "Text");
assert.equal(fileKind("archive.bin", "application/octet-stream"), "File");

const review = (execution) => [{ id: `review-${execution}`, label: "Review draft", verb: "review_draft", data: { execution_id: execution } }];
const file = (id, timestamp, status, actions = review(id)) => ({ id, timestamp, turn_id: "t", role: "tool", kind: "artifact", status: "succeeded", content: { type: "artifact", artifact: { name: `${id}.html`, path: `${id}.html`, status } }, actions, fallback_text: id });
const draft = (version) => ({ state: "draft", version });

// The newest draft waiting for review wins, by time rather than position:
// saved-draft results are appended after the turns they belong to.
assert.equal(newestWaitingDraft([
  file("later", "2026-09-21T10:00:00Z", draft(1)),
  file("earlier", "2026-09-21T09:00:00Z", draft(2)),
]), "later");
// An accepted draft, a file saved straight to the folder, a draft whose
// review is not offered, or a card with no status is never the one.
assert.equal(newestWaitingDraft([
  file("waiting", "2026-09-21T09:00:00Z", draft(2)),
  file("accepted", "2026-09-21T10:00:00Z", { state: "accepted", version: 1 }),
  file("saved", "2026-09-21T11:00:00Z", { state: "in_folder" }, []),
  file("unreviewable", "2026-09-21T12:00:00Z", draft(1), []),
  file("unknown", "2026-09-21T13:00:00Z", undefined),
]), "waiting");
// Equal times fall back to position.
assert.equal(newestWaitingDraft([file("a", "2026-09-21T09:00:00Z", draft(1)), file("b", "2026-09-21T09:00:00Z", draft(1))]), "b");
assert.equal(newestWaitingDraft([]), null);

console.log("result-card: ok");
