import { createEffect, createResource, createSignal, For, on, onCleanup, Show } from "solid-js";
import { activeAgentId, agentOpening, agentsEpoch, openingAgentId, backend, isRunning, sessions, settingsOpen, setAgentCreateOpen, narrowViewport, inboxOpen, inboxUnread, libraryOpen, setInboxOpen, setLibraryOpen, setAutomationsOpen, automationsOpen, setNotice, setSearchOpen, setSettingsOpen, setSettingsScope, setShowShortcuts, setSidebarOpen, topicsChanged } from "../store";
import { openAgentChat, openTopic } from "../App";
import * as api from "../api";
import { host } from "../host";
import AgentMark from "./AgentMark";
import { sortByRecent } from "../agentRecents";
import Icon from "./Icon";
import Skeleton from "./Skeleton";
import { historyPages, itemsOf, presentationOf } from "../store";
import { latestWorkLabel, turnDisplayLabel } from "../turnPairing";
import { stripControlScaffolding } from "../structured";

function AgentTopicList(props: { agent: Pick<api.Agent, "id" | "name" | "revision" | "character" | "animation" | "voice"> }) {
  const [topics, setTopics] = createSignal<api.AgentTopic[]>([]);
  const [next, setNext] = createSignal<string | null>(null);
  const [error, setError] = createSignal("");
  const [loading, setLoading] = createSignal(false);
  const [showArchived, setShowArchived] = createSignal(false);
  const [expanded, setExpanded] = createSignal(false);
  const [loadedOnce, setLoadedOnce] = createSignal(false);
  let requestVersion = 0;
  createEffect(on(() => backend().cwd, () => {
    requestVersion++;
    setLoading(false); setError("");
    setTopics([]); setNext(null); setLoadedOnce(false);
    if (expanded()) void load();
  }));
  const load = async (before?: string) => {
    if (loading()) return;
    const cwd = backend().cwd;
    const agentId = props.agent.id;
    const request = ++requestVersion;
    const epoch = topicsChanged();
    setLoading(true);
    try {
      const page = await api.listAgentTopics(agentId, before);
      if (cwd !== backend().cwd || agentId !== props.agent.id || request !== requestVersion) return;
      setTopics((current) => {
        const combined = before ? [...current, ...page.topics] : page.topics;
        const unique = new Map<string, api.AgentTopic>();
        for (const topic of combined) unique.set(`${topic.session_id}/${topic.id}`, topic);
        return [...unique.values()];
      });
      setNext(page.next ?? null);
      setError("");
      setLoadedOnce(true);
    } catch (e) { if (cwd === backend().cwd && agentId === props.agent.id && request === requestVersion) setError(e instanceof Error ? e.message : String(e)); }
    finally { if (request === requestVersion) { setLoading(false); if (expanded() && topicsChanged() !== epoch) void load(); } }
  };
  createEffect(on(topicsChanged, () => { if (expanded() && loadedOnce() && !loading()) void load(); }, { defer: true }));
  return <details class="sb-topics" onToggle={(event) => { const open = (event.currentTarget as HTMLDetailsElement).open; setExpanded(open); if (open && !loadedOnce() && !loading()) void load(); }}>
    <summary>Topics</summary>
    <Show when={error()}><p class="sb-topics-error" role="alert">Could not load topics. <button type="button" onClick={() => void load()}>Try again</button></p></Show>
    <For each={topics().filter((topic) => !topic.archived).sort((a, b) => Number(b.pinned) - Number(a.pinned))}>{(topic) => <button type="button" class="sb-topic-link" title={topic.title} onClick={() => void openTopic(topic.session_id, topic.id, props.agent)}><Show when={topic.pinned}><Icon name="pin" size={12} /></Show><span>{topic.title}</span></button>}</For>
    <Show when={topics().some((topic) => topic.archived)}>
      <details class="sb-topics-archived"><summary onClick={(event) => { event.stopPropagation(); setShowArchived(true); }}>Archived topics</summary>
        <Show when={showArchived()}><For each={topics().filter((topic) => topic.archived)}>{(topic) => <button type="button" class="sb-topic-link archived" title={topic.title} onClick={() => void openTopic(topic.session_id, topic.id, props.agent)}>{topic.title}</button>}</For></Show>
      </details>
    </Show>
    <Show when={topics().length === 0 && !loading() && !error()}><span class="sb-topic-empty">No topics yet</span></Show>
    <Show when={next()}><button type="button" class="sb-topic-more" disabled={loading()} onClick={() => void load(next()!)}>{loading() ? "Loading…" : "More topics"}</button></Show>
  </details>;
}

