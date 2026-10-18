// Keeps each conversation's Canvas in step across surfaces (docs/design/66 §0a).
// Failed writes remain in the in-memory operation queue until acknowledged.
import { createEffect, createRoot, on, onCleanup, createSignal } from "solid-js";
import * as api from "./api";
import { applyOp, fromStored, toStored, withCanvas, type CanvasOp } from "./canvasStack";
import { activeId, canvasStacks, freshCanvasEntry, onCanvasChange, setCanvasStacks } from "./store";
import { watchSession } from "./streamHub";
import { createCanvasSyncCoordinator, type CanvasSaveStatus } from "./canvasSyncState";

/** Typing a note writes once it pauses, not on every key. */
const WRITE_AFTER_MS = 300;
const RETRY_MS = 5000;
const [statuses, setStatuses] = createSignal<Record<string, CanvasSaveStatus>>({});

function show(conversation: string, stored: api.StoredCanvas, pending: readonly CanvasOp[]) {
  setCanvasStacks((stacks) => {
    let next = withCanvas(stacks, conversation, fromStored(stored, stacks[conversation], freshCanvasEntry));
    for (const op of pending) next = applyOp(next, conversation, op, freshCanvasEntry);
    return next;
  });
}

const coordinator = createCanvasSyncCoordinator<CanvasOp, ReturnType<typeof toStored>, api.StoredCanvas>({
  put: api.putCanvas,
  get: api.getCanvas,
  snapshot: (conversation) => toStored(canvasStacks()[conversation]),
  revision: (stored) => stored.revision,
  applyStored: show,
  retryMs: RETRY_MS,
  onStatus(conversation, status) {
    setStatuses((current) => ({ ...current, [conversation]: status }));
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
