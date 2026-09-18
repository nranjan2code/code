import { createEffect, createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import { trapFocus } from "../focusTrap";
import {
  activeAgentId,
  activeId,
  agentPickerOpen,
  agentPickerTab,
  backend,
  sessions,
  setActiveId,
  setAgentCreateOpen,
  setAgentPickerOpen,
  setAgentPickerTab,
  workspaceSwitching,
} from "../store";
import { openAgentChat, refreshBackend, refreshSessions, switchWorkspace } from "../App";
import * as api from "../api";
import { agentGlyph } from "../agentGlyph";
import { sortByRecent } from "../agentRecents";
import DirectoryPicker from "./DirectoryPicker";
import Icon from "./Icon";

type LifecycleFilter = "active" | "all" | "paused" | "archived";

export default function AgentPickerModal() {
  const [agents, setAgents] = createSignal<api.Agent[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [switching, setSwitching] = createSignal(false);
  const [error, setError] = createSignal("");
  const [lifecycleFilter, setLifecycleFilter] = createSignal<LifecycleFilter>("active");
  const [lifecycleBusy, setLifecycleBusy] = createSignal<string | null>(null);

  const [searchQuery, setSearchQuery] = createSignal("");

  const allAgents = createMemo<api.Agent[]>(() => {
    const list = agents();
    if (list.some((a) => a.id === "vak")) return list;
    const defaultAgent: api.Agent = {
      id: "vak",
      revision: 1,
      lifecycle: "active",
      name: "Vak",
      character: "spark",
      personality: "Adaptive, helpful, safety-first autonomous assistant.",
      behaviour: "Take initiative on clear requests and explain next useful step.",
      responsibilities: "Core engineering, analysis, coding, and general tasks.",
      instructions: "",
      animation: "subtle",
      voice: "default",
    };
    return [defaultAgent, ...list];
  });

  // Mirrors vak_config::paths::agent_workspace() (crates/vak-config/src/paths.rs):
  // the built-in "vak" agent uses the base workspace directly; every other
  // agent gets an isolated subdirectory nested under that same base, so two
  // agents never share a working directory.
  const resolveAgentWorkspace = (base: string, agentId: string) =>
    agentId === "vak" ? base : `${base}/.vak/agents/${agentId}/workspace`;

  // The session for the currently open chat already carries a server-resolved
  // cwd (see api.openAgent); fall back to computing it client-side when no
  // session has been opened yet for the agent selected in the Fleet Roster.
  const activeWorkspace = createMemo(() => {
    const live = sessions().find((s) => s.session_id === activeId())?.cwd;
    if (live) return live;
    const base = backend().cwd;
    return base ? resolveAgentWorkspace(base, activeAgentId()) : "";
  });

  const filteredAgents = createMemo(() => {
    const q = searchQuery().trim().toLowerCase();
    const filter = lifecycleFilter();
    let list = allAgents().filter((a) => filter === "all" || (a.lifecycle ?? "active") === filter);
    if (q) {
      list = list.filter(
        (a) =>
          a.name.toLowerCase().includes(q) ||
          a.id.toLowerCase().includes(q) ||
          (a.personality && a.personality.toLowerCase().includes(q))
      );
    }
    return sortByRecent(list);
  });

  const setLifecycle = async (agent: api.Agent, lifecycle: api.Agent["lifecycle"]) => {
    setLifecycleBusy(agent.id);
    setError("");
    try {
      const next = agents().map((a) => (a.id === agent.id ? { ...a, lifecycle } : a));
      await api.saveAgents(next, "user");
      setAgents(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to update agent.");
    } finally {
      setLifecycleBusy(null);
    }
  };

  const loadData = async () => {
    setLoading(true);
    setError("");
    try {
      const agentsRes = await api.listAgents();
      setAgents(agentsRes.agents);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  createEffect(() => {
    if (agentPickerOpen()) {
      setSearchQuery("");
      void loadData();
    }
  });

  const handleSelectAgent = async (agentId: string) => {
    if (switching()) return;
    setSwitching(true);
    setError("");
    try {
      const sessionId = await openAgentChat(agentId);
      if (sessionId) {
        setAgentPickerOpen(false);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to open agent.");
    } finally {
      setSwitching(false);
    }
  };

  return (
    <Show when={agentPickerOpen()}>
      <div class="modal-back" onClick={() => setAgentPickerOpen(false)}>
        <div
          class="modal"
          style="max-width: 680px; width: 92vw;"
          role="dialog"
          aria-modal="true"
          aria-labelledby="agent-picker-title"
          onClick={(e) => e.stopPropagation()}
          use:trapFocus
        >
          {/* Header */}
          <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 12px;">
            <div>
              <h3 id="agent-picker-title" style="margin: 0; font-size: 16px; font-weight: 600;">
                Agent Fleet & Working Target
              </h3>
              <span style="color: var(--muted); font-size: 12px;">
                Select an autonomous persona or point to an execution directory.
              </span>
            </div>
            <button
              type="button"
              class="icon-button subtle"
              aria-label="Close"
              onClick={() => setAgentPickerOpen(false)}
            >
              <Icon name="close" size={14} />
            </button>
          </div>

          {/* Navigation Tabs */}
          <div style="display: flex; gap: 6px; margin-bottom: 16px; border-bottom: 1px solid var(--border); padding-bottom: 8px;">
            <button
              type="button"
              class="tab-button"
              classList={{ active: agentPickerTab() === "fleet" }}
              style="display: flex; align-items: center; gap: 6px; padding: 6px 12px; font-size: 13px; border-radius: var(--radius-sm);"
              onClick={() => setAgentPickerTab("fleet")}
            >
              <Icon name="spark" size={14} />
              <span>Fleet Roster ({allAgents().length})</span>
            </button>
            <button
              type="button"
              class="tab-button"
              classList={{ active: agentPickerTab() === "target" }}
              style="display: flex; align-items: center; gap: 6px; padding: 6px 12px; font-size: 13px; border-radius: var(--radius-sm);"
              onClick={() => setAgentPickerTab("target")}
            >
              <Icon name="folder" size={14} />
              <span>Target Directory</span>
            </button>
          </div>

          <Show when={error()}>
            <div style="background: var(--rose-wash); color: var(--red); padding: 8px 12px; border-radius: var(--radius-sm); font-size: 12px; margin-bottom: 12px;">
              {error()}
            </div>
          </Show>

          {/* TAB 1: FLEET ROSTER */}
          <Show when={agentPickerTab() === "fleet"}>
            <div style="display: flex; gap: 8px; margin-bottom: 8px;">
              <input
                type="search"
                placeholder="Find agent by name, id, or specialty…"
                value={searchQuery()}
                onInput={(e) => setSearchQuery(e.currentTarget.value)}
                style="flex: 1; padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--border); background: var(--surface); color: var(--text); font-size: 12.5px;"
              />
              <button
                type="button"
                class="button primary"
                style="white-space: nowrap;"
                onClick={() => { setAgentPickerOpen(false); setAgentCreateOpen(true); }}
              >
                <Icon name="add" size={14} /> New agent
              </button>
            </div>
            <div role="group" aria-label="Filter by status" style="display: flex; gap: 4px; margin-bottom: 8px;">
              <For each={[{ id: "active", label: "Active" }, { id: "paused", label: "Paused" }, { id: "archived", label: "Archived" }, { id: "all", label: "All" }] as const}>
                {(f) => (
                  <button
                    type="button"
                    class="button subtle"
                    classList={{ active: lifecycleFilter() === f.id }}
                    style="font-size: 11.5px; padding: 3px 9px;"
                    aria-pressed={lifecycleFilter() === f.id}
                    onClick={() => setLifecycleFilter(f.id)}
                  >
                    {f.label}
                  </button>
                )}
              </For>
            </div>
            <div style="max-height: 380px; overflow-y: auto; display: flex; flex-direction: column; gap: 8px;">
              <Show when={loading()}>
                <div style="text-align: center; color: var(--muted); padding: 24px; font-size: 13px;">
                  Loading agent fleet…
                </div>
              </Show>
              <Show when={!loading() && filteredAgents().length === 0}>
                <div style="text-align: center; color: var(--muted); padding: 24px; font-size: 13px;">
                  No matching agents found.
                </div>
              </Show>

              <For each={filteredAgents()}>
                {(agent) => {
                  const isActive = createMemo(() => activeAgentId() === agent.id);
                  const glyph = agentGlyph(agent.character);
                  return (
                    <div
                      style="display: flex; align-items: center; justify-content: space-between; padding: 12px; border-radius: var(--radius); border: 1px solid var(--border); background: var(--surface); transition: border-color 0.15s, transform 0.15s; cursor: pointer;"
                      classList={{ "active-agent-card": isActive() }}
                      onClick={() => void handleSelectAgent(agent.id)}
                    >
                      <div style="display: flex; align-items: center; gap: 12px;">
                        <div style="width: 36px; height: 36px; border-radius: 50%; background: var(--surface-raised); border: 1px solid var(--border); display: flex; align-items: center; justify-content: center; font-size: 16px;">
                          {glyph}
                        </div>
                        <div>
                          <div style="display: flex; align-items: center; gap: 6px;">
                            <strong style="font-size: 14px;">{agent.name}</strong>
                            <span style="font-size: 10.5px; padding: 2px 6px; border-radius: 4px; background: var(--surface-raised); color: var(--muted); font-family: var(--mono);">
                              {agent.id}
                            </span>
                            <Show when={isActive()}>
                              <span style="font-size: 10px; padding: 2px 6px; border-radius: 4px; background: var(--accent); color: var(--on-accent); font-weight: 600;">
                                CURRENT
                              </span>
                            </Show>
                            <Show when={agent.lifecycle === "paused"}>
                              <span style="font-size: 10px; padding: 2px 6px; border-radius: 4px; background: var(--surface-raised); color: var(--muted); border: 1px solid var(--border-soft);">
                                PAUSED
                              </span>
                            </Show>
                            <Show when={agent.lifecycle === "archived"}>
                              <span style="font-size: 10px; padding: 2px 6px; border-radius: 4px; background: var(--surface-raised); color: var(--muted); border: 1px solid var(--border-soft);">
                                ARCHIVED
                              </span>
                            </Show>
                          </div>
                          <p style="margin: 3px 0 0; font-size: 12px; color: var(--muted); line-height: 1.3;">
                            {agent.personality || "Persistent autonomous specialist."}
                          </p>
                        </div>
                      </div>

                      <div style="display: flex; align-items: center; gap: 6px;">
                        <Show when={agent.id !== "vak"}>
                          <Show when={agent.lifecycle === "active" || !agent.lifecycle}>
                            <button
                              type="button"
                              class="icon-button subtle has-tooltip"
                              data-tooltip="Pause — hide from the everyday switcher without deleting it"
                              aria-label={`Pause ${agent.name}`}
                              disabled={lifecycleBusy() === agent.id}
                              onClick={(e) => { e.stopPropagation(); void setLifecycle(agent, "paused"); }}
                            >
                              <Icon name="timer" size={14} />
                            </button>
                          </Show>
                          <Show when={agent.lifecycle === "paused"}>
                            <button
                              type="button"
                              class="button subtle"
                              style="font-size: 11.5px; padding: 3px 8px;"
                              disabled={lifecycleBusy() === agent.id}
                              onClick={(e) => { e.stopPropagation(); void setLifecycle(agent, "active"); }}
                            >
                              Resume
                            </button>
                          </Show>
                          <Show when={agent.lifecycle !== "archived"}>
                            <button
                              type="button"
                              class="icon-button subtle has-tooltip"
                              data-tooltip="Archive"
                              aria-label={`Archive ${agent.name}`}
                              disabled={lifecycleBusy() === agent.id}
                              onClick={(e) => { e.stopPropagation(); void setLifecycle(agent, "archived"); }}
                            >
                              <Icon name="archive" size={14} />
                            </button>
                          </Show>
                          <Show when={agent.lifecycle === "archived"}>
                            <button
                              type="button"
                              class="button subtle"
                              style="font-size: 11.5px; padding: 3px 8px;"
                              disabled={lifecycleBusy() === agent.id}
                              onClick={(e) => { e.stopPropagation(); void setLifecycle(agent, "active"); }}
                            >
                              Restore
                            </button>
                          </Show>
                        </Show>
                        <button
                          type="button"
                          class="button"
                          style="font-size: 12px; padding: 4px 10px;"
                          disabled={switching() || isActive()}
                        >
                          {isActive() ? "Active" : "Switch"}
                        </button>
                      </div>
                    </div>
                  );
                }}
              </For>
            </div>
          </Show>

          {/* TAB 2: TARGET WORKING DIRECTORY */}
          <Show when={agentPickerTab() === "target"}>
            <div>
              <div style="padding: 10px 12px; background: var(--surface-raised); border: 1px solid var(--border); border-radius: var(--radius-sm); margin-bottom: 12px;">
                <span style="font-size: 11px; color: var(--muted); text-transform: uppercase; letter-spacing: 0.05em; display: block;">
                  {activeAgentId() === "vak" ? "Current Working Directory" : "Isolated Working Directory"}
                </span>
                <strong style="font-family: var(--mono); font-size: 13px; color: var(--text); word-break: break-all;">
                  {activeWorkspace() || "Default workspace"}
                </strong>
                <p style="margin: 4px 0 0; font-size: 11.5px; color: var(--muted);">
                  {activeAgentId() === "vak"
                    ? "The active agent will run tools, inspect files, and execute commands within this directory."
                    : "Nested under the project workspace so this agent never sees another agent's files. Change the project workspace below to move it."}
                </p>
              </div>

              <DirectoryPicker
                disabled={workspaceSwitching()}
                onPick={async (dir) => {
                  setAgentPickerOpen(false);
                  await switchWorkspace(dir);
                }}
              />
            </div>
          </Show>
        </div>
      </div>
    </Show>
  );
}
