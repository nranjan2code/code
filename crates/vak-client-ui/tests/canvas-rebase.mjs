import assert from "node:assert/strict";
import { rebaseCanvas } from "../src/canvasRebase.ts";
import { closeEntry, openEntry, toStored, updateEntry, MAX_ENTRIES } from "../src/canvasStack.ts";
import { createCanvasSyncCoordinator } from "../src/canvasSyncState.ts";
import { freshEntry } from "../src/canvasViewers.ts";
import { fileSubject, subjectKey } from "../src/canvasSubject.ts";

const file = (name) => fileSubject(name, { sessionId: "s" }, name);
const fresh = (subject) => freshEntry(subject, false);
const open = (canvas, subject) => openEntry({ c: canvas }, "c", subject, fresh).c;
const draft = (canvas, subject, value) => updateEntry({ c: canvas }, "c", subjectKey(subject), { draft: value }).c;
const stored = (canvas) => toStored(canvas);
const tick = () => new Promise((resolve) => setImmediate(resolve));

// Several queued keystroke snapshots replay to the final local note.
{
  const subject = file("a.md");
  let local = open(undefined, subject);
  local = draft(local, subject, "final draft");
  const base = stored(open(undefined, subject));
  const result = rebaseCanvas(local, { entries: [], active: null }, base, [
    { op: "open", subject },
    { op: "update", key: subjectKey(subject), patch: { draft: "f" } },
    { op: "update", key: subjectKey(subject), patch: { draft: "final draft" } },
  ], fresh);
  assert.equal(result.canvas.entries[0].draft, "final draft");
}

// A remote copy of the final value is already reconciled, including when no baseline exists.
{
  const subject = file("a.md");
  let local = draft(open(undefined, subject), subject, "same");
  const remote = stored(local);
  assert.ok(!("conflict" in rebaseCanvas(local, remote, stored(open(undefined, subject)), [], fresh)));
  assert.ok(!("conflict" in rebaseCanvas(local, remote, undefined, [], fresh)));
  const changed = draft(open(undefined, subject), subject, "remote update");
  assert.equal(rebaseCanvas(local, stored(changed), remote, [], fresh).canvas.entries[0].draft, "remote update",
    "a read without a queued draft edit accepts the server's latest note");
  const closed = { entries: [], active: null };
  assert.equal(rebaseCanvas(local, closed, remote, [], fresh).canvas, null,
    "a read without pending work accepts a remote close");
  const viewOnly = rebaseCanvas(local, stored(changed), remote, [
    { op: "update", key: subjectKey(subject), patch: { view: "source" } },
  ], fresh);
  assert.equal(viewOnly.canvas.entries[0].draft, "remote update", "a non-draft update keeps the remote note");
  const unknown = structuredClone(remote);
  unknown.entries[0].draft = "other";
  assert.deepEqual(rebaseCanvas(local, unknown, undefined, [
    { op: "update", key: subjectKey(subject), patch: { draft: "same" } },
  ], fresh), { conflict: true });
}

// A remote clear conflicts with a local edit made from the old nonempty note.
{
  const subject = file("clear.md");
  const baseCanvas = draft(open(undefined, subject), subject, "old");
  const local = draft(baseCanvas, subject, "local edit");
  const remote = draft(open(undefined, subject), subject, "");
  assert.deepEqual(rebaseCanvas(local, stored(remote), stored(baseCanvas), [
    { op: "update", key: subjectKey(subject), patch: { draft: "local edit" } },
  ], fresh), { conflict: true });
}

// A remotely closed dirty tab is restored; clean tabs may be evicted to make room.
{
  const subject = file("mine.md");
  const local = draft(open(undefined, subject), subject, "keep me");
  let remoteCanvas = open(undefined, file("remote.md"));
  remoteCanvas = draft(remoteCanvas, file("remote.md"), "remote note");
  const result = rebaseCanvas(local, stored(remoteCanvas), stored(local), [
    { op: "update", key: subjectKey(subject), patch: { draft: "keep me" } },
  ], fresh);
  assert.equal(result.canvas.entries.find((entry) => entry.key === subjectKey(subject)).draft, "keep me");
  assert.equal(result.canvas.entries.find((entry) => entry.key === subjectKey(file("remote.md"))).draft, "remote note");
  assert.equal(result.canvas.entries.find((entry) => entry.key === subjectKey(subject)).mode, local.entries[0].mode);
}

// If every remote tab is dirty, restoration cannot evict one and must conflict.
{
  const mineSubject = file("mine.md");
  const local = draft(open(undefined, mineSubject), mineSubject, "mine");
  let remoteCanvas;
  for (let i = 0; i < MAX_ENTRIES; i++) {
    const subject = file(`remote-${i}.md`);
    remoteCanvas = open(remoteCanvas, subject);
    remoteCanvas = draft(remoteCanvas, subject, `note-${i}`);
  }
  assert.deepEqual(rebaseCanvas(local, stored(remoteCanvas), stored(local), [
    { op: "update", key: subjectKey(mineSubject), patch: { draft: "mine" } },
  ], fresh), { conflict: true });
  const emptiedLocal = draft(local, mineSubject, "");
  const emptyResult = rebaseCanvas(emptiedLocal, stored(remoteCanvas), stored(local), [
    { op: "update", key: subjectKey(mineSubject), patch: { draft: "temporary" } },
    { op: "update", key: subjectKey(mineSubject), patch: { draft: "" } },
  ], fresh);
  assert.equal(emptyResult.canvas.entries.some((entry) => entry.key === subjectKey(mineSubject)), false,
    "an intermediate nonempty update must not revive a tab whose final local draft is empty");
}

