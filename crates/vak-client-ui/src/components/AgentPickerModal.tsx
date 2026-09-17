import { createEffect, createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import { trapFocus } from "../focusTrap";
import {
  activeAgentId,
  agentPickerOpen,
  agentPickerTab,
  backend,
  setActiveId,
  setAgentPickerOpen,
  setAgentPickerTab,
  workspaceSwitching,
} from "../store";
import { openAgentChat, refreshBackend, refreshSessions, switchWorkspace } from "../App";
import * as api from "../api";
import DirectoryPicker from "./DirectoryPicker";
import Icon from "./Icon";

export default function AgentPickerModal() {
  const [agents, setAgents] = createSignal<api.Agent[]>([]);
  const [templates, setTemplates] = createSignal<api.AgentTemplate[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [switching, setSwitching] = createSignal(false);
  const [error, setError] = createSignal("");

  // Create form state
  const [newId, setNewId] = createSignal("");
  const [newName, setNewName] = createSignal("");
  const [newCharacter, setNewCharacter] = createSignal<"orb" | "leaf" | "sun" | "wave" | "spark">("spark");
  const [newPersonality, setNewPersonality] = createSignal("Sharp, methodical, and proactive.");
  const [newInstructions, setNewInstructions] = createSignal("");
  const [creating, setCreating] = createSignal(false);

  const glyphMap: Record<string, string> = {
    orb: "◌",
    leaf: "◒",
    sun: "☼",
    wave: "〰",
    spark: "✦",
  };

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

  const filteredAgents = createMemo(() => {
    const q = searchQuery().trim().toLowerCase();
    const list = allAgents();
    if (!q) return list;
    return list.filter(
      (a) =>
        a.name.toLowerCase().includes(q) ||
        a.id.toLowerCase().includes(q) ||
        (a.personality && a.personality.toLowerCase().includes(q))
    );
  });

  const loadData = async () => {
    setLoading(true);
    setError("");
    try {
      const [agentsRes, templatesRes] = await Promise.all([
        api.listAgents(),
        api.listAgentTemplates().catch(() => ({ templates: [] })),
      ]);
      setAgents(agentsRes.agents);
      setTemplates(templatesRes.templates);
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

  const handleApplyTemplate = (tmpl: api.AgentTemplate) => {
    const slug = tmpl.template_id.toLowerCase().replace(/[^a-z0-9]+/g, "-");
    setNewId(slug);
    setNewName(tmpl.name);
    setNewCharacter(tmpl.character);
    setNewPersonality(tmpl.personality);
    setNewInstructions(tmpl.instructions);
  };

  const handleCreateAgent = async (e: Event) => {
    e.preventDefault();
    const id = newId().trim().toLowerCase().replace(/[^a-z0-9_-]+/g, "-");
    if (!id || !newName().trim()) {
      setError("ID and Name are required.");
      return;
    }
    setCreating(true);
    setError("");
    try {
      const agent: api.Agent = {
        id,
        revision: 1,
        lifecycle: "active",
        name: newName().trim(),
        character: newCharacter(),
        personality: newPersonality().trim(),
        behaviour: "Deliver high-quality outcomes with clear reasoning.",
        responsibilities: "",
        instructions: newInstructions().trim(),
        animation: "subtle",
        voice: "default",
      };
      await api.saveAgents([...agents(), agent], "user");
      await handleSelectAgent(id);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to create agent.");
      setCreating(false);
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
              classList={{ active: agentPickerTab() === "create" }}
              style="display: flex; align-items: center; gap: 6px; padding: 6px 12px; font-size: 13px; border-radius: var(--radius-sm);"
              onClick={() => setAgentPickerTab("create")}
            >
              <Icon name="tune" size={14} />
              <span>Create Specialist</span>
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
            <div style="margin-bottom: 8px;">
              <input
                type="search"
                placeholder="Find agent by name, id, or specialty…"
                value={searchQuery()}
                onInput={(e) => setSearchQuery(e.currentTarget.value)}
                style="width: 100%; padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--border); background: var(--surface); color: var(--text); font-size: 12.5px;"
              />
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
                  const glyph = glyphMap[agent.character] || "✦";
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
                          </div>
                          <p style="margin: 3px 0 0; font-size: 12px; color: var(--muted); line-height: 1.3;">
                            {agent.personality || "Persistent autonomous specialist."}
                          </p>
                        </div>
                      </div>

                      <div>
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

          {/* TAB 2: CREATE SPECIALIST */}
          <Show when={agentPickerTab() === "create"}>
            <div style="max-height: 420px; overflow-y: auto; padding-right: 4px;">
              <Show when={templates().length > 0}>
                <div style="margin-bottom: 14px;">
                  <span style="font-size: 11.5px; font-weight: 600; color: var(--muted); text-transform: uppercase; letter-spacing: 0.04em;">
                    Templates
                  </span>
                  <div style="display: flex; gap: 8px; overflow-x: auto; padding: 6px 0;">
                    <For each={templates()}>
                      {(tmpl) => (
                        <button
                          type="button"
                          style="padding: 6px 10px; border-radius: var(--radius-sm); border: 1px solid var(--border); background: var(--surface-raised); font-size: 12px; white-space: nowrap; cursor: pointer; text-align: left;"
                          onClick={() => handleApplyTemplate(tmpl)}
                        >
                          <span style="font-weight: 500;">{tmpl.name}</span>
                          <span style="display: block; font-size: 10.5px; color: var(--muted);">{tmpl.domain}</span>
                        </button>
                      )}
                    </For>
                  </div>
                </div>
              </Show>

              <form onSubmit={handleCreateAgent} style="display: flex; flex-direction: column; gap: 10px;">
                <div>
                  <label style="display: block; font-size: 12px; font-weight: 500; margin-bottom: 4px;">
                    Agent Name
                  </label>
                  <input
                    type="text"
                    required
                    placeholder="e.g. Security Reviewer"
                    value={newName()}
                    onInput={(e) => setNewName(e.currentTarget.value)}
                    style="width: 100%; padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--border); background: var(--surface); color: var(--text);"
                  />
                </div>

                <div>
                  <label style="display: block; font-size: 12px; font-weight: 500; margin-bottom: 4px;">
                    Personality & Demeanor
                  </label>
                  <input
                    type="text"
                    placeholder="e.g. Rigorous, cautious, and detail-obsessed."
                    value={newPersonality()}
                    onInput={(e) => setNewPersonality(e.currentTarget.value)}
                    style="width: 100%; padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--border); background: var(--surface); color: var(--text);"
                  />
                </div>

                <div>
                  <label style="display: block; font-size: 12px; font-weight: 500; margin-bottom: 4px;">
                    Core Instructions / System Prompt
                  </label>
                  <textarea
                    rows={3}
                    placeholder="Instructions that govern this agent's reasoning, tool use, and tone."
                    value={newInstructions()}
                    onInput={(e) => setNewInstructions(e.currentTarget.value)}
                    style="width: 100%; padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--border); background: var(--surface); color: var(--text); font-family: var(--sans); font-size: 12.5px; resize: vertical;"
                  />
                </div>

                <div style="display: flex; justify-content: flex-end; gap: 8px; margin-top: 6px;">
                  <button
                    type="button"
                    class="button subtle"
                    onClick={() => setAgentPickerTab("fleet")}
                  >
                    Cancel
                  </button>
                  <button
                    type="submit"
                    class="button primary"
                    disabled={creating() || !newName().trim()}
                  >
                    {creating() ? "Creating…" : "Create & Launch"}
                  </button>
                </div>
              </form>
            </div>
          </Show>

          {/* TAB 3: TARGET WORKING DIRECTORY */}
          <Show when={agentPickerTab() === "target"}>
            <div>
              <div style="padding: 10px 12px; background: var(--surface-raised); border: 1px solid var(--border); border-radius: var(--radius-sm); margin-bottom: 12px;">
                <span style="font-size: 11px; color: var(--muted); text-transform: uppercase; letter-spacing: 0.05em; display: block;">
                  Current Working Directory
                </span>
                <strong style="font-family: var(--mono); font-size: 13px; color: var(--text);">
                  {backend().cwd || "Default workspace"}
                </strong>
                <p style="margin: 4px 0 0; font-size: 11.5px; color: var(--muted);">
                  The active agent will run tools, inspect files, and execute commands within this directory.
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
