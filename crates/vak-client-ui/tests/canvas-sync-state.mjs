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

// A declined stale rebase must preserve the original CAS revision. Retrying
// may ask again, but must never send an unreviewed snapshot at the remote head.
{
  const timers = [], bases = [], baselines = [];
  let accept = false;
  const coordinator = createCanvasSyncCoordinator({
    get: async () => ({ revision: 1, note: "original" }),
    put: async (_id, base) => { bases.push(base); return { stale: { revision: 2, note: "remote" } }; },
    snapshot: () => ({ note: "local" }), revision: (stored) => stored.revision,
    applyStored: (_id, _stored, pending, baseline) => {
      if (!pending.length) return true;
      baselines.push(baseline);
      return accept;
    },
    schedule: (fn) => { const timer = { fn, cancelled: false }; timers.push(timer); return timer; },
    cancel: (timer) => { timer.cancelled = true; },
  });
  await coordinator.read("c");
  coordinator.enqueue("c", "local edit");
  const timer = timers[0]; timer.cancelled = true; timer.fn(); await tick();
  assert.deepEqual(coordinator.status("c"), { state: "error", reason: "conflict" });
  assert.equal(timers.some((entry) => !entry.cancelled), false);
  coordinator.retry("c"); await tick();
  assert.deepEqual(bases, [1, 1], "conflict must not advance the write revision");
  assert.deepEqual(baselines.map((baseline) => baseline.note), ["original", "original"]);
  accept = true;
  coordinator.retry("c"); await tick();
  assert.equal(coordinator.status("c").state, "pending", "safe replay remains pending until a real write receipt");
}

// A recovered checkpoint restores the view and resumes the original CAS base.
{
  const timers = [], puts = [], restored = [];
  let durable = { base: 4, pending: ["offline"], snapshot: ["offline-view"], baseline: { revision: 4 } };
  const coordinator = createCanvasSyncCoordinator({
    recovery: { load: async () => durable, save: async (_id, value) => { durable = value; }, restore: (_id, snapshot, pending) => { restored.push({ snapshot, pending: [...pending] }); return true; } },
    get: async () => { throw new Error("checkpoint should avoid initial fetch"); },
    put: async (_id, base, snapshot) => { puts.push({ base, snapshot }); return { saved: { revision: base + 1 } }; },
    snapshot: () => ["offline-view"], revision: (stored) => stored.revision, applyStored: () => true,
    schedule: (fn, delay) => { const timer = { fn, delay, cancelled: false }; timers.push(timer); return timer; }, cancel: (timer) => { timer.cancelled = true; },
  });
  await coordinator.read("c");
  await tick();
  assert.deepEqual(restored, [{ snapshot: ["offline-view"], pending: [] }]);
  coordinator.retry("c"); await tick(); await tick();
  assert.equal(puts.length, 1); assert.equal(puts[0].base, 4); assert.deepEqual(puts[0].snapshot, ["offline-view"]);
  assert.equal(durable, null);
}

// A failed load blocks both network directions; an explicit retry can load again.
{
  let loads = 0, gets = 0, puts = 0;
  const timers = [];
  const coordinator = createCanvasSyncCoordinator({
    recovery: { load: async () => { if (++loads === 1) throw new Error("storage unavailable"); return { base: 2, pending: ["old"], snapshot: ["restored"] }; }, save: async () => {}, restore: () => true },
    get: async () => { gets++; return { revision: 3 }; }, put: async () => { puts++; return { saved: { revision: 3 } }; },
    snapshot: () => ["restored"], revision: (x) => x.revision, applyStored: () => true,
    schedule: (fn, delay) => { const t = { fn, delay }; timers.push(t); return t; },
  });
  await coordinator.read("c");
  assert.deepEqual(coordinator.status("c"), { state: "error", reason: "recovery", recovery: "unavailable" });
  coordinator.enqueue("c", "new"); await tick();
  assert.equal(gets, 0); assert.equal(puts, 0);
  coordinator.retry("c"); await tick();
  assert.equal(loads, 2); assert.equal(puts, 1);
}

// New edits arriving during async recovery are included in restore and remain queued.
{
  const loading = deferred(), restored = [], timers = [];
  const coordinator = createCanvasSyncCoordinator({
    recovery: { load: () => loading.promise, save: async () => {}, restore: (_id, snapshot, pending) => { restored.push({ snapshot, pending: [...pending] }); return true; } },
    get: async () => ({ revision: 0 }), put: async () => ({ saved: { revision: 1 } }),
    snapshot: () => ["local"], revision: (x) => x.revision, applyStored: () => true,
    schedule: (fn, delay) => { const t = { fn, delay, cancelled: false }; timers.push(t); return t; }, cancel: (t) => { t.cancelled = true; },
  });
  const reading = coordinator.read("c"); coordinator.enqueue("c", "new-edit");
  loading.resolve({ base: 5, pending: ["recovered"], snapshot: ["old"] }); await reading; await tick();
  assert.deepEqual(restored, [{ snapshot: ["old"], pending: ["new-edit"] }]);
  assert.equal(coordinator.status("c").state, "pending");
}

