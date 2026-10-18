import { rebaseCanvas } from "./canvasRebase";
// Keeps each conversation's Canvas in step across surfaces (docs/design/66 §0a).
// Failed writes remain in the in-memory operation queue until acknowledged.
import { createEffect, createRoot, on, onCleanup, createSignal } from "solid-js";
import * as api from "./api";
import { applyOp, fromStored, toStored, withCanvas, type CanvasOp, type StoredCanvasShape } from "./canvasStack";
import { isCanvasSubject, subjectKey } from "./canvasSubject";
import { createCanvasRecovery } from "./canvasRecovery";
import { activeId, canvasStacks, freshCanvasEntry, onCanvasChange, setCanvasStacks } from "./store";
import { watchSession } from "./streamHub";
import { createCanvasSyncCoordinator, type CanvasCheckpoint, type CanvasConflict, type CanvasSaveStatus } from "./canvasSyncState";

/** Typing a note writes once it pauses, not on every key. */
const WRITE_AFTER_MS = 300;
const RETRY_MS = 5000;
const [statuses, setStatuses] = createSignal<Record<string, CanvasSaveStatus>>({});
const [conflicts, setConflicts] = createSignal<Record<string, CanvasConflict<CanvasOp, StoredCanvasShape, api.StoredCanvas> | undefined>>({});

function show(conversation: string, stored: api.StoredCanvas, pending: readonly CanvasOp[], baseline?: api.StoredCanvas) {
  const stacks = canvasStacks();
  const result = rebaseCanvas(stacks[conversation], stored, baseline, pending, freshCanvasEntry);
  if ("conflict" in result) return false;
  setCanvasStacks(withCanvas(stacks, conversation, result.canvas));
  return true;
}

type Checkpoint = CanvasCheckpoint<CanvasOp, StoredCanvasShape, api.StoredCanvas>;
function object(value: unknown): value is Record<string, unknown> { return !!value && typeof value === "object" && !Array.isArray(value); }
function validPatch(patch: Record<string, unknown>): boolean {
  const fields = ["mode", "view", "selection", "draft", "feedbackOpen", "panel", "dismissedVersion"];
  if (!Object.keys(patch).every((key) => fields.includes(key))) return false;
  if (patch.mode !== undefined && patch.mode !== "split" && patch.mode !== "focused") return false;
  if (patch.view !== undefined && patch.view !== null && typeof patch.view !== "string") return false;
  if (patch.draft !== undefined && typeof patch.draft !== "string") return false;
  if (patch.feedbackOpen !== undefined && typeof patch.feedbackOpen !== "boolean") return false;
  if (patch.panel !== undefined && !["discussion", "activity", "changes"].includes(String(patch.panel))) return false;
  if (patch.dismissedVersion !== undefined && (!Number.isSafeInteger(patch.dismissedVersion) || (patch.dismissedVersion as number) < 0)) return false;
  const selection = patch.selection;
  if (selection === undefined || selection === null) return true;
  if (!object(selection)) return false;
  return selection.kind === "anchor" ? typeof selection.anchor === "string" && !!selection.anchor
    : selection.kind === "lines" && Number.isInteger(selection.start) && (selection.end === undefined || Number.isInteger(selection.end));
}
function validShape(value: unknown): value is StoredCanvasShape {
  if (!object(value) || !Array.isArray(value.entries) || value.entries.length > 8) return false;
  const keys = new Set<string>();
  for (const entry of value.entries) {
    if (!object(entry) || !isCanvasSubject(entry.subject) || entry.key !== subjectKey(entry.subject) || keys.has(entry.key as string)) return false;
    if (typeof entry.draft !== "string") return false;
    keys.add(entry.key as string);
  }
  return keys.size === 0 ? value.active === null : typeof value.active === "string" && keys.has(value.active);
}
function validCheckpoint(value: unknown): value is Checkpoint {
  if (!object(value) || !Number.isSafeInteger(value.base) || (value.base as number) < 0 || !validShape(value.snapshot) || !Array.isArray(value.pending) || (!value.pending.length && value.conflict !== true) || (value.conflict !== undefined && typeof value.conflict !== "boolean")) return false;
  if (value.baseline !== undefined && (!validShape(value.baseline) || !object(value.baseline) || value.baseline.schema !== 1 || value.baseline.revision !== value.base)) return false;
  return value.pending.every((op) => {
    if (!object(op)) return false;
    if (op.op === "open") return isCanvasSubject(op.subject);
    if (typeof op.key !== "string" || !op.key) return false;
    if (op.op === "activate" || op.op === "close") return true;
    if (op.op !== "update" || !object(op.patch)) return false;
    return validPatch(op.patch);
  });
}
const journal = createCanvasRecovery<CanvasOp, StoredCanvasShape, api.StoredCanvas>({
  // The desktop backend port changes at restart; the webview origin does not.
  // The immutable server identity is additionally authenticated in ciphertext.
  namespace: () => location.origin,
  key: api.canvasRecoveryKey,
  validate: validCheckpoint,
});

