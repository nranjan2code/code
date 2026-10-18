import { createEffect, createSignal, For, Show } from "solid-js";

import type { CanvasSaveStatus } from "../../canvasSyncState";
import type { CanvasConflictNote } from "../../canvasSync";

function explanation(reason?: string): string {
  switch (reason) {
    case "recovery_cleanup": return "Canvas changes saved, but local recovery cleanup failed. Keep this window open and retry before reloading.";
    case "recovery": return "Earlier Canvas changes could not be recovered. Connect and sign in, then retry. Copy any notes written in this window before reloading. Earlier recovery data is kept.";
    case "conflict": return "Another window changed these notes or filled the Canvas. Your local changes are kept here. Copy your notes, resolve the other window, then retry.";
    case "auth": return "Canvas changes haven’t saved. Sign in again, then retry.";
    case "too_large": return "Canvas changes are too large to save. Shorten a note or close an unused tab, then retry.";
    case "missing": return "This conversation is unavailable. Copy your unsent notes before closing.";
    case "throttled": return "Saving is delayed. Your changes remain here and will retry.";
    case "newer_schema": return "This Canvas needs a newer app. Copy your unsent notes before closing or updating.";
    case "invalid": return "Canvas changes couldn’t be saved. Copy your unsent notes before closing.";
    default: return "Canvas changes haven’t saved. Your changes remain here and will retry.";
  }
}

/** Pending changes are never described as saved. A failure keeps a way back
 * to the original action, without exposing server details or note content. */
export default function CanvasSaveNotice(props: {
  status: CanvasSaveStatus;
  onRetry: () => void;
  onCopy?: () => void;
  conflict?: { token: number; notes: CanvasConflictNote[] };
  onResolve?: (token: number, choices: Readonly<Record<string, "mine" | "saved">>) => Promise<boolean>;
}) {
  const [choices, setChoices] = createSignal<Record<string, "mine" | "saved">>({});
  const [resolving, setResolving] = createSignal(false);
  createEffect(() => {
    props.conflict?.token;
    setChoices({});
  });
  const resolve = async () => {
    const review = props.conflict;
    if (!review || !props.onResolve || review.notes.some((note) => !choices()[note.key])) return;
    setResolving(true);
    try { await props.onResolve(review.token, choices()); }
    finally { setResolving(false); }
  };
  return <Show when={props.status.state !== "saved"}>
    <div class="artifact-canvas-notice">
      <span role="status" aria-live="polite">{props.status.reason === "conflict" && props.conflict?.notes.length
        ? "These notes changed in another window. Compare both versions and choose what to keep for each note."
        : props.status.state === "error" ? explanation(props.status.reason) : "Saving Canvas changes…"}</span>
      <Show when={props.status.reason === "conflict" && props.conflict}>
        {(review) => <div class="canvas-conflict-review" aria-live="off">
          <Show when={review().notes.length === 0}>
            <p>There is no note-only resolution for this Canvas change. Copy your notes, then retry after resolving the Canvas elsewhere.</p>
          </Show>
          <For each={review().notes}>{(note) => <section class="canvas-conflict-note">
            <h3>{note.title}</h3>
            <div class="canvas-conflict-versions">
              <div><strong>Your changes</strong><pre>{note.local || "(empty note)"}</pre></div>
              <div><strong>Saved copy</strong><pre>{note.saved || "(empty note)"}</pre></div>
            </div>
            <div class="settings-actions">
              <button type="button" class="settings-button" aria-pressed={choices()[note.key] === "mine"} disabled={resolving()} onClick={() => setChoices((current) => ({ ...current, [note.key]: "mine" }))}>Keep my changes</button>
              <button type="button" class="settings-button" aria-pressed={choices()[note.key] === "saved"} disabled={resolving()} onClick={() => setChoices((current) => ({ ...current, [note.key]: "saved" }))}>Use saved copy</button>
            </div>
          </section>}</For>
          <Show when={review().notes.length > 0}>
            <button type="button" class="artifact-canvas-btn" disabled={resolving() || review().notes.some((note) => !choices()[note.key])} onClick={() => void resolve()}>
              {resolving() ? "Saving your choices…" : "Save these choices"}
            </button>
          </Show>
        </div>}
      </Show>
      <Show when={props.status.recovery === "unavailable" && props.status.reason !== "recovery" && props.status.reason !== "recovery_cleanup"}>
        <span>Recovery on this device is unavailable. Keep this window open or copy your notes until saving succeeds.</span>
      </Show>
      <Show when={props.status.recovery === "ready"}>
        <span>Recovery is kept in this tab. After a reload, connect and sign in to restore it.</span>
      </Show>
      <Show when={(props.status.reason === "conflict" || props.status.reason === "recovery" || props.status.recovery === "unavailable") && props.onCopy}>
        <button type="button" class="artifact-canvas-btn" onClick={() => props.onCopy?.()}>Copy unsent notes</button>
      </Show>
      <Show when={props.status.state === "error"}>
        <button type="button" class="artifact-canvas-btn" onClick={props.onRetry}>Retry saving</button>
      </Show>
    </div>
  </Show>;
}
