import { createEffect, createSignal, For, on, onCleanup, Show } from "solid-js";
import { activeAgentId, agentOpening, agentsEpoch, openingAgentId, backend, isRunning, sessions, settingsOpen, setAgentCreateOpen, narrowViewport, inboxOpen, inboxUnread, libraryOpen, setInboxOpen, setLibraryOpen, setTasksOpen, setNotice, setSearchOpen, setSettingsOpen, setSettingsScope, setShowShortcuts, setSidebarOpen } from "../store";
import { openAgentChat } from "../App";
import * as api from "../api";
import { host } from "../host";
import AgentMark from "./AgentMark";
import { sortByRecent } from "../agentRecents";
import Icon from "./Icon";
import Skeleton from "./Skeleton";

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
  // One quiet line under each agent: what it is doing, else its latest conversation.
  const agentStatus = (id: string) => agentIsRunning(id)
    ? "Working…"
    : sessions().find((s) => s.agent?.id === id && s.title)?.title ?? "";

  const AgentRow = (props: { id: string; name: string; character?: string; motion?: api.Agent["animation"] }) => (
    <button type="button" class="sb-agent-item" classList={{ active: activeAgentId() === props.id, opening: openingAgentId() === props.id }}
      aria-busy={openingAgentId() === props.id} aria-current={activeAgentId() === props.id ? "page" : undefined}
      disabled={agentOpening()} onClick={() => void openAgentChat(props.id)}>
      <AgentMark character={props.character} motion={props.motion} size={28} state={agentIsRunning(props.id) ? "working" : "idle"} />
      <span class="sb-agent-text">
        <span class="sb-agent-name">{props.name}</span>
        <Show when={agentStatus(props.id)}><span class="sb-agent-sub">{agentStatus(props.id)}</span></Show>
      </span>
    </button>
  );

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
      <button type="button" class="sb-search-row" classList={{ active: libraryOpen() && !inboxOpen() }} onClick={() => { setInboxOpen(false); setLibraryOpen(true); if (narrowViewport()) setSidebarOpen(false); }}><Icon name="layers" /><span>Library</span></button>
      <button type="button" class="sb-search-row" classList={{ active: inboxOpen() }} aria-label={inboxUnread() ? `Inbox, ${inboxUnread()} unread` : "Inbox"} onClick={() => { setLibraryOpen(false); setInboxOpen(true); if (narrowViewport()) setSidebarOpen(false); }}><Icon name="bell" /><span>Inbox</span><Show when={inboxUnread() > 0}><kbd>{inboxUnread()}</kbd></Show></button>
      <button type="button" class="sb-search-row" onClick={() => { setTasksOpen(true); if (narrowViewport()) setSidebarOpen(false); }}><Icon name="timer" /><span>Automations</span></button>

      <div class="sb-section-row"><span class="sb-section-title">Agents</span></div>
      <nav class="sb-agent-list" aria-label="Agents">
        <AgentRow id="vak" name="Vakyartha" character="vak" />
        <Show when={loaded()} fallback={
          <Skeleton shapes={["row", "row"]} class="sb-agent-skeleton" label="Loading your agents" />
        }>
          <For each={shown()}>{(profile) => <AgentRow id={profile.id} name={profile.name} character={profile.character} motion={profile.animation} />}</For>
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
