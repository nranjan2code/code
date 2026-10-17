import { createResource, createSignal, Show } from "solid-js";
import * as api from "../api";
import { openArtifactCanvas } from "../store";
import SchedulePicker from "./SchedulePicker";

/** What keeps a Library page current (docs/design/98 §12): the automation
 *  whose runs make its versions, or a way to make one that asks the page's
 *  request again on a schedule. */
export default function PageKeeper(props: { artifactId: string; title: string; request: string | null }) {
  const [keepers, { refetch }] = createResource(
    () => props.artifactId,
    (id) => api.listTriggers().then((list) => list.triggers.filter((trigger) => trigger.page === id)).catch(() => []),
  );
  const [zone] = createResource(() => api.getConfig().then((config) => config.owner?.timezone).catch(() => undefined));
  const [picking, setPicking] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const keep = async (repeat: import("../types").Repeat) => {
    const request = props.request;
    if (!request) return;
    setBusy(true);
    setError(null);
    try {
      await api.createTrigger({ name: props.title || "Page", repeat, action: { kind: "prompt", text: request }, page: props.artifactId });
      setPicking(false);
      await refetch();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Show
      when={keepers()?.[0]}
      fallback={
        <Show when={props.request}>
          <Show when={picking()} fallback={<button class="btn" onClick={() => setPicking(true)}>Keep current</button>}>
            <div class="library-keeper">
              <p class="library-note">Ask for this page again on a schedule; each answer becomes its next version.</p>
              <SchedulePicker zone={zone()} busy={busy()} label="Keep current" onPick={(repeat) => void keep(repeat)} />
              <button class="btn sm" type="button" onClick={() => setPicking(false)}>Cancel</button>
              <Show when={error()}><div class="worker-error" role="alert">{error()}</div></Show>
            </div>
          </Show>
        </Show>
      }
    >
      {(keeper) => (
        <button class="btn" onClick={() => openArtifactCanvas({ kind: "automation", title: keeper().name, taskId: keeper().id })}>
          Kept current by {keeper().name}
        </button>
      )}
    </Show>
  );
}
