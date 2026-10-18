import { Show } from "solid-js";

import type { CanvasSaveStatus } from "../../canvasSyncState";

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
export default function CanvasSaveNotice(props: { status: CanvasSaveStatus; onRetry: () => void; onCopy?: () => void }) {
  return <Show when={props.status.state !== "saved"}>
    <div class="artifact-canvas-notice" role="status" aria-live="polite">
      <span>{props.status.state === "error" ? explanation(props.status.reason) : "Saving Canvas changes…"}</span>
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
