import { createEffect, For, onCleanup } from "solid-js";
import { notices, dismissNotice } from "../store";
import Icon from "./Icon";

function ToastItem(props: { item: { kind: "error" | "info"; text: string }; index: number }) {
  createEffect(() => {
    const timer = window.setTimeout(
      () => dismissNotice(props.index),
      // Long enough to read: a longer message stays longer.
      Math.max(props.item.kind === "error" ? 7000 : 4000, props.item.text.length * 70),
    );
    onCleanup(() => window.clearTimeout(timer));
  });

  return (
    <div class="toast" classList={{ error: props.item.kind === "error" }} role="status" aria-live="polite">
      <span class="toast-icon"><Icon name={props.item.kind === "error" ? "close" : "check"} size={14} /></span>
      <span>{props.item.text}</span>
      <button aria-label="Dismiss notification" onClick={() => dismissNotice(props.index)}><Icon name="close" size={13} /></button>
    </div>
  );
}

export default function Toast() {
  return (
    <div class="toast-stack">
      <For each={notices()}>
        {(item, index) => <ToastItem item={item} index={index()} />}
      </For>
    </div>
  );
}
