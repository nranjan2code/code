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
import { createCanvasSyncCoordinator, type CanvasCheckpoint, type CanvasSaveStatus } from "./canvasSyncState";

/** Typing a note writes once it pauses, not on every key. */
const WRITE_AFTER_MS = 300;
const RETRY_MS = 5000;
const [statuses, setStatuses] = createSignal<Record<string, CanvasSaveStatus>>({});

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
  if (!object(value) || !Number.isSafeInteger(value.base) || (value.base as number) < 0 || !validShape(value.snapshot) || !Array.isArray(value.pending) || !value.pending.length) return false;
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
  onStatus(conversation, status) {
    setStatuses((current) => ({ ...current, [conversation]: !recoverySupported && status.state !== "saved" ? { ...status, recovery: "unavailable" } : status }));
  },
});

/** Reactive status for the header in this conversation's Canvas. */
export function canvasSaveState(conversation: string): CanvasSaveStatus {
  return statuses()[conversation] ?? { state: "saved" };
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
