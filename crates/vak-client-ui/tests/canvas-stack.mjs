import assert from "node:assert/strict";
import { activateEntry, activeEntry, applyOp, closeEntry, entriesOf, fromStored, isShared, MAX_ENTRIES, openEntry, toStored, updateEntry, withCanvas } from "../src/canvasStack.ts";
import { freshEntry, viewerSpec } from "../src/canvasViewers.ts";
import { fileSubject, subjectKey } from "../src/canvasSubject.ts";

const file = (path, origin = { sessionId: "s1" }) => fileSubject(path, origin, path);
const open = (stacks, conversation, subject, narrow = false) => openEntry(stacks, conversation, subject, (opened) => freshEntry(opened, narrow));

// Each conversation has its own Canvas.
let stacks = open({}, "a", file("one.html"));
stacks = open(stacks, "b", file("two.csv"));
assert.equal(activeEntry(stacks, "a").subject.path, "one.html");
assert.equal(activeEntry(stacks, "b").subject.path, "two.csv");
assert.equal(activeEntry(stacks, "c"), null);

// A new subject opens in front; reopening one keeps what was done in it.
stacks = open(stacks, "a", file("notes.rs"));
assert.equal(entriesOf(stacks, "a").length, 2);
assert.equal(activeEntry(stacks, "a").subject.path, "notes.rs");
stacks = updateEntry(stacks, "a", subjectKey(file("one.html")), { draft: "tighten the header", selection: { start: 3, end: 5 }, view: "source" });
stacks = open(stacks, "a", file("one.html"));
const back = activeEntry(stacks, "a");
assert.equal(entriesOf(stacks, "a").length, 2);
assert.deepEqual([back.draft, back.selection, back.view], ["tighten the header", { start: 3, end: 5 }, "source"]);

// A citation moves within the open file: the subject is replaced, the reader's state stays.
stacks = open(stacks, "a", { ...file("one.html"), anchor: "p:4" });
assert.equal(activeEntry(stacks, "a").subject.anchor, "p:4");
assert.equal(activeEntry(stacks, "a").draft, "tighten the header");

// The same name in another run, or another version of a draft, is another subject.
assert.notEqual(subjectKey(file("a.html")), subjectKey(file("a.html", { sessionId: "s1", executionId: "e1" })));
assert.notEqual(subjectKey(file("a.html", { sessionId: "s1", candidateId: "c1" })), subjectKey(file("a.html", { sessionId: "s1", candidateId: "c2" })));
assert.notEqual(subjectKey({ kind: "inline", title: "t", html: "<p>a</p>" }), subjectKey({ kind: "inline", title: "t", html: "<p>b</p>" }));

// Closing brings the neighbour forward; the last close empties the conversation.
stacks = open(stacks, "a", file("third.tsv"));
stacks = closeEntry(stacks, "a", subjectKey(file("third.tsv")));
assert.equal(activeEntry(stacks, "a").subject.path, "notes.rs");
stacks = closeEntry(stacks, "a", subjectKey(file("notes.rs")));
assert.equal(activeEntry(stacks, "a").subject.path, "one.html");
stacks = closeEntry(stacks, "a", subjectKey(file("one.html")));
assert.equal(activeEntry(stacks, "a"), null);
assert.equal(activeEntry(stacks, "b").subject.path, "two.csv");
assert.equal(closeEntry(stacks, "a", "nothing"), stacks);

// Activating an unknown tab changes nothing; a known one comes forward.
stacks = open(open({}, "a", file("x.html")), "a", file("y.html"));
assert.equal(activateEntry(stacks, "a", "missing"), stacks);
assert.equal(activeEntry(activateEntry(stacks, "a", subjectKey(file("x.html"))), "a").subject.path, "x.html");

// Too many drops the oldest one that is not the one being opened.
let crowded = {};
for (let i = 0; i < MAX_ENTRIES + 3; i++) crowded = open(crowded, "a", file(`f${i}.txt`));
assert.equal(entriesOf(crowded, "a").length, MAX_ENTRIES);
assert.equal(entriesOf(crowded, "a")[0].subject.path, "f3.txt");
assert.equal(activeEntry(crowded, "a").subject.path, `f${MAX_ENTRIES + 2}.txt`);