export default function Sidebar() {
  const [agents, setAgents] = createSignal<api.Agent[]>([]);
  const [loaded, setLoaded] = createSignal(false);
  const [error, setError] = createSignal("");

  createEffect(() => {
    const cwd = backend().cwd;
    const ready = backend().ready;
    const editing = settingsOpen();
    setAgents([]);
    setLoaded(false);
    if (!ready || editing) return;
    let disposed = false;
    const refresh = async () => {
      try {
        const result = await api.listAgents();
        if (!disposed && cwd === backend().cwd) {
          // A new list with the same agents would rebuild every row (and
          // redraw every character) on each 10 s refresh.
          if (!loaded() || JSON.stringify(result.agents) !== JSON.stringify(agents())) setAgents(result.agents);
          setError("");
          setLoaded(true);
        }
      } catch (e) { if (!disposed) { setError(e instanceof Error ? e.message : String(e)); setLoaded(true); } }
    };
    void refresh();
    const timer = window.setInterval(refresh, 10000);
    createEffect(on(agentsEpoch, () => void refresh(), { defer: true }));
    onCleanup(() => { disposed = true; window.clearInterval(timer); });
  });

  // Paused/archived agents stay out of the everyday switcher — manage them
  // from Your agents, which shows every lifecycle state.
  const shown = () => sortByRecent(
    agents().filter((p) => (p.lifecycle ?? "active") === "active")
  );

  const agentIsRunning = (id: string) => sessions().some((s) => s.agent?.id === id && isRunning(s.session_id));
  const AgentRow = (props: { id: string; name: string; character?: api.Agent["character"]; motion?: api.Agent["animation"]; profile?: api.Agent }) => {
    const recent = () => sessions().find(s => s.agent?.id === props.id && isRunning(s.session_id))
      ?? sessions().find(s => s.agent?.id === props.id);
    // One bounded recent page, refreshed only when the session changes. Keep
    // this preview separate from the reader's lazily loaded history window.
    const [preview] = createResource(() => {
      const session = recent();
      if (session && !historyPages[session.session_id]?.newer
        && itemsOf(session.session_id).some(item => item.kind === "user")) return false;
      return session ? `${session.session_id}:${session.updated_at ?? ""}` : false;
    }, async () => {
      const session = recent();
      if (!session) return "";
      try { return latestWorkLabel((await api.conversationSnapshot(session.session_id)).presentation); }
      catch { return ""; }
    });
    const label = () => {
      const session = recent();
      if (!session) return "";
      if (!historyPages[session.session_id]?.newer) {
        const timeline = presentationOf(session.session_id);
        const request = itemsOf(session.session_id).findLast(item => item.kind === "user");
        if (request?.kind === "user") return turnDisplayLabel(request.entryId, timeline) || stripControlScaffolding(request.text);
        const title = latestWorkLabel(timeline);
        if (title) return title;
      }
      return preview.loading ? "" : preview() ?? "";
    };
    const status = () => label().replace(/\s+/g, " ").trim() || (agentIsRunning(props.id) ? "Working…" : "");
    return (
    <div class="sb-agent-group">
    <button type="button" class="sb-agent-item" classList={{ active: activeAgentId() === props.id, opening: openingAgentId() === props.id }}
      aria-busy={openingAgentId() === props.id} aria-current={activeAgentId() === props.id ? "page" : undefined}
      disabled={agentOpening()} onClick={() => void openAgentChat(props.id)}>
      <AgentMark character={props.character} motion={props.motion} size={28} state={agentIsRunning(props.id) ? "working" : "idle"} />
      <span class="sb-agent-text">
        <span class="sb-agent-name">{props.name}</span>
        <Show when={status()}><span class="sb-agent-sub" title={`${agentIsRunning(props.id) ? "Working on" : "Last worked on"}: ${status()}`}>{status()}</span></Show>
      </span>
    </button>
    <AgentTopicList agent={props.profile ?? { id: props.id, name: props.name, revision: 1, character: props.character ?? ("vak" as api.Agent["character"]), animation: props.motion ?? "subtle", voice: "default" }} />
    </div>
    );
  };

  return (
    <aside class="sidebar">
      <div class="window-drag-strip" data-titlebar aria-hidden="true" />
      <div class="sb-head" data-titlebar>
        <div class="brand" aria-label="Vakyartha">
          <span class="brand-mark"><img src={`${import.meta.env.BASE_URL}assets/brand/songbird-colour.svg`} alt="" /></span>
          <span class="brand-wordmark" aria-hidden="true" />
        </div>
        <div class="sb-head-actions">
          <button type="button" class="icon-button has-tooltip" data-tooltip="Hide sidebar ⌘B" aria-label="Hide sidebar" onClick={() => setSidebarOpen(false)}><Icon name="sidebar" /></button>
        </div>
      </div>

      <button type="button" class="sb-search-row" onClick={() => setSearchOpen(true)}><Icon name="search" /><span>Search</span><kbd>⌘K</kbd></button>
      <button type="button" class="sb-search-row" classList={{ active: libraryOpen() && !inboxOpen() }} onClick={() => { setInboxOpen(false); setAutomationsOpen(false); setLibraryOpen(true); if (narrowViewport()) setSidebarOpen(false); }}><Icon name="layers" /><span>Library</span></button>
      <button type="button" class="sb-search-row" classList={{ active: inboxOpen() }} aria-label={inboxUnread() ? `Inbox, ${inboxUnread()} unread` : "Inbox"} onClick={() => { setLibraryOpen(false); setAutomationsOpen(false); setInboxOpen(true); if (narrowViewport()) setSidebarOpen(false); }}><Icon name="bell" /><span>Inbox</span><Show when={inboxUnread() > 0}><kbd>{inboxUnread()}</kbd></Show></button>
      <button type="button" class="sb-search-row" classList={{ active: automationsOpen() }} onClick={() => { setInboxOpen(false); setLibraryOpen(false); setAutomationsOpen(true); if (narrowViewport()) setSidebarOpen(false); }}><Icon name="timer" /><span>Automations</span></button>

      <div class="sb-section-row"><span class="sb-section-title">Agents</span></div>
      <nav class="sb-agent-list" aria-label="Agents">
        <AgentRow id="vak" name="Vakyartha" character="vak" />
        <Show when={loaded()} fallback={
          <Skeleton shapes={["row", "row"]} class="sb-agent-skeleton" label="Loading your agents" />
        }>
          <For each={shown()}>{(profile) => <AgentRow id={profile.id} name={profile.name} character={profile.character} motion={profile.animation} profile={profile} />}</For>
        </Show>
        <Show when={error()}><p class="worker-error" role="alert">Could not load agents: {error()} Retrying automatically.</p></Show>
        <button type="button" class="sb-agent-item sb-new-agent" onClick={() => setAgentCreateOpen(true)}><Icon name="add" /><span>New agent</span></button>
      </nav>

      <div class="sidebar-footer">
        <button type="button" class="sidebar-settings" onClick={() => { setSettingsScope("workspace"); setSettingsOpen(true); }}><Icon name="gear" /><span>Settings</span><kbd>⌘,</kbd></button>
        <details class="sidebar-more" data-menu>
          <summary aria-label="Help and account"><Icon name="more" /></summary>
          <div class="sidebar-more-menu" role="menu">
            <button type="button" role="menuitem" onClick={(event) => { (event.currentTarget.closest("details") as HTMLDetailsElement).open = false; setShowShortcuts(true); }}><Icon name="tune" />Keyboard shortcuts</button>
            <Show when={host.logout}><button type="button" role="menuitem" onClick={() => void host.logout?.().then(() => window.location.reload()).catch((error) => setNotice({ kind: "error", text: `Could not sign out: ${error instanceof Error ? error.message : String(error)}` }))}><Icon name="lock" />Sign out</button></Show>
          </div>
        </details>
      </div>
    </aside>
  );
}
