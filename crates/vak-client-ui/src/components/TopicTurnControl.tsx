import { createSignal, For, Show } from "solid-js";
import * as api from "../api";
import { setNotice, setTopicsChanged } from "../store";

/** A correction affordance attached to the readable source request in the chat. */
export default function TopicTurnControl(props: { sessionId: string; turnId: string; request: string }) {
  const [open, setOpen] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [snapshot, setSnapshot] = createSignal<api.SessionTopics | null>(null);
  const [selected, setSelected] = createSignal<string[]>([]);
  const [error, setError] = createSignal("");

  const show = async () => {
    const wasOpen = open();
    setOpen(!wasOpen);
    if (wasOpen) return;
    setLoading(true);
    try {
      const next = await api.listSessionTopics(props.sessionId);
      setSnapshot(next);
      const current = next.placements.filter((placement) => placement.turn_id === props.turnId);
      setSelected([...new Set(current.flatMap((placement) => placement.topic_ids))]);
      setError("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally { setLoading(false); }
  };
  const save = async (topicIds: string[]) => {
    const current = snapshot();
    if (!current || busy()) return;
    setBusy(true);
    try {
      const next = await api.updateSessionTopics(props.sessionId, {
        action: "assign", turn_id: props.turnId, outcome: "This request", whole_turn: true, topic_ids: topicIds,
      }, current.revision);
      setSnapshot(next);
      setTopicsChanged((count) => count + 1);
      setSelected(topicIds);
      setOpen(false);
      setError("");
      setNotice({ kind: "info", text: topicIds.length ? "Request organized." : "Request left unassigned." });
    } catch (cause) {
      if (cause instanceof api.ApiError && cause.status === 409) {
        try {
          const latest = await api.listSessionTopics(props.sessionId);
          setSnapshot(latest);
          const current = latest.placements.filter((placement) => placement.turn_id === props.turnId);
          setSelected([...new Set(current.flatMap((placement) => placement.topic_ids))]);
        } catch { /* keep the conflict visible */ }
        setError("Topics changed elsewhere. The latest choices are shown; review before saving again.");
      } else {
        const message = cause instanceof Error ? cause.message : String(cause);
        setError(message);
        setNotice({ kind: "error", text: `Could not organize this request: ${message}` });
      }
    } finally { setBusy(false); }
  };

  const available = () => (snapshot()?.topics ?? []).filter((topic) => !topic.archived);
  return <div class="topic-turn-control">
    <button type="button" class="topic-turn-button" aria-expanded={open()} aria-label={`Organize request: ${props.request}`} onClick={() => void show()}>
      Organize
    </button>
    <Show when={open()}>
      <div class="topic-turn-popover" role="group" aria-label="Organize this request">
        <strong>Place this request in Topics</strong>
        <Show when={loading()}><span>Loading topics…</span></Show>
        <Show when={error()}><span class="topics-error" role="alert">{error()}</span></Show>
        <For each={available()}>{(topic) => <label><input type="checkbox" checked={selected().includes(topic.id)} disabled={busy() || loading()} onChange={(event) => {
          const next = event.currentTarget.checked ? [...selected(), topic.id] : selected().filter((id) => id !== topic.id);
          setSelected(next);
        }} />{topic.title}</label>}</For>
        <Show when={!loading() && available().length === 0}><span>Create a Topic in the Topics panel first.</span></Show>
        <div class="topic-turn-actions">
          <button type="button" disabled={busy() || loading() || !snapshot()} onClick={() => void save(selected())}>Save</button>
          <button type="button" disabled={busy() || loading() || !snapshot()} onClick={() => void save([])}>Leave unassigned</button>
          <button type="button" onClick={() => setOpen(false)}>Close</button>
        </div>
      </div>
    </Show>
  </div>;
}
