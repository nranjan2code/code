import { createSignal, Show } from "solid-js";
import Icon from "./Icon";

export default function MessageActions(props: { text: string; role: "user" | "assistant" }) {
  const [copied, setCopied] = createSignal(false);
  const [copyFailed, setCopyFailed] = createSignal(false);
  const copy = async () => {
    setCopyFailed(false);
    try {
      await navigator.clipboard.writeText(props.text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopyFailed(true);
    }
  };
  const editPrompt = () => {
    window.dispatchEvent(new CustomEvent("vak:edit-prompt", { detail: { text: props.text } }));
  };

  return (
    <div class={`msg-actions msg-actions-${props.role}`} aria-label="Message actions">
      <button
        type="button"
        class="msg-action-btn"
        title={copied() ? "Copied" : copyFailed() ? "Copy failed" : "Copy text"}
        aria-label={copied() ? "Copied" : copyFailed() ? "Copy failed" : "Copy text"}
        onClick={copy}
      >
        <Show when={copied()} fallback={<Icon name="copy" size={11} />}>
          <Icon name="check" size={11} />
        </Show>
        <span>{copied() ? "Copied" : copyFailed() ? "Copy failed" : "Copy"}</span>
      </button>
      <Show when={props.role === "user"}>
        <button
          type="button"
          class="msg-action-btn"
          title="Edit prompt in composer"
          aria-label="Edit prompt in composer"
          onClick={editPrompt}
        >
          <Icon name="code" size={11} />
          <span>Edit</span>
        </button>
      </Show>
    </div>
  );
}