// What a viewer can do is data: documents open focused, a phone always does, a phone reads a PDF's text.
assert.equal(freshEntry(file("a.docx"), false).mode, "focused");
assert.equal(freshEntry(file("a.html"), false).mode, "split");
assert.equal(freshEntry(file("a.html"), true).mode, "focused");
assert.equal(freshEntry(file("a.pdf"), false).view, "pages");
assert.equal(freshEntry(file("a.pdf"), true).view, "text");
assert.equal(freshEntry(file("a.rs"), false).view, null);
assert.equal(freshEntry(file("a.docx"), false).feedbackOpen, false);
assert.equal(freshEntry(file("a.html"), false).feedbackOpen, true);
assert.equal(viewerSpec(file("a.csv")).views.length, 2);
assert.equal(viewerSpec({ kind: "live_server", title: "app", serverName: "web", sessionId: "s" }).devices, true);
assert.equal(viewerSpec(file("a.png")).devices, false);

// A routine has no comments to make and nothing to point at.
const automation = viewerSpec({ kind: "automation", title: "r", taskId: "t1" });
assert.equal(automation.feedback, "none");
assert.equal(automation.selects(null), null);
assert.equal(automation.reloadable, true);
assert.equal(freshEntry({ kind: "automation", title: "r", taskId: "t1" }, false).feedbackOpen, false);

// Reopening what is open keeps the very same subject, so nothing is read again.
{
  let shown = open({}, "a", file("page.html"));
  const before = activeEntry(shown, "a").subject;
  shown = open(shown, "a", file("page.html"));
  assert.equal(activeEntry(shown, "a").subject, before);
}

// A change is an operation: applied here, and again on a newer Canvas from another surface.
{
  const fresh = (subject) => freshEntry(subject, false);
  const theirs = applyOp(applyOp({}, "a", { op: "open", subject: file("x.html") }, fresh), "a", { op: "open", subject: file("y.html") }, fresh);
  const mine = [{ op: "open", subject: file("z.csv") }, { op: "update", key: subjectKey(file("z.csv")), patch: { draft: "sort by date" } }];
  let rebased = theirs;
  for (const op of mine) rebased = applyOp(rebased, "a", op, fresh);
  assert.deepEqual(entriesOf(rebased, "a").map((entry) => entry.subject.path), ["x.html", "y.html", "z.csv"]);
  assert.equal(activeEntry(rebased, "a").draft, "sort by date");
  // Only what another surface would see is shared: the layout is this device's.
  assert.equal(isShared({ op: "update", key: "k", patch: { mode: "focused" } }), false);
  assert.equal(isShared({ op: "update", key: "k", patch: { draft: "x" } }), true);

  // Stored without this device's layout; read back with it, and unchanged tabs stay the same objects.
  const stored = toStored(rebased.a);
  assert.ok(stored.entries.every((entry) => !("mode" in entry)));
  const local = { ...rebased.a, entries: rebased.a.entries.map((entry) => ({ ...entry, mode: "focused" })) };
  const back = fromStored(JSON.parse(JSON.stringify(stored)), local, fresh);
  assert.equal(back, local, "nothing changed, so nothing is redrawn");
  const moved = JSON.parse(JSON.stringify(stored));
  moved.active = subjectKey(file("x.html"));
  const after = fromStored(moved, local, fresh);
  assert.equal(after.active, subjectKey(file("x.html")));
  assert.equal(after.entries[0], local.entries[0]);
  assert.equal(after.entries[0].mode, "focused");
  // What this client cannot show is left out, never drawn.
  const hostile = { entries: [{ key: "k", subject: { kind: "nope", title: "?" } }, ...stored.entries], active: null };
  assert.equal(fromStored(hostile, undefined, fresh).entries.length, 3);
  assert.equal(fromStored({ entries: [], active: null }, local, fresh), null);
  assert.deepEqual(withCanvas({ a: local }, "a", null), {});
}

console.log("canvas stack ok");
