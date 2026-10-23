import { createEffect, createSignal, For, Show } from "solid-js";
import * as api from "../api";
import { activeTopic, setActiveTopic, setNotice, setTopicsChanged, topicsChanged, organisationOf, acceptOrganisation, refreshOrganisation, presentationOf } from "../store";
import { openTopic } from "../App";

export default function TopicsPanel(props: { sessionId: string }) {
  const [state, setState] = createSignal<api.SessionTopics | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [title, setTitle] = createSignal("");
  const [renameId, setRenameId] = createSignal<string | null>(null);
  const [renameTitle, setRenameTitle] = createSignal("");
  const [renameTurn, setRenameTurn] = createSignal<string | null>(null);
  const [showArchived, setShowArchived] = createSignal(false);
  const [error, setError] = createSignal("");

  const refresh = async () => {
    const requestedSession = props.sessionId;
    try {
      const next = await api.listSessionTopics(requestedSession);
      if (props.sessionId === requestedSession) { setState(next); setError(""); }
    }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };
  createEffect(() => { props.sessionId; setState(null); void refresh(); });
  createEffect(() => { topicsChanged(); const latest = organisationOf(props.sessionId); if (latest) setState(latest); });
  const change = async (action: api.TopicAction): Promise<(api.SessionTopics & { created_topic_id?: string }) | null> => {
    const revision = state()?.revision;
    if (revision == null) return null;
    setBusy(true);
    try {
      const next = await api.updateSessionTopics(props.sessionId, action, revision);
      setState(next);
      acceptOrganisation(props.sessionId, next);
      void refreshOrganisation(props.sessionId);
      setTopicsChanged((count) => count + 1);
      const selected = activeTopic();
      if (selected?.sessionId === props.sessionId) {
        const topic = next.topics.find((item) => item.id === selected.topicId);
        if (topic) setActiveTopic({ ...selected, title: topic.title, archived: topic.archived });
      }
      setError("");
      return next;
    }
    catch (e) {
      if (e instanceof api.ApiError && e.status === 409) {
        await refresh();
        setError("Topics changed elsewhere. The latest choices are shown; review before trying again.");
      } else {
        const message = e instanceof Error ? e.message : String(e);
        setError(message);
        setNotice({ kind: "error", text: `Could not update topics: ${message}` });
      }
      return null;
    }
    finally { setBusy(false); }
  };
  const create = async (event: SubmitEvent) => {
    event.preventDefault();
    const value = title().trim();
    if (!value || busy()) return;
    const next = await change({ action: "create", title: value });
    if (next) {
      setTitle("");
      if (next.created_topic_id) {
        const created = next.topics.find((topic) => topic.id === next.created_topic_id);
        if (created) { setRenameTitle(created.title); setRenameId(created.id); }
      }
    }
  };
  const visible = () => (state()?.topics ?? []).filter((topic) => showArchived() || !topic.archived);

  return <section class="topics-panel" aria-label="Topics">
    <div class="topics-panel-head">
      <div><p>Browse topics and name requests from this conversation.</p></div>
      <label class="topics-auto"><input type="checkbox" checked={state()?.automatic ?? true} disabled={!state() || busy()} onChange={(event) => void change({ action: "automatic", automatic: event.currentTarget.checked })} />Group related requests automatically</label>
    </div>
    <Show when={error()}><p class="topics-error" role="alert">{error()}</p></Show>
    <Show when={state()} fallback={<p class="topics-empty">Loading topics…</p>}>
      <p class="topics-empty"><Show when={state()?.organisation_pending} fallback={state()?.organisation_failed ? "Some request names or topics need another try." : "Names and topics are updated after each answer."}>Naming requests and updating topics…</Show></p>
      <button type="button" disabled={busy() || state()?.organisation_pending} onClick={() => {
        const turns = [...new Set((presentationOf(props.sessionId)?.items ?? []).flatMap(item => item.role === "user" && item.turn_id ? [item.turn_id] : []))];
        if (turns.length) void change({ action: "organise", turn_ids: turns, retry: !!state()?.organisation_failed });
      }}>{state()?.organisation_failed ? "Try again" : "Name and group requests shown here"}</button>
      <For each={(state()?.turn_names ?? []).filter(name => name.title)}>{(name) => <div class="topics-row">
        <Show when={renameTurn() === name.turn_id} fallback={<><strong>{name.title}</strong><button type="button" disabled={busy()} onClick={() => { setRenameTurn(name.turn_id); setRenameTitle(name.title ?? ""); }}>Rename request</button></>}>
          <form class="topics-rename" onSubmit={(event) => { event.preventDefault(); if (renameTitle().trim()) void change({action:"rename_turn", turn_id:name.turn_id, title:renameTitle().trim()}).then(result => { if (result) setRenameTurn(null); }); }}>
            <input aria-label="Request name" value={renameTitle()} onInput={event => setRenameTitle(event.currentTarget.value)} />
            <button type="submit" disabled={busy() || !renameTitle().trim()}>Save</button>
            <button type="button" onClick={() => setRenameTurn(null)}>Cancel</button>
          </form>
        </Show>
      </div>}</For>
      <form class="topics-create" onSubmit={create}>
        <input aria-label="New topic name" placeholder="Name a topic" value={title()} onInput={(event) => setTitle(event.currentTarget.value)} />
        <button type="submit" disabled={!title().trim() || busy()}>Add</button>
      </form>
      <div class="topics-list" aria-label="Conversation topics">
        <Show when={visible().length} fallback={<p class="topics-empty">Topics chosen for this conversation will appear here.</p>}>
          <For each={visible()}>{(topic) => <div class="topics-row">
            <Show when={renameId() === topic.id} fallback={<strong>{topic.title}</strong>}>
              <form class="topics-rename" onSubmit={(event) => { event.preventDefault(); const value = renameTitle().trim(); if (value) void change({ action: "rename", topic_id: topic.id, title: value }).then(() => setRenameId(null)); }}>
                <input aria-label="Topic name" value={renameTitle()} onInput={(event) => setRenameTitle(event.currentTarget.value)} />
                <button type="submit" disabled={busy()}>Save</button>
              </form>
            </Show>
            <div class="topics-row-actions">
              <button type="button" disabled={busy()} aria-label={topic.pinned ? `Unpin ${topic.title}` : `Pin ${topic.title}`} onClick={() => void change({ action: "pin", topic_id: topic.id, pinned: !topic.pinned })}>{topic.pinned ? "Unpin" : "Pin"}</button>
              <button type="button" disabled={busy()} onClick={() => { setRenameId(topic.id); setRenameTitle(topic.title); }}>Rename</button>
              <button type="button" disabled={busy()} onClick={() => void change({ action: topic.archived ? "reopen" : "archive", topic_id: topic.id })}>{topic.archived ? "Reopen" : "Archive"}</button>
            </div>
          </div>}</For>
        </Show>
      </div>
      <Show when={(state()?.topics ?? []).some((topic) => topic.archived)}>
        <button class="topics-archived-toggle" type="button" aria-expanded={showArchived()} onClick={() => setShowArchived(!showArchived())}>{showArchived() ? "Hide archived topics" : "Show archived topics"}</button>
      </Show>
      <Show when={(state()?.placements.length ?? 0) > 0}>
        <div class="topics-placements">
          <h3>Answer placement</h3>
          <For each={state()?.placements ?? []}>{(placement) => <div class="topics-placement">
            <div class="topics-placement-label"><strong>{placement.outcome || "This request"}</strong><span>{placement.source === "model" ? "Organized by Vakyartha" : "Set by you"}</span></div>
            <div class="topics-placement-links"><For each={placement.topic_ids}>{(topicId) => { const topic = () => state()?.topics.find((item) => item.id === topicId); return <button type="button" disabled={!topic()} onClick={() => void openTopic(props.sessionId, topicId, undefined, placement.turn_id)}>Open {topic()?.title ?? "topic"} here</button>; }}</For></div>
            <div class="topics-checks" role="group" aria-label={`Topics for ${placement.outcome || "this request"}`}>
              <For each={(state()?.topics ?? []).filter((topic) => !topic.archived)}>{(topic) => <label><input type="checkbox" checked={placement.topic_ids.includes(topic.id)} disabled={busy()} onChange={(event) => {
                const topicIds = event.currentTarget.checked ? [...placement.topic_ids, topic.id] : placement.topic_ids.filter((id) => id !== topic.id);
                void change({ action: "assign", turn_id: placement.turn_id, outcome: placement.outcome, whole_turn: placement.whole_turn, topic_ids: topicIds });
              }} />{topic.title}</label>}</For>
            </div>
          </div>}</For>
        </div>
      </Show>
    </Show>
  </section>;
}
