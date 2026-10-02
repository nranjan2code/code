// Keeps each conversation's Canvas the same on every surface that shows it:
// the desktop app and the web app at once, or one after the other
// (docs/design/66 §0a). The server keeps the Canvas with the conversation's
// Agent and a revision; this module reads it and writes the changes made here.
//
// A change is applied here at once (`changeCanvas`) and queued as the
// operation it was. Queued changes are written together, from the revision
// they were made on; when another surface wrote first the server answers with
// its Canvas, the queued operations are applied again on top of it, and the
// result is written. Reading is level-triggered: on opening a conversation,
// on a `canvas` hint from the stream, and when the window comes back, so a
// missed hint costs a moment, never the Canvas.

import { createEffect, createRoot, onCleanup } from "solid-js";
import * as api from "./api";
import { applyOp, fromStored, toStored, withCanvas, type CanvasOp } from "./canvasStack";
import { activeId, canvasStacks, freshCanvasEntry, onCanvasChange, setCanvasStacks } from "./store";
import { watchSession } from "./streamHub";

interface Sync {
  /** The server's revision the Canvas here was last made from. */
  base: number;
  /** Changes made here and not yet written. */
  pending: CanvasOp[];
  writing: boolean;
  timer?: ReturnType<typeof setTimeout>;
  /** Whether the server's Canvas has been read since this session began. */
  read: boolean;
}

const syncs = new Map<string, Sync>();
/** Typing a note writes once it pauses, not on every key. */
const WRITE_AFTER_MS = 300;
/** A write that failed for want of the server is tried again after this. */
const RETRY_MS = 5000;

function syncOf(conversation: string): Sync {
  let sync = syncs.get(conversation);
  if (!sync) {
    sync = { base: 0, pending: [], writing: false, read: false };
    syncs.set(conversation, sync);
  }
  return sync;
}

function schedule(conversation: string, delay: number) {
  const sync = syncOf(conversation);
  if (sync.timer) clearTimeout(sync.timer);
  sync.timer = setTimeout(() => {
    sync.timer = undefined;
    void write(conversation);
  }, delay);
}

/** Shows the server's Canvas, with the changes made here and not yet written
 *  applied on top of it. */
function show(conversation: string, stored: api.StoredCanvas, pending: readonly CanvasOp[]) {
  setCanvasStacks((stacks) => {
    let next = withCanvas(stacks, conversation, fromStored(stored, stacks[conversation], freshCanvasEntry));
    for (const op of pending) next = applyOp(next, conversation, op, freshCanvasEntry);
    return next;
  });
}

async function write(conversation: string) {
  const sync = syncOf(conversation);
  if (sync.writing || sync.pending.length === 0) return;
  sync.writing = true;
  const sent = sync.pending.length;
  try {
    const answer = await api.putCanvas(conversation, sync.base, toStored(canvasStacks()[conversation]));
    if ("saved" in answer) {
      sync.base = answer.saved.revision;
      sync.read = true;
      sync.pending.splice(0, sent);
    } else {
      // Another surface wrote first: make the same changes on its Canvas.
      sync.base = answer.stale.revision;
      sync.read = true;
      show(conversation, answer.stale, sync.pending);
    }
    sync.writing = false;
    if (sync.pending.length) schedule(conversation, 0);
  } catch (cause) {
    sync.writing = false;
    if (cause instanceof api.ApiError && cause.status >= 400 && cause.status < 500) {
      // The conversation is gone, or this Canvas cannot be kept (too large):
      // it stays as it is here, and the next change tries again.
      sync.pending.length = 0;
      return;
    }
    schedule(conversation, RETRY_MS);
  }
}

/** Reads a conversation's Canvas from the server, unless changes made here
 *  are on their way (their write learns of anything newer). */
export async function readCanvas(conversation: string) {
  if (!conversation) return;
  const sync = syncOf(conversation);
  if (sync.writing || sync.pending.length) return;
  try {
    const stored = await api.getCanvas(conversation);
    if (sync.writing || sync.pending.length) return;
    if (sync.read && stored.revision === sync.base) return;
    sync.base = stored.revision;
    sync.read = true;
    show(conversation, stored, []);
  } catch {
    // Unreachable for now: the Canvas here stays, and the next read tries again.
  }
}

/** Starts keeping Canvases in step. Called once, when the app starts. */
export function startCanvasSync() {
  onCanvasChange((conversation, op) => {
    const sync = syncOf(conversation);
    sync.pending.push(op);
    schedule(conversation, op.op === "update" ? WRITE_AFTER_MS : 0);
  });
  createRoot(() => {
    createEffect(() => {
      const conversation = activeId();
      if (!conversation) return;
      void readCanvas(conversation);
      const stop = watchSession(conversation, {
        canvas: (revision) => {
          if (revision !== syncOf(conversation).base) void readCanvas(conversation);
        },
      });
      onCleanup(stop);
    });
  });
  const again = () => {
    const conversation = activeId();
    if (conversation && document.visibilityState === "visible") void readCanvas(conversation);
  };
  window.addEventListener("focus", again);
  document.addEventListener("visibilitychange", again);
}
