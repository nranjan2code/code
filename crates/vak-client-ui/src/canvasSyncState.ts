/** Queue and retry policy for conversation Canvas writes. Kept independent of
 * Solid and the browser so failures and races can be exercised in Node. */
export type CanvasSaveReason = "auth" | "too_large" | "missing" | "throttled" | "server" | "invalid" | "newer_schema" | "conflict" | "recovery" | "recovery_cleanup";
export type CanvasSaveState = "saved" | "saving" | "pending" | "error";
export interface CanvasSaveStatus { state: CanvasSaveState; reason?: CanvasSaveReason; recovery?: "saving" | "ready" | "unavailable" | "locked" }
export interface CanvasWriteError { status?: number; kind?: string }
export interface CanvasCheckpoint<Op, Snapshot, Stored> { base: number; pending: Op[]; snapshot: Snapshot; baseline?: Stored }

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
}

interface Queue<Op, Stored> {
  base: number; pending: Op[]; writing: boolean; read: boolean; ready: boolean; loading?: Promise<boolean>;
  timer?: unknown; state: CanvasSaveStatus; baseline?: Stored; saves: Promise<void>; saveFailed: boolean; cleanup: boolean;
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
  const queues = new Map<string, Queue<Op, Stored>>();
  const schedule = options.schedule ?? ((fn, ms) => setTimeout(fn, ms));
  const cancel = options.cancel ?? ((timer) => clearTimeout(timer as ReturnType<typeof setTimeout>));
  const retryMs = options.retryMs ?? 5000;
  const queue = (id: string) => {
    let q = queues.get(id);
    if (!q) { q = { base: 0, pending: [], writing: false, read: false, ready: !options.recovery, state: { state: "saved" }, saves: Promise.resolve(), saveFailed: false, cleanup: false }; queues.set(id, q); }
    return q;
  };
  const publish = (id: string, q: Queue<Op, Stored>, status: CanvasSaveStatus) => {
    if (options.recovery && status.recovery === undefined) status.recovery = q.saveFailed ? "unavailable" : "ready";
    q.state = status;
    options.onStatus?.(id, status);
  };
  const checkpoint = (id: string, q: Queue<Op, Stored>): CanvasCheckpoint<Op, Snapshot, Stored> => ({ base: q.base, pending: [...q.pending], snapshot: options.snapshot(id), ...(q.baseline !== undefined ? { baseline: q.baseline } : {}) });
  const save = (id: string, q: Queue<Op, Stored>, value: CanvasCheckpoint<Op, Snapshot, Stored> | null): Promise<void> => {
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
  async function initialize(id: string, q: Queue<Op, Stored>): Promise<boolean> {
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
        q.base = saved.base; q.baseline = saved.baseline; q.read = saved.baseline !== undefined;
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
    if (!await initialize(id, q) || q.writing || q.pending.length === 0) return;
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
          publish(id, q, { state: "error", reason: "conflict" });
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
    if (!await initialize(id, q) || q.writing || q.pending.length) {
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
      publish(id, q, { state: "pending", ...(options.recovery ? { recovery: "saving" } : {}) }); scheduleWrite(id, delay);
    },
    read,
    retry(id: string) {
      const q = queue(id);
      if (q.pending.length) { if (q.timer !== undefined) { cancel(q.timer); q.timer = undefined; } void write(id); }
      else void read(id);
    },
    status(id: string): CanvasSaveStatus { return queue(id).state; },
  };
}