// Checkpoint failures are visible but do not suppress canonical server writes.
{
  let puts = 0;
  const timers = [];
  const coordinator = createCanvasSyncCoordinator({
    recovery: { load: async () => null, save: async () => { throw new Error("quota"); }, restore: () => true },
    get: async () => ({ revision: 0 }), put: async () => { puts++; return { saved: { revision: 1 } }; },
    snapshot: () => ["local"], revision: (x) => x.revision, applyStored: () => true,
    schedule: (fn, delay) => { const t = { fn, delay, cancelled: false }; timers.push(t); return t; }, cancel: (t) => { t.cancelled = true; },
  });
  coordinator.enqueue("c", "op"); await tick(); coordinator.retry("c"); await tick();
  assert.equal(puts, 1);
  assert.equal(coordinator.status("c").recovery, "unavailable");
}

// Serialized checkpoint writes ensure an older delayed save cannot follow ACK clear.
{
  const firstSave = deferred(), saves = [], timers = [];
  let saveCalls = 0, puts = 0;
  const coordinator = createCanvasSyncCoordinator({
    recovery: { load: async () => null, save: async (_id, cp) => { saves.push(cp); if (++saveCalls === 1) await firstSave.promise; }, restore: () => true },
    get: async () => ({ revision: 0 }), put: async () => { puts++; return { saved: { revision: 1 } }; },
    snapshot: () => ["local"], revision: (x) => x.revision, applyStored: () => true,
    schedule: (fn, delay) => { const t = { fn, delay, cancelled: false }; timers.push(t); return t; }, cancel: (t) => { t.cancelled = true; },
  });
  coordinator.enqueue("c", "op"); await tick();
  coordinator.retry("c"); await tick();
  assert.equal(puts, 0, "PUT waits for the pending checkpoint operation");
  firstSave.resolve(); await tick(); await tick();
  assert.equal(puts, 1); assert.equal(saves.at(-1), null, "ACK clear is ordered after any earlier checkpoint save");
}

// The acknowledged prefix and snapshot are captured together after checkpoint
// work; edits made while the request is in flight remain in the next checkpoint.
{
  const putFirst = deferred(), timers = [], saves = [];
  let view = [], calls = 0;
  const coordinator = createCanvasSyncCoordinator({
    recovery: { load: async () => null, save: async (_id, cp) => { saves.push(cp); }, restore: () => true },
    get: async () => ({ revision: 0 }),
    put: async (_id, base, snapshot) => { calls++; if (calls === 1) return putFirst.promise; return { saved: { revision: base + 1 } }; },
    snapshot: () => [...view], revision: (x) => x.revision, applyStored: () => true,
    schedule: (fn, delay) => { const t = { fn, delay, cancelled: false }; timers.push(t); return t; }, cancel: (t) => { t.cancelled = true; },
  });
  view.push("one"); coordinator.enqueue("c", "one"); await tick(); await tick();
  coordinator.retry("c"); await tick(); await tick();
  assert.equal(calls, 1);
  view.push("two"); coordinator.enqueue("c", "two");
  putFirst.resolve({ saved: { revision: 1 } }); await tick(); await tick();
  assert.equal(coordinator.status("c").state, "pending");
  const durablePending = saves.filter(Boolean).at(-1);
  assert.deepEqual(durablePending.pending, ["two"]);
  assert.deepEqual(durablePending.snapshot, ["one", "two"]);
}

console.log("canvas sync failure preservation passed");

// A failed ACK cleanup stays visible and retries cleanup without replaying PUT.
{
  let failClear = true, puts = 0;
  const coordinator = createCanvasSyncCoordinator({
    recovery: { load: async () => null, save: async (_id, cp) => { if (cp === null && failClear) throw new Error("quota"); }, restore: () => true },
    get: async () => ({ revision: 1 }), put: async () => { puts++; return { saved: { revision: 1 } }; },
    snapshot: () => ["note"], revision: (x) => x.revision, applyStored: () => true,
    schedule: () => 1, cancel: () => {},
  });
  coordinator.enqueue("cleanup", "note"); await tick();
  coordinator.retry("cleanup"); await tick(); await tick();
  assert.equal(coordinator.status("cleanup").reason, "recovery_cleanup");
  failClear = false;
  coordinator.retry("cleanup"); await tick(); await tick();
  assert.equal(coordinator.status("cleanup").state, "saved");
  assert.equal(puts, 1, "cleanup retry never resends acknowledged operations");
}
