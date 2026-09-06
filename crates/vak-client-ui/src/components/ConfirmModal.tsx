import { Show, createSignal, onCleanup, onMount } from "solid-js";
import Icon from "./Icon";

export interface ConfirmConfig {
  title: string;
  description: string;
  detail?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  isDanger?: boolean;
  onConfirm: () => Promise<void> | void;
}

export default function ConfirmModal(props: {
  config: ConfirmConfig | null;
  onClose: () => void;
}) {
  const [busy, setBusy] = createSignal(false);

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !busy()) {
      props.onClose();
    }
  };

  onMount(() => {
    window.addEventListener("keydown", onKeyDown);
    onCleanup(() => window.removeEventListener("keydown", onKeyDown));
  });

  const handleConfirm = async () => {
    if (!props.config || busy()) return;
    try {
      setBusy(true);
      await props.config.onConfirm();
      props.onClose();
    } finally {
      setBusy(false);
    }
  };

  return (
    <Show when={props.config}>
      {(cfg) => (
        <div class="modal-back" onClick={() => !busy() && props.onClose()}>
          <div
            class="modal confirm-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="confirm-modal-title"
            onClick={(e) => e.stopPropagation()}
          >
            <div class="confirm-modal-header">
              <div class="confirm-modal-icon" classList={{ danger: cfg().isDanger }}>
                <Icon name={cfg().isDanger ? "warning" : "shield"} size={20} />
              </div>
              <div class="confirm-modal-heading">
                <h3 id="confirm-modal-title">{cfg().title}</h3>
                <p class="confirm-modal-desc">{cfg().description}</p>
              </div>
              <button
                class="icon-button subtle confirm-modal-close"
                aria-label="Close"
                disabled={busy()}
                onClick={props.onClose}
              >
                <Icon name="close" size={14} />
              </button>
            </div>

            <Show when={cfg().detail}>
              <div class="confirm-modal-detail">
                <Icon name="shield" size={13} />
                <span>{cfg().detail}</span>
              </div>
            </Show>

            <div class="confirm-modal-actions">
              <button
                class="btn-subtle"
                disabled={busy()}
                onClick={props.onClose}
              >
                {cfg().cancelLabel || "Cancel"}
              </button>
              <button
                class="btn-action"
                classList={{ danger: cfg().isDanger }}
                disabled={busy()}
                onClick={handleConfirm}
              >
                {busy() ? "Working…" : (cfg().confirmLabel || "Confirm")}
              </button>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
}
