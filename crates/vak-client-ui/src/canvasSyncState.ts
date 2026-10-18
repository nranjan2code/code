/** Queue and retry policy for conversation Canvas writes. Kept independent of
 * Solid and the browser so failures and races can be exercised in Node. */
export type CanvasSaveReason = "auth" | "too_large" | "missing" | "throttled" | "server" | "invalid" | "newer_schema" | "conflict" | "recovery" | "recovery_cleanup";
export type CanvasSaveState = "saved" | "saving" | "pending" | "error";
export interface CanvasSaveStatus { state: CanvasSaveState; reason?: CanvasSaveReason; recovery?: "saving" | "ready" | "unavailable" | "locked" }
export interface CanvasWriteError { status?: number; kind?: string }
export interface CanvasCheckpoint<Op, Snapshot, Stored> { base: number; pending: Op[]; snapshot: Snapshot; baseline?: Stored; conflict?: boolean }
export interface CanvasConflict<Op, Snapshot, Stored> {
  token: number;
  base: number;
  local: Snapshot;
  remote: Stored;
  pending: Op[];
  baseline?: Stored;
}

type WriteAnswer<T> = { saved: T } | { stale: T };
interface Recovery<Op, Snapshot, Stored> {
  load(id: string): Promise<CanvasCheckpoint<Op, Snapshot, Stored> | null>;
  save(id: string, checkpoint: CanvasCheckpoint<Op, Snapshot, Stored> | null): Promise<void>;
  restore(id: string, snapshot: Snapshot, pendingNow: readonly Op[]): boolean;
}
interface Options<Op, Snapshot, Stored> {
  put(conversation: string, base: number, snapshot: Snapshot): Promise<WriteAnswer<Stored>>;
  get(conversation: string): Promise<Stored>;
  snapshot(conversation: string): Snapshot;
  revision(stored: Stored): number;
  applyStored(conversation: string, stored: Stored, pending: readonly Op[], baseline?: Stored): boolean | void;
  recovery?: Recovery<Op, Snapshot, Stored>;
  retryMs?: number;
  schedule?(fn: () => void, delay: number): unknown;
  cancel?(timer: unknown): void;
  onStatus?(conversation: string, status: CanvasSaveStatus): void;
  equalSnapshot?(left: Snapshot, right: Snapshot): boolean;
  onConflict?(conversation: string, conflict: CanvasConflict<Op, Snapshot, Stored>): void;
  applyResolution?(conversation: string, snapshot: Snapshot): void;
}

interface Queue<Op, Snapshot, Stored> {
  base: number; pending: Op[]; writing: boolean; read: boolean; ready: boolean; loading?: Promise<boolean>;
  timer?: unknown; state: CanvasSaveStatus; baseline?: Stored; saves: Promise<void>; saveFailed: boolean; cleanup: boolean;
  conflict?: CanvasConflict<Op, Snapshot, Stored>; conflictNeedsRead: boolean; token: number;
}

function reasonOf(error: unknown): CanvasSaveReason | undefined {
  const e = error as CanvasWriteError | null;
  if (e?.kind === "newer_schema") return "newer_schema";
  switch (e?.status) {
    case 401: case 403: return "auth";
    case 404: case 410: return "missing";
    case 413: return "too_large";
    case 429: return "throttled";
    case 400: return "invalid";
    case 409: return "newer_schema";
    case undefined: return undefined;
    default: return (e?.status ?? 0) >= 500 ? "server" : "invalid";
  }
}