// A concurrent change to the same note conflicts; unrelated tabs and notes merge.
{
  const mineSubject = file("mine.md");
  const otherSubject = file("other.md");
  const baselineCanvas = open(undefined, mineSubject);
  const baseline = stored(baselineCanvas);
  const local = draft(baselineCanvas, mineSubject, "local");
  let remoteCanvas = draft(open(undefined, mineSubject), mineSubject, "remote");
  remoteCanvas = draft(openEntry({ c: remoteCanvas }, "c", otherSubject, fresh).c, otherSubject, "unrelated");
  const beforeLocal = structuredClone(local);
  const beforeRemote = structuredClone(remoteCanvas);
  const conflict = rebaseCanvas(local, stored(remoteCanvas), baseline, [
    { op: "update", key: subjectKey(mineSubject), patch: { draft: "local" } },
  ], fresh);
  assert.deepEqual(conflict, { conflict: true });
  assert.deepEqual(local, beforeLocal);
  assert.deepEqual(remoteCanvas, beforeRemote);
}

console.log("canvas rebase ok");

// Coordinator integration: a conflict keeps the original base and queue until
// a later server revision safely agrees with the local draft.
{
  const subject = file("integrated.md");
  const key = subjectKey(subject);
  let canvas = open(undefined, subject);
  const baseline = { revision: 1, ...stored(canvas) };
  let server = baseline;
  let calls = [];
  const timers = [];
  const applied = [];
  const coordinator = createCanvasSyncCoordinator({
    get: async () => server,
    snapshot: () => stored(canvas),
    revision: (value) => value.revision,
    put: async (_id, base, snapshot) => {
      calls.push({ base, snapshot });
      if (calls.length === 1) return { stale: { revision: 2, ...stored(draft(open(undefined, subject), subject, "remote")) } };
      if (calls.length === 2) return { stale: { revision: 2, ...stored(draft(open(undefined, subject), subject, "remote")) } };
      if (calls.length === 3) return { stale: server };
      return { saved: { revision: 4, ...snapshot } };
    },
    applyStored: (_id, value, pending, old) => {
      const result = rebaseCanvas(canvas, value, old, pending, fresh);
      if ("conflict" in result) return false;
      canvas = result.canvas;
      applied.push(value.revision);
      return true;
    },
    schedule: (fn, delay) => { const timer = { fn, delay, cancelled: false }; timers.push(timer); return timer; },
    cancel: (timer) => { timer.cancelled = true; },
  });
  await coordinator.read("c");
  canvas = draft(canvas, subject, "local");
  coordinator.enqueue("c", { op: "update", key, patch: { draft: "local" } });
  const fire = async () => {
    const timer = timers.find((item) => !item.cancelled);
    assert.ok(timer, "expected queued coordinator write");
    timer.cancelled = true;
    timer.fn();
    await tick();
  };
  await fire();
  assert.deepEqual(coordinator.status("c"), { state: "error", reason: "conflict" });
  assert.equal(canvas.entries[0].draft, "local", "failed rebase must not publish server state");
  coordinator.retry("c"); await tick();
  assert.deepEqual(coordinator.status("c"), { state: "error", reason: "conflict" });
  assert.deepEqual(calls.slice(0, 2).map((call) => call.base), [1, 1], "conflict retries from the last accepted baseline");
  assert.deepEqual(applied, [1], "conflicting snapshots are never published");

  server = { revision: 3, ...stored(draft(open(undefined, subject), subject, "local")) };
  coordinator.retry("c"); await tick();
  assert.deepEqual(coordinator.status("c"), { state: "pending" });
  assert.equal(calls[2].base, 1, "the stale response is still based on the original accepted revision");
  assert.deepEqual(applied, [1, 3]);
  await fire();
  assert.deepEqual(coordinator.status("c"), { state: "saved" });
  assert.equal(calls[3].base, 3, "safe rebase advances the next write to revision three");
}

// An empty remote note has no competing text, even without a known baseline.
{
  const subject = file("new.md");
  const clean = open(undefined, subject);
  const local = draft(clean, subject, "new local note");
  const result = rebaseCanvas(local, stored(clean), undefined, [
    { op: "update", key: subjectKey(subject), patch: { draft: "new local note" } },
  ], fresh);
  assert.equal(result.canvas.entries[0].draft, "new local note");
}

// Explicit local closure supersedes its earlier queued typing; replay must
// not invent a lost note or permanently block that intentional close.
{
  const subject = file("closed.md"), key = subjectKey(subject);
  const base = open(undefined, subject);
  const local = closeEntry({ c: draft(base, subject, "typed then closed") }, "c", key).c;
  const result = rebaseCanvas(local, stored(base), stored(base), [
    { op: "update", key, patch: { draft: "typed then closed" } },
    { op: "close", key },
  ], fresh);
  assert.equal(result.canvas, null);
}
