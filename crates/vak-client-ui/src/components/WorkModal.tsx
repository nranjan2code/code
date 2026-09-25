import { For, Show, createResource, createSignal } from "solid-js";
import * as api from "../api";
import { activeId, setWorkOpen, workOpen } from "../store";
import Sheet from "./Sheet";

export default function WorkModal() {
  const [commandError, setCommandError] = createSignal<string | null>(null);
  const [work, { refetch }] = createResource(
    () => (workOpen() ? activeId() : null),
    (id) => api.work(id!),
  );
  const command = async (value: Record<string, unknown>) => {
    const id = activeId();
    if (!id) return;
    setCommandError(null);
    try {
      await api.workCommand(id, value);
      await refetch();
    } catch (error) {
      setCommandError(error instanceof Error ? error.message : String(error));
    }
  };
  return <Show when={workOpen()}>
    <Sheet size="wide" class="work-modal" title="Background tasks" onClose={() => setWorkOpen(false)}>
        <Show when={commandError()}>{(message) => <div class="error-state" role="alert"><strong>Could not update managed work</strong><p>{message()}</p></div>}</Show>
        <Show when={!work.loading} fallback={<div class="empty">Loading background tasks…</div>}>
          <Show when={!work.error} fallback={<div class="error-state" role="alert"><strong>Managed work unavailable</strong><p>{String(work.error)}</p><button type="button" class="ghost small" onClick={() => void refetch()}>Retry</button></div>}>
          <Show when={work()} fallback={<div class="empty">This session has no managed work contract.</div>}>
            {(projection) => <><p class="work-objective">{projection().contract.objective}</p><p class="dim">{projection().status} · <span class="mono">{projection().contract.contract_id}</span></p><div class="work-items"><For each={Object.entries(projection().items) as Array<[string, { status: string; attempt: number; blocker?: string }]>}>{([id, item]) => <div class="work-item"><span class={`status-dot status-${item.status}`} /><span class="mono work-item-id">{id}</span><span class="dim">{item.status} · attempt {item.attempt}</span><Show when={item.status === "failed" || item.status === "interrupted"}><button type="button" class="ghost small" onClick={() => void command({ operation: "retry", item_id: id, reason: "operator retry" })}>Retry</button></Show></div>}</For></div><Show when={projection().status === "blocked"}><button type="button" class="primary small" onClick={() => void command({ operation: "resume", reason: "operator resumed contract" })}>Resume contract</button></Show></>}
          </Show>
          </Show>
        </Show>
    </Sheet>
  </Show>;
}