/** Owns this process's pending queue and, when configured, its recoverable checkpoint. */
export function createCanvasSyncCoordinator<Op, Snapshot, Stored>(options: Options<Op, Snapshot, Stored>) {
  const queues = new Map<string, Queue<Op, Snapshot, Stored>>();
  const schedule = options.schedule ?? ((fn, ms) => setTimeout(fn, ms));
  const cancel = options.cancel ?? ((timer) => clearTimeout(timer as ReturnType<typeof setTimeout>));
  const retryMs = options.retryMs ?? 5000;
  const queue = (id: string) => {
    let q = queues.get(id);
    if (!q) { q = { base: 0, pending: [], writing: false, read: false, ready: !options.recovery, state: { state: "saved" }, saves: Promise.resolve(), saveFailed: false, cleanup: false, conflictNeedsRead: false, token: 0 }; queues.set(id, q); }
    return q;
  };
  const publish = (id: string, q: Queue<Op, Snapshot, Stored>, status: CanvasSaveStatus) => {
    if (options.recovery && status.recovery === undefined) status.recovery = q.saveFailed ? "unavailable" : "ready";
    q.state = status;
    options.onStatus?.(id, status);
  };
  const checkpoint = (id: string, q: Queue<Op, Snapshot, Stored>, overrides: Partial<CanvasCheckpoint<Op, Snapshot, Stored>> = {}): CanvasCheckpoint<Op, Snapshot, Stored> => ({ base: q.base, pending: [...q.pending], snapshot: options.snapshot(id), ...(q.baseline !== undefined ? { baseline: q.baseline } : {}), ...(q.conflict || q.conflictNeedsRead ? { conflict: true } : {}), ...overrides });
  const conflict = (id: string, q: Queue<Op, Snapshot, Stored>, remote: Stored) => {
    q.conflictNeedsRead = false;
    q.conflict = { token: ++q.token, base: q.base, local: options.snapshot(id), remote, pending: [...q.pending], baseline: q.baseline };
    options.onConflict?.(id, q.conflict);
    publish(id, q, { state: "error", reason: "conflict" });
  };
  const save = (id: string, q: Queue<Op, Snapshot, Stored>, value: CanvasCheckpoint<Op, Snapshot, Stored> | null): Promise<void> => {
    if (!options.recovery) return Promise.resolve();
    const work = q.saves.catch(() => undefined).then(async () => {
      await options.recovery!.save(id, value);
      q.saveFailed = false;
    }).catch(() => { q.saveFailed = true; });
    q.saves = work.then(() => {
      if (options.recovery && q.ready) publish(id, q, { ...q.state, recovery: q.saveFailed ? "unavailable" : "ready" });
    });
    return q.saves;
  };
  async function initialize(id: string, q: Queue<Op, Snapshot, Stored>): Promise<boolean> {
    if (!options.recovery || q.ready) return true;
    if (q.loading) return q.loading;
    q.loading = (async () => {
      let saved: CanvasCheckpoint<Op, Snapshot, Stored> | null;
      try { saved = await options.recovery!.load(id); }
      catch { publish(id, q, { state: "error", reason: "recovery", recovery: "unavailable" }); return false; }
      if (saved) {
        const current = [...q.pending];
        let restored = false;
        try { restored = options.recovery!.restore(id, saved.snapshot, current); } catch { restored = false; }
        if (!restored) { publish(id, q, { state: "error", reason: "recovery", recovery: "locked" }); return false; }
        q.base = saved.base; q.baseline = saved.baseline; q.read = saved.baseline !== undefined; q.conflictNeedsRead = saved.conflict === true;
        q.pending = [...saved.pending, ...current];
      }
      q.ready = true;
      if (q.pending.length) publish(id, q, { state: "pending", recovery: saved ? "ready" : "saving" });
      else publish(id, q, { state: "saved", recovery: "ready" });
      return true;
    })();
    const result = await q.loading;
    q.loading = undefined;
    return result;
  }
  const scheduleWrite = (id: string, delay: number) => {
    const q = queue(id);
    if (q.timer !== undefined) cancel(q.timer);
    q.timer = schedule(() => { q.timer = undefined; void write(id); }, delay);
  };
  async function write(id: string): Promise<void> {
    const q = queue(id);
    if (!await initialize(id, q) || q.writing || q.pending.length === 0 || q.conflict || q.conflictNeedsRead) return;
    q.writing = true;
    publish(id, q, { state: "saving" });
    try {
      if (options.recovery) {
        let checkpointWork: Promise<void>;
        do { checkpointWork = q.saves; await checkpointWork; } while (checkpointWork !== q.saves);
      }
      const sent = q.pending.length;
      const sentSnapshot = options.snapshot(id);
      const answer = await options.put(id, q.base, sentSnapshot);
      if ("saved" in answer) {
        q.base = options.revision(answer.saved); q.read = true; q.baseline = answer.saved;
        q.pending.splice(0, sent);
      } else {
        if (options.applyStored(id, answer.stale, q.pending, q.baseline) === false) {
          q.writing = false;
          if (q.timer !== undefined) { cancel(q.timer); q.timer = undefined; }
          conflict(id, q, answer.stale);
          if (options.recovery) {
            await save(id, q, checkpoint(id, q, { conflict: true }));
          }
          return;
        }
        q.base = options.revision(answer.stale); q.read = true; q.baseline = answer.stale;
      }
      if (options.recovery) {
        const cp = q.pending.length ? checkpoint(id, q) : null;
        await save(id, q, cp);
        q.cleanup = cp === null && q.saveFailed;
      }
      q.writing = false;
      if (q.cleanup) { publish(id, q, { state: "error", reason: "recovery_cleanup", recovery: "unavailable" }); return; }
      if (q.pending.length) { publish(id, q, { state: "pending" }); scheduleWrite(id, 0); }
      else publish(id, q, { state: "saved" });
    } catch (error) {
      q.writing = false;
      const reason = reasonOf(error);
      publish(id, q, { state: "error", ...(reason ? { reason } : {}) });
      if (reason === undefined || reason === "server" || reason === "throttled") scheduleWrite(id, retryMs);
      else if (q.timer !== undefined) { cancel(q.timer); q.timer = undefined; }
    }
  }
  async function read(id: string): Promise<void> {
    if (!id) return;
    const q = queue(id);
    const wasReady = q.ready;
    if (!await initialize(id, q) || q.writing) return;
    if (q.conflictNeedsRead) {
      try {
        const remote = await options.get(id);
        if (q.pending.length === 0) {
          options.applyStored(id, remote, [], q.baseline);
          q.base = options.revision(remote); q.baseline = remote; q.read = true; q.conflictNeedsRead = false;
          if (options.recovery) await save(id, q, null);
          publish(id, q, q.saveFailed ? { state: "error", reason: "recovery_cleanup", recovery: "unavailable" } : { state: "saved" });
        } else if (options.revision(remote) > q.base) conflict(id, q, remote);
      } catch { /* Keep the recovered queue paused until a later read succeeds. */ }
      return;
    }
    if (q.conflict) {
      try {
        const remote = await options.get(id);
        if (!q.pending.length) {
          options.applyStored(id, remote, [], q.baseline);
          q.base = options.revision(remote); q.baseline = remote; q.read = true; q.conflict = undefined;
          if (options.recovery) await save(id, q, null);
          publish(id, q, q.saveFailed ? { state: "error", reason: "recovery_cleanup", recovery: "unavailable" } : { state: "saved" });
        } else if (options.revision(remote) >= options.revision(q.conflict.remote)) conflict(id, q, remote);
      } catch { /* Keep showing the last known comparison while offline. */ }
      return;
    }
    if (q.conflict || q.pending.length) {
      if (!wasReady && q.ready && q.pending.length) scheduleWrite(id, 0);
      return;
    }
    if (q.cleanup) {
      await save(id, q, null);
      q.cleanup = q.saveFailed;
      publish(id, q, q.cleanup ? { state: "error", reason: "recovery_cleanup", recovery: "unavailable" } : { state: "saved" });
      if (q.cleanup) return;
    }
    try {
      const stored = await options.get(id);
      if (q.writing || q.pending.length) return;
      if (q.read && options.revision(stored) <= q.base) return;
      if (options.applyStored(id, stored, [], q.baseline) === false) return;
      q.base = options.revision(stored); q.read = true; q.baseline = stored;
    } catch { /* A later level-triggered read can try again. */ }
  }
  return {
    enqueue(id: string, op: Op, delay = 0) {
      const q = queue(id); q.pending.push(op);
      if (options.recovery) { void initialize(id, q).then((ok) => { if (ok) void save(id, q, checkpoint(id, q)); }).catch(() => undefined); }
      if (q.conflict) {
        q.conflict = { ...q.conflict, token: ++q.token, local: options.snapshot(id), pending: [...q.pending] };
        options.onConflict?.(id, q.conflict);
        if (options.recovery) void save(id, q, checkpoint(id, q, { conflict: true }));
        publish(id, q, { state: "error", reason: "conflict" });
      } else if (q.conflictNeedsRead) {
        publish(id, q, { state: "error", reason: "conflict" });
      } else {
        publish(id, q, { state: "pending", ...(options.recovery ? { recovery: "saving" } : {}) }); scheduleWrite(id, delay);
      }
    },
    read,
    retry(id: string) {
      const q = queue(id);
      if (q.conflict || q.conflictNeedsRead) { void read(id); return; }
      if (q.pending.length) { if (q.timer !== undefined) { cancel(q.timer); q.timer = undefined; } void write(id); }
      else void read(id);
    },
    status(id: string): CanvasSaveStatus { return queue(id).state; },
    conflict(id: string): CanvasConflict<Op, Snapshot, Stored> | undefined { return queue(id).conflict; },
    async resolve(id: string, token: number, snapshot: Snapshot, pending: readonly Op[]): Promise<boolean> {
      const q = queue(id);
      const current = q.conflict;
      if (!current || current.token !== token || q.writing || q.conflictNeedsRead) return false;
      const same = options.equalSnapshot ?? ((left: Snapshot, right: Snapshot) => JSON.stringify(left) === JSON.stringify(right));
      if (!same(options.snapshot(id), current.local)) {
        conflict(id, q, current.remote);
        return false;
      }
      const remoteBase = options.revision(current.remote);
      const next = { base: remoteBase, pending: [...pending], snapshot, baseline: current.remote, conflict: true };
      if (options.recovery) {
        await save(id, q, next);
        if (q.saveFailed) { publish(id, q, { state: "error", reason: "conflict", recovery: "unavailable" }); return false; }
      }
      if (q.conflict?.token !== token) {
        if (options.recovery) await save(id, q, checkpoint(id, q, { conflict: true }));
        publish(id, q, { state: "error", reason: "conflict" });
        return false;
      }
      if (!same(options.snapshot(id), current.local)) {
        q.conflict = { ...current, token: ++q.token, local: options.snapshot(id), pending: [...q.pending] };
        options.onConflict?.(id, q.conflict);
        if (options.recovery) await save(id, q, checkpoint(id, q, { conflict: true }));
        publish(id, q, { state: "error", reason: "conflict" });
        return false;
      }
      options.applyResolution?.(id, snapshot);
      q.base = remoteBase; q.baseline = current.remote; q.pending = [...pending]; q.conflict = undefined; q.conflictNeedsRead = false;
      if (!q.pending.length) {
        // Keep edits made during the explicit-choice PUT queued behind it.
        // The acknowledgement covers only `snapshot`, never a later view.
        q.writing = true;
        try {
          const answer = await options.put(id, q.base, snapshot);
          if ("stale" in answer) {
            q.writing = false;
            conflict(id, q, answer.stale);
            if (options.recovery) await save(id, q, checkpoint(id, q, { conflict: true }));
            return false;
          }
          q.base = options.revision(answer.saved); q.baseline = answer.saved; q.read = true;
          if (options.recovery) {
            const cp = q.pending.length ? checkpoint(id, q) : null;
            await save(id, q, cp);
            q.cleanup = cp === null && q.saveFailed;
          }
          q.writing = false;
          if (q.cleanup) { publish(id, q, { state: "error", reason: "recovery_cleanup", recovery: "unavailable" }); return false; }
          if (q.pending.length) { publish(id, q, { state: "pending" }); scheduleWrite(id, 0); }
          else publish(id, q, { state: "saved" });
          return true;
        } catch {
          q.writing = false;
          q.conflictNeedsRead = true;
          if (options.recovery) await save(id, q, checkpoint(id, q, { conflict: true }));
          publish(id, q, { state: "error", reason: "conflict" });
          return false;
        }
      }
      publish(id, q, { state: q.pending.length ? "pending" : "saved" });
      if (q.pending.length) scheduleWrite(id, 0);
      return true;
    },
  };
}
