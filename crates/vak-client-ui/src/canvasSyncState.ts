/** Queue and retry policy for conversation Canvas writes. Kept independent of
 * Solid and the browser so failures and races can be exercised in Node. */
export type CanvasSaveReason = "auth" | "too_large" | "missing" | "throttled" | "server" | "invalid" | "newer_schema" | "conflict";
export type CanvasSaveState = "saved" | "saving" | "pending" | "error";
export interface CanvasSaveStatus { state: CanvasSaveState; reason?: CanvasSaveReason }
export interface CanvasWriteError { status?: number; kind?: string }

type WriteAnswer<T> = { saved: T } | { stale: T };
interface Options<Op, Snapshot, Stored> {
  put(conversation: string, base: number, snapshot: Snapshot): Promise<WriteAnswer<Stored>>;
  get(conversation: string): Promise<Stored>;
  snapshot(conversation: string): Snapshot;
  revision(stored: Stored): number;
  applyStored(conversation: string, stored: Stored, pending: readonly Op[], baseline?: Stored): boolean | void;
  retryMs?: number;
  schedule?(fn: () => void, delay: number): unknown;
  cancel?(timer: unknown): void;
  onStatus?(conversation: string, status: CanvasSaveStatus): void;
}

interface Queue<Op, Stored> {
  base: number; pending: Op[]; writing: boolean; read: boolean;
  timer?: unknown; state: CanvasSaveStatus; baseline?: Stored;
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

/** Owns only this process's pending operation queue; it writes no durable state. */
export function createCanvasSyncCoordinator<Op, Snapshot, Stored>(options: Options<Op, Snapshot, Stored>) {
  const queues = new Map<string, Queue<Op, Stored>>();
  const schedule = options.schedule ?? ((fn, ms) => setTimeout(fn, ms));
  const cancel = options.cancel ?? ((timer) => clearTimeout(timer as ReturnType<typeof setTimeout>));
  const retryMs = options.retryMs ?? 5000;
  const queue = (id: string) => {
    let q = queues.get(id);
    if (!q) { q = { base: 0, pending: [], writing: false, read: false, state: { state: "saved" } }; queues.set(id, q); }
    return q;
  };
  const publish = (id: string, q: Queue<Op, Stored>, status: CanvasSaveStatus) => {
    q.state = status;
    options.onStatus?.(id, status);
  };
  const scheduleWrite = (id: string, delay: number) => {
    const q = queue(id);
    if (q.timer !== undefined) cancel(q.timer);
    q.timer = schedule(() => { q.timer = undefined; void write(id); }, delay);
  };
  async function write(id: string): Promise<void> {
    const q = queue(id);
    if (q.writing || q.pending.length === 0) return;
    q.writing = true;
    const sent = q.pending.length;
    publish(id, q, { state: "saving" });
    try {
      const answer = await options.put(id, q.base, options.snapshot(id));
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
      q.writing = false;
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
    if (q.writing || q.pending.length) return;
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
      publish(id, q, { state: "pending" }); scheduleWrite(id, delay);
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
