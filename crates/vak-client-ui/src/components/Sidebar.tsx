import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import { activeAgentId, agentOpening, backend, isRunning, sessions, settingsOpen, setAgentCreateOpen, setSettingsOpen, setSettingsScope, setShowShortcuts, setSidebarOpen } from "../store";
import { openAgentChat } from "../App";
import * as api from "../api";
import { host } from "../host";
import AgentMark from "./AgentMark";
import { sortByRecent } from "../agentRecents";
import Icon from "./Icon";

export default function Sidebar() {
  const [agents, setAgents] = createSignal<api.Agent[]>([]);
  const [loaded, setLoaded] = createSignal(false);
  const [error, setError] = createSignal("");
  const [query, setQuery] = createSignal("");
  const [searching, setSearching] = createSignal(false);

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
        if (!disposed && cwd === backend().cwd) { setAgents(result.agents); setError(""); setLoaded(true); }
      } catch (e) { if (!disposed) { setError(e instanceof Error ? e.message : String(e)); setLoaded(true); } }
    };
    void refresh();
    const timer = window.setInterval(refresh, 10000);
    onCleanup(() => { disposed = true; window.clearInterval(timer); });
  });

  // Paused/archived agents stay out of the everyday switcher — manage them
  // from Fleet Roster, which shows every lifecycle state.
  const shown = () => sortByRecent(
    agents().filter((p) => (p.lifecycle ?? "active") === "active" && p.name.toLocaleLowerCase().includes(query().toLocaleLowerCase()))
  );

  const agentIsRunning = (id: string) => sessions().some((s) => s.agent?.id === id && isRunning(s.session_id));

  return (
    <aside class="sidebar">
      <div class="sb-head">
        <div class="brand" aria-label="Vak">
          <span class="brand-mark"><img src={`${import.meta.env.BASE_URL}vak-icon.png`} alt="" /></span>
          <span>vak</span>
        </div>
        <div class="sb-head-actions">
          <button type="button" class="icon-button subtle" aria-label="Search agents" aria-expanded={searching()} onClick={() => setSearching(!searching())}><Icon name="search" /></button>
          <button type="button" class="icon-button subtle" aria-label="Hide sidebar" onClick={() => setSidebarOpen(false)}><Icon name="sidebar" /></button>
        </div>
      </div>

      <div class="sb-section-row"><span class="sb-section-title">Your agents</span></div>
      <Show when={searching()}><div class="sb-search-wrap"><input class="sb-search" type="search" aria-label="Search agents" placeholder="Find an agent" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} /></div></Show>

      <nav class="sb-agent-list" aria-label="Agents">
        <button type="button" class="sb-agent-item" classList={{active: activeAgentId() === "vak"}} aria-current={activeAgentId() === "vak" ? "page" : undefined} disabled={agentOpening()} onClick={() => void openAgentChat("vak")}><AgentMark character="vak" size={22} state={agentIsRunning("vak") ? "working" : "idle"} /><span>Vak</span></button>
        <Show when={loaded()} fallback={
          <div class="sb-agent-skeleton" aria-hidden="true"><span /><span /></div>
        }>
          <For each={shown()}>{(profile) =>
            <button type="button" class="sb-agent-item" classList={{active: activeAgentId() === profile.id}} aria-current={activeAgentId() === profile.id ? "page" : undefined} title={profile.name} disabled={agentOpening()} onClick={() => void openAgentChat(profile.id)}><AgentMark character={profile.character} motion={profile.animation} size={22} state={agentIsRunning(profile.id) ? "working" : "idle"} /><span>{profile.name}</span></button>
          }</For>
        </Show>
        <Show when={query() && loaded() && !shown().length}><p class="sb-empty">No matching agents.</p></Show>
        <Show when={error()}><p class="worker-error" role="alert">Could not load agents: {error()} Retrying automatically.</p></Show>
        <Show when={agentOpening()}><p role="status" class="sb-empty">Opening agent…</p></Show>
        <button type="button" class="sb-agent-item" onClick={() => setAgentCreateOpen(true)}><Icon name="add" /><span>Agent</span></button>
      </nav>

      <div class="sidebar-footer">
        <button type="button" class="sidebar-settings" onClick={() => { setSettingsScope("workspace"); setSettingsOpen(true); }}><Icon name="gear" /><span>Settings</span><kbd>⌘,</kbd></button>
        <Show when={host.logout}><button class="sidebar-help" aria-label="Sign out" onClick={() => void host.logout?.().then(() => window.location.reload())}><Icon name="shield" /></button></Show>
        <button class="sidebar-help" aria-label="Keyboard shortcuts" onClick={() => setShowShortcuts(true)}>?</button>
      </div>
    </aside>
  );
}