function restore(conversation: string, snapshot: StoredCanvasShape, pending: readonly CanvasOp[]): boolean {
  const stacks = canvasStacks();
  const recovered = fromStored(snapshot, stacks[conversation], freshCanvasEntry);
  // An edit made before unlocking must not silently replace an earlier note.
  for (const op of pending) {
    if (op.op !== "update" || op.patch.draft === undefined) continue;
    const prior = recovered?.entries.find((entry) => entry.key === op.key);
    if (!prior && op.patch.draft) return false;
    if (prior?.draft && prior.draft !== op.patch.draft) return false;
  }
  let next = withCanvas(stacks, conversation, recovered);
  for (const op of pending) {
    const updated = applyOp(next, conversation, op, freshCanvasEntry);
    if (updated === next && op.op === "open") return false;
    next = updated;
  }
  setCanvasStacks(next);
  return true;
}

const recoverySupported = typeof indexedDB !== "undefined" && !!globalThis.crypto?.subtle && !!navigator.locks;

const coordinator = createCanvasSyncCoordinator<CanvasOp, ReturnType<typeof toStored>, api.StoredCanvas>({
  put: api.putCanvas,
  get: api.getCanvas,
  snapshot: (conversation) => toStored(canvasStacks()[conversation]),
  revision: (stored) => stored.revision,
  applyStored: show,
  recovery: recoverySupported ? { ...journal, restore } : undefined,
  retryMs: RETRY_MS,
  equalSnapshot: (left, right) => JSON.stringify(left) === JSON.stringify(right),
  onConflict(conversation, conflict) { setConflicts((current) => ({ ...current, [conversation]: conflict })); },
  applyResolution(conversation, snapshot) {
    const current = canvasStacks();
    const canvas = fromStored(snapshot, current[conversation], freshCanvasEntry);
    setCanvasStacks(withCanvas(current, conversation, canvas));
  },
  onStatus(conversation, status) {
    setStatuses((current) => ({ ...current, [conversation]: !recoverySupported && status.state !== "saved" ? { ...status, recovery: "unavailable" } : status }));
  },
});

/** Reactive status for the header in this conversation's Canvas. */
export function canvasSaveState(conversation: string): CanvasSaveStatus {
  return statuses()[conversation] ?? { state: "saved" };
}

export interface CanvasConflictNote { key: string; title: string; local: string; saved: string }
export function canvasConflictReview(conversation: string) {
  const conflict = conflicts()[conversation];
  if (!conflict) return undefined;
  const local = new Map(conflict.local.entries.map((entry) => [String(entry.key), entry]));
  const remote = new Map(conflict.remote.entries.map((entry) => [String(entry.key), entry]));
  const baseline = new Map((conflict.baseline?.entries ?? []).map((entry) => [String(entry.key), entry]));
  const keys = new Set(conflict.pending.flatMap((op) => op.op === "update" && op.patch.draft !== undefined ? [op.key] : []));
  const notes: CanvasConflictNote[] = [];
  for (const key of keys) {
    const mine = local.get(key);
    if (!mine) continue;
    const saved = remote.get(key);
    const oldDraft = typeof baseline.get(key)?.draft === "string" ? String(baseline.get(key)?.draft) : "";
    const savedDraft = typeof saved?.draft === "string" ? String(saved.draft) : "";
    if (String(mine.draft ?? "") === savedDraft || (saved && savedDraft === oldDraft)) continue;
    const subject = mine.subject as Record<string, unknown> | undefined;
    notes.push({ key, title: typeof subject?.title === "string" ? subject.title : "Canvas note", local: String(mine.draft ?? ""), saved: saved ? savedDraft : "This note was removed from the saved Canvas." });
  }
  return { token: conflict.token, notes, remote: conflict.remote, baseline: conflict.baseline, local: conflict.local, pending: conflict.pending };
}

