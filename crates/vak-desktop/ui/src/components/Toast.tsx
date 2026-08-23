import { createEffect, onCleanup, Show } from "solid-js";
import { notice, setNotice } from "../store";
import Icon from "./Icon";

export default function Toast() {
  createEffect(() => {
    const current = notice();
    if (!current) return;
    const timer = window.setTimeout(() => setNotice(null), current.kind === "error" ? 7000 : 4000);
    onCleanup(() => window.clearTimeout(timer));
  });

  return (
    <Show when={notice()}>
      {(item) => (
        <div class="toast" classList={{ error: item().kind === "error" }} role="status" aria-live="polite">
          <span class="toast-icon"><Icon name={item().kind === "error" ? "close" : "check"} size={14} /></span>
          <span>{item().text}</span>
          <button aria-label="Dismiss notification" onClick={() => setNotice(null)}><Icon name="close" size={13} /></button>
        </div>
      )}
    </Show>
  );
}
