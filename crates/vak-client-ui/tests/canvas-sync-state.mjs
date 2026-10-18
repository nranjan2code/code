import assert from "node:assert/strict";
import { createCanvasSyncCoordinator } from "../src/canvasSyncState.ts";

const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const tick = () => new Promise((resolve) => setImmediate(resolve));

function harness(put) {
  const timers = [];
  const snapshots = new Map();
  const applied = [];
  const reads = [];
  const coordinator = createCanvasSyncCoordinator({
    put: put ?? (async (_id, base, snapshot) => ({ saved: { revision: base + 1, snapshot } })),
    get: async (id) => { const stored = { revision: 9, entries: ["server"] }; reads.push(id); return stored; },
    snapshot: (id) => [...(snapshots.get(id) ?? [])],
    revision: (stored) => stored.revision,
    applyStored: (id, stored, pending) => applied.push({ id, stored, pending: [...pending] }),
    schedule: (fn, delay) => { const timer = { fn, delay, cancelled: false }; timers.push(timer); return timer; },
    cancel: (timer) => { timer.cancelled = true; },
    retryMs: 5000,
  });
  const enqueue = (id, op) => { snapshots.set(id, [...(snapshots.get(id) ?? []), op]); coordinator.enqueue(id, op); };
  const fire = async (delay) => {
    const timer = timers.find((candidate) => !candidate.cancelled && candidate.delay === delay);
    assert.ok(timer, `expected scheduled timer at ${delay}ms`);
    timer.cancelled = true; timer.fn(); await tick();
  };
  return { coordinator, enqueue, timers, applied, reads, fire };
}

for (const [status, reason] of [[401, "auth"], [403, "auth"], [404, "missing"], [409, "newer_schema"], [413, "too_large"]]) {
  const h = harness(async () => { throw Object.assign(new Error("write failed"), { status }); });
  h.enqueue("c", "draft"); await h.fire(0);
  assert.deepEqual(h.coordinator.status("c"), { state: "error", reason });
  assert.equal(h.timers.some((timer) => !timer.cancelled), false, `${status} must pause automatic retries`);
  await h.coordinator.read("c");
  assert.deepEqual(h.applied, [], `${status} must keep a read from replacing pending local work`);
  h.coordinator.retry("c"); await tick();
  assert.deepEqual(h.coordinator.status("c"), { state: "error", reason });
}

// An edit made while a write is in flight can schedule another timer. If the
// write then fails permanently, that timer must be cancelled along with the
// paused queue so it cannot silently retry later.
{
  const first = deferred(); let calls = 0;
  const h = harness(async () => { calls++; if (calls === 1) return first.promise; return { saved: { revision: 1 } }; });
  h.enqueue("c", "first");
  const initial = h.timers.find((entry) => entry.delay === 0); initial.cancelled = true; initial.fn();
  await tick();
  h.enqueue("c", "second");
  first.reject(Object.assign(new Error("unauthorized"), { status: 401 })); await tick();
  assert.deepEqual(h.coordinator.status("c"), { state: "error", reason: "auth" });
  assert.equal(h.timers.some((timer) => !timer.cancelled), false, "in-flight edit timer must be cancelled on permanent failure");
  h.coordinator.retry("c"); await tick();
  assert.deepEqual(h.coordinator.status("c"), { state: "saved" }, "explicit retry keeps and sends both pending operations");
}

// Only the typed server kind names a newer schema; other 4xx values stay
// invalid unless the status itself is the existing 409 newer-schema response.
{
  const h = harness(async () => { throw Object.assign(new Error("bad request"), { status: 400, kind: "unsupported_version" }); });
  h.enqueue("c", "draft"); await h.fire(0);
  assert.deepEqual(h.coordinator.status("c"), { state: "error", reason: "invalid" });
}

for (const failure of [Object.assign(new Error("offline")), Object.assign(new Error("busy"), { status: 500 }), Object.assign(new Error("slow down"), { status: 429 })]) {
  const h = harness(async () => { throw failure; });
  h.enqueue("c", "draft"); await h.fire(0);
  assert.equal(h.coordinator.status("c").state, "error");
  assert.ok(h.timers.some((timer) => !timer.cancelled && timer.delay === 5000));
}

// Changes made during an in-flight request survive its acknowledgement and
// are sent in the next request. Only the sent prefix is acknowledged.
{
  const first = deferred();
  const calls = [];
  const h = harness(async (_id, base, snapshot) => { calls.push({ base, snapshot }); return first.promise; });
  h.enqueue("c", "first");
  const timer = h.timers.find((entry) => entry.delay === 0); timer.fn();
  await tick();
  h.enqueue("c", "second");
  first.resolve({ saved: { revision: 1 } }); await tick();
  assert.equal(h.coordinator.status("c").state, "pending");
  await h.fire(0);
  assert.deepEqual(calls.map((call) => call.snapshot), [["first"], ["first", "second"]]);
  assert.deepEqual(h.coordinator.status("c"), { state: "saved" });
}

// A read already in flight cannot replace local edits, and focus retry sends
// the retained queue instead of accepting a remote read.
{
  const read = deferred(); const calls = [];
  const h = harness(async (_id, _base, snapshot) => { calls.push(snapshot); return { saved: { revision: 1 } }; });
  h.coordinator = createCanvasSyncCoordinator({
    put: async (...args) => { calls.push(args[2]); return { saved: { revision: 1 } }; },
    get: () => read.promise, snapshot: () => ["local"], revision: (stored) => stored.revision,
    applyStored: (...args) => h.applied.push(args), schedule: (fn, delay) => { const timer = { fn, delay }; h.timers.push(timer); return timer; },
  });
  const reading = h.coordinator.read("c");
  h.coordinator.enqueue("c", "local");
  read.resolve({ revision: 9, entries: ["remote"] }); await reading;
  assert.deepEqual(h.applied, []);
  h.coordinator.retry("c"); await tick();
  assert.deepEqual(calls, [["local"]]);
  assert.deepEqual(h.coordinator.status("c"), { state: "saved" });
}

// Concurrent level reads cannot roll the Canvas back when an older request
// finishes after a newer revision was already shown.
{
  const older = deferred(), newer = deferred();
  const applied = [];
  let count = 0;
  const coordinator = createCanvasSyncCoordinator({
    put: async () => { throw new Error("unexpected write"); },
    get: () => count++ === 0 ? older.promise : newer.promise,
    snapshot: () => ({}), revision: (stored) => stored.revision,
    applyStored: (_id, stored) => applied.push(stored.revision),
  });
  const first = coordinator.read("c"), second = coordinator.read("c");
  newer.resolve({ revision: 3 }); await second;
  older.resolve({ revision: 2 }); await first;
  assert.deepEqual(applied, [3]);
}

console.log("canvas sync failure preservation passed");