export async function resolveCanvasConflict(conversation: string, token: number, choices: Readonly<Record<string, "mine" | "saved">>): Promise<boolean> {
  const conflict = conflicts()[conversation];
  if (!conflict || conflict.token !== token) return false;
  const localCanvas = fromStored(conflict.local, canvasStacks()[conversation], freshCanvasEntry) ?? undefined;
  const rebased = rebaseCanvas(localCanvas, conflict.remote, conflict.baseline, conflict.pending, freshCanvasEntry, choices);
  if ("conflict" in rebased) return false;
  const target = toStored(rebased.canvas ?? undefined);
  const remoteCanvas = fromStored(conflict.remote, localCanvas, freshCanvasEntry);
  const pending = diffCanvas(remoteCanvas, rebased.canvas);
  const accepted = await coordinator.resolve(conversation, token, target, pending);
  if (accepted) setConflicts((current) => ({ ...current, [conversation]: undefined }));
  return accepted;
}

function diffCanvas(remote: ReturnType<typeof fromStored>, target: ReturnType<typeof fromStored>): CanvasOp[] {
  const before = remote?.entries ?? [];
  const after = target?.entries ?? [];
  const ops: CanvasOp[] = [];
  const beforeByKey = new Map(before.map((entry) => [entry.key, entry]));
  const afterByKey = new Map(after.map((entry) => [entry.key, entry]));
  for (const entry of after) if (!beforeByKey.has(entry.key)) ops.push({ op: "open", subject: entry.subject });
  for (const entry of before) if (!afterByKey.has(entry.key)) ops.push({ op: "close", key: entry.key });
  for (const entry of after) {
    const old = beforeByKey.get(entry.key);
    if (!old) continue;
    const patch: Partial<typeof entry> = {};
    for (const field of ["view", "selection", "draft", "feedbackOpen", "panel", "dismissedVersion"] as const) {
      if (JSON.stringify(entry[field]) !== JSON.stringify(old[field])) Object.assign(patch, { [field]: entry[field] });
    }
    if (Object.keys(patch).length) ops.push({ op: "update", key: entry.key, patch });
  }
  if (target && target.active !== remote?.active) ops.push({ op: "activate", key: target.active });
  return ops;
}

/** True while any operation has not yet received a successful write receipt. */
export function hasUnsavedCanvasChanges(): boolean {
  for (const status of Object.values(statuses())) if (status.state !== "saved") return true;
  return false;
}

/** A person can retry a paused failure; this also replaces a focus/visibility read. */
export function retryCanvas(conversation: string) { coordinator.retry(conversation); }

export async function readCanvas(conversation: string) { await coordinator.read(conversation); }

/** Starts keeping Canvases in step. Called once, when the app starts. */
export function startCanvasSync() {
  onCanvasChange((conversation, op) => coordinator.enqueue(conversation, op, op.op === "update" ? WRITE_AFTER_MS : 0));
  createRoot(() => {
    createEffect(on(activeId, (conversation) => {
      if (!conversation) return;
      retryCanvas(conversation);
      const stop = watchSession(conversation, {
        canvas: () => {
          // Hints only trigger a level read. The coordinator suppresses it
          // while local writes are pending, including paused failures.
          void coordinator.read(conversation);
        },
      });
      onCleanup(stop);
    }));
  });
  const again = () => {
    const conversation = activeId();
    if (conversation && document.visibilityState === "visible") retryCanvas(conversation);
  };
  window.addEventListener("focus", again);
  document.addEventListener("visibilitychange", again);
  window.addEventListener("beforeunload", (event) => {
    if (!hasUnsavedCanvasChanges()) return;
    event.preventDefault();
    event.returnValue = "";
  });
}
