import { Show, createSignal } from "solid-js";
import Icon from "./Icon";
import Sheet from "./Sheet";

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
        <Sheet
          size="narrow"
          title={cfg().title}
          subtitle={cfg().description}
          onClose={props.onClose}
          busy={busy()}
          footer={<>
            <button type="button" class="btn" disabled={busy()} onClick={props.onClose}>{cfg().cancelLabel || "Cancel"}</button>
            <button type="button" class="btn" classList={{ primary: !cfg().isDanger, danger: cfg().isDanger }} disabled={busy()} onClick={() => void handleConfirm()}>{busy() ? "Working…" : (cfg().confirmLabel || "Confirm")}</button>
          </>}
        >
          <Show when={cfg().detail}>
            <div class="confirm-modal-detail">
              <Icon name="shield" size={14} />
              <span>{cfg().detail}</span>
            </div>
          </Show>
        </Sheet>
      )}
    </Show>
  );
}
