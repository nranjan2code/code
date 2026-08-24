import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import {
  density,
  openInEditor,
  providers,
  setDensity,
  setNotice,
  setProviders,
  setSettingsOpen,
  setSetupNeeded,
  setShowShortcuts,
  uiPreferences,
  updateUiPreference,
  sessions,
  type Density,
} from "../store";
import type { ConfigSnapshot } from "../types";
import * as api from "../api";
import { loadHealth, refreshSessions } from "../App";
import Icon, { type IconName } from "./Icon";

/** Sentinel option that swaps the model select for a free-text field. */
const CUSTOM_MODEL = "\u0000custom";

type Page = "general" | "appearance" | "agent" | "permissions" | "reliability" | "integrations" | "services" | "learning" | "advanced" | "archived";

const pages: { id: Page; label: string; icon: IconName; hint: string }[] = [
  { id: "general", label: "General", icon: "gear", hint: "notifications suggestions" },
  { id: "appearance", label: "Appearance", icon: "palette", hint: "theme text density motion" },
  { id: "agent", label: "Agent", icon: "spark", hint: "provider model turns subagents" },
  { id: "permissions", label: "Permissions", icon: "shield", hint: "access sandbox approvals" },
  { id: "reliability", label: "Reliability", icon: "timer", hint: "retries timeout circuit breaker" },
  { id: "integrations", label: "Integrations", icon: "plug", hint: "mcp hooks skills" },
  { id: "services", label: "Services", icon: "grid", hint: "gateway bridge tray watchdog background" },
  { id: "learning", label: "Learning", icon: "history", hint: "memory notes skill proposals review promote" },
  { id: "advanced", label: "Advanced", icon: "tune", hint: "paths context configuration" },
  { id: "archived", label: "Archived tasks", icon: "archive", hint: "restore delete history" },
];

function Switch(props: { checked: boolean; onChange: (next: boolean) => void; label: string }) {
  return <button class="switch" classList={{ on: props.checked }} role="switch" aria-checked={props.checked} aria-label={props.label} onClick={() => props.onChange(!props.checked)}><span /></button>;
}

function Row(props: { title: string; description: string; children: JSX.Element; danger?: boolean }) {
  return <div class="setting-row" classList={{ danger: props.danger }}><div class="setting-copy"><strong>{props.title}</strong><span>{props.description}</span></div><div class="setting-control">{props.children}</div></div>;
}

function Group(props: { title?: string; children: JSX.Element }) {
  return <section class="settings-group"><Show when={props.title}><h3>{props.title}</h3></Show><div class="settings-card">{props.children}</div></section>;
}

function fmt(value: number): string {
  return value.toLocaleString();
}

export default function Settings() {
  const [page, setPage] = createSignal<Page>("general");
  const [query, setQuery] = createSignal("");
  const [config, setConfig] = createSignal<ConfigSnapshot | null>(null);
  const [loading, setLoading] = createSignal(true);
  // Background-service states (docs/design/27-operations.md); polled while
  // the Services page is open.
  const [ops, setOps] = createSignal<api.OpsStatusShape | null>(null);

  async function refreshOps() {
    try {
      setOps(await api.opsStatus());
    } catch {
      /* gateway down is itself the state we are displaying */
    }
  }

  const [notes, setNotes] = createSignal<api.NoteBlock[]>([]);
  const [proposals, setProposals] = createSignal<api.SkillProposal[]>([]);

  // MCP manager state: loaded when the integrations page opens; edits are
  // local until Save pushes the whole table.
  const [mcpServers, setMcpServers] = createSignal<Record<string, api.McpServerDef> | null>(null);
  const [mcpDirty, setMcpDirty] = createSignal(false);
  const [mcpSaving, setMcpSaving] = createSignal(false);

  async function refreshMcp() {
    try {
      const res = await api.getMcpServers();
      setMcpServers(res.servers ?? {});
      setMcpDirty(false);
    } catch (e) {
      setNotice({ kind: "error", text: `Could not load MCP servers: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  async function saveMcp() {
    const servers = mcpServers() ?? {};
    setMcpSaving(true);
    try {
      await api.putMcpServers(servers);
      setMcpDirty(false);
      setNotice({ kind: "info", text: `Saved ${Object.keys(servers).length} MCP server(s) — applied to new turns` });
    } catch (e) {
      setNotice({ kind: "error", text: e instanceof Error ? e.message : String(e) });
    } finally {
      setMcpSaving(false);
    }
  }

  function updateServer(name: string, patch: Partial<api.McpServerDef>) {
    setMcpServers((cur) => ({ ...cur, [name]: { ...(cur?.[name] ?? { command: "", args: [], env: {}, network: false }), ...patch } }));
    setMcpDirty(true);
  }

  function addServer() {
    let n = "new-server";
    let i = 2;
    while (mcpServers()?.[n]) n = `new-server-${i++}`;
    setMcpServers((cur) => ({ ...(cur ?? {}), [n]: { command: "", args: [], env: {}, network: false } }));
    setMcpDirty(true);
  }

  function renameServer(oldName: string, nextName: string) {
    if (!nextName.trim() || nextName === oldName) return;
    setMcpServers((cur) => {
      const entries = Object.entries(cur ?? {});
      const replaced = entries.map(([k, v]) => (k === oldName ? [nextName, v] as [string, api.McpServerDef] : [k, v] as [string, api.McpServerDef]));
      return Object.fromEntries(replaced);
    });
    setMcpDirty(true);
  }

  function removeServer(name: string) {
    setMcpServers((cur) => {
      const next = { ...(cur ?? {}) };
      delete next[name];
      return next;
    });
    setMcpDirty(true);
  }


  async function refreshLearning() {
    try {
      setNotes((await api.listMemory()).notes);
      setProposals((await api.listProposals()).proposals);
    } catch {
      /* gateway down — lists simply stay stale */
    }
  }

  async function promote(id: string) {
    try {
      await api.promoteProposal(id);
      await refreshLearning();
    } catch (e) {
      setNotice({ kind: "error", text: e instanceof Error ? e.message : String(e) });
    }
  }

  async function reject(id: string) {
    try {
      await api.rejectProposal(id);
      await refreshLearning();
    } catch (e) {
      setNotice({ kind: "error", text: e instanceof Error ? e.message : String(e) });
    }
  }

  async function runOp(service: "gateway" | "telegram", action: "start" | "stop" | "restart" | "install" | "uninstall") {
    try {
      await api.opsAction(service, action);
      await refreshOps();
    } catch (e) {
      setNotice({ kind: "error", text: e instanceof Error ? e.message : String(e) });
    }
  }
  const [provider, setProvider] = createSignal("");
  const [model, setModel] = createSignal("");
  // Discovered from the provider's API with the configured key — never a
  // baked-in list, which goes stale the moment a provider ships a model.
  const [catalog, setCatalog] = createSignal<string[]>([]);
  const [catalogNote, setCatalogNote] = createSignal<string | null>(null);
  // Any identifier the provider accepts is valid, so the list never becomes a
  // cage: this switches the control to free text.
  const [customModel, setCustomModel] = createSignal(false);
  const [maxTurns, setMaxTurns] = createSignal(40);
  const [saving, setSaving] = createSignal(false);
  const [keyDraft, setKeyDraft] = createSignal<string | null>(null);
  const [keyBusy, setKeyBusy] = createSignal(false);

  const loadProviders = async () => {
    try {
      const p = await api.listProviders();
      setProviders(p);
      setSetupNeeded(!p.current_configured);
    } catch {
      /* keep previous snapshot */
    }
  };
  onMount(() => void loadProviders());
  onMount(() => void refreshSessions());
  createEffect(() => {
    if (page() !== "services") return;
    void refreshOps();
    const t = setInterval(() => void refreshOps(), 5000);
    onCleanup(() => clearInterval(t));
  });
  createEffect(() => {
    if (page() !== "learning") return;
    void refreshLearning();
    const t = setInterval(() => void refreshLearning(), 8000);
    onCleanup(() => clearInterval(t));
  });
  createEffect(() => {
    if (page() !== "integrations") return;
    void refreshMcp();
  });

  // Re-run whenever the selected provider changes; a stale response from a
  // provider the user has since moved off is discarded.
  createEffect(() => {
    const name = provider();
    // Track the configured flag too: adding or revoking a key changes what
    // this provider can reach, so the catalogue must be re-derived.
    providers()?.providers.find((p) => p.name === name)?.configured;
    if (!name) return;
    setCatalog([]);
    setCatalogNote("discovering models…");
    void (async () => {
      try {
        const r = await api.discoverModels(name);
        if (name !== provider()) return;
        setCatalog(r.models);
        setCatalogNote(r.models.length ? null : "provider returned no models");
        if (r.models.length && !r.models.includes(model())) setModel(r.models[0]);
      } catch (error) {
        if (name !== provider()) return;
        setCatalogNote(error instanceof Error ? error.message : String(error));
      }
    })();
  });

  // The provider the credential controls act on — the selected one, or the
  // active one before any selection has been made.
  const keyProvider = () => provider() || providers()?.current || "";

  const currentProviderInfo = () =>
    providers()?.providers.find((p) => p.name === (provider() || providers()?.current));

  const load = async () => {
    setLoading(true);
    try {
      const next = await api.getConfig();
      setConfig(next);
      setProvider(next.provider);
      setModel(next.model);
      setMaxTurns(next.max_turns);
    } catch (error) {
      setNotice({ kind: "error", text: `Could not load settings: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setLoading(false);
    }
  };
  onMount(() => void load());
  const visiblePages = createMemo(() => {
    const needle = query().trim().toLowerCase();
    return (needle ? pages.filter((item) => `${item.label} ${item.hint}`.toLowerCase().includes(needle)) : pages).filter((item) => item.id !== "archived");
  });
  const showArchivedPage = createMemo(() => {
    const needle = query().trim().toLowerCase();
    return !needle || "archived tasks restore delete history".includes(needle);
  });
  const archivedSessions = createMemo(() => sessions().filter((session) => session.archived));

  const restoreTask = async (id: string) => {
    try {
      await api.setArchived(id, false);
      await refreshSessions();
      setNotice({ kind: "info", text: "Task restored to the sidebar." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not restore that task: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  const deleteTask = async (id: string) => {
    const task = archivedSessions().find((session) => session.session_id === id);
    if (!window.confirm(`Delete “${task?.title || "Untitled task"}”? This cannot be undone in VakCoder.`)) return;
    try {
      await api.deleteSession(id);
      await refreshSessions();
      setNotice({ kind: "info", text: "Task deleted from history." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not delete that task: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  const deleteAllArchived = async () => {
    if (!archivedSessions().length || !window.confirm(`Delete all ${archivedSessions().length} archived tasks? This cannot be undone in VakCoder.`)) return;
    try {
      const result = await api.deleteAllArchived();
      await refreshSessions();
      setNotice({ kind: "info", text: `${result.deleted} archived task${result.deleted === 1 ? "" : "s"} deleted.` });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not delete archived tasks: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  // True when the form differs from what the backend is actually running.
  const agentDirty = () => {
    const c = config();
    if (!c) return false;
    return provider() !== c.provider || model() !== c.model || maxTurns() !== c.max_turns;
  };

  const applyAgent = async () => {
    setSaving(true);
    try {
      await api.patchConfig({ provider: provider(), model: model(), max_turns: maxTurns() });
      await Promise.all([load(), loadHealth()]);
      setNotice({ kind: "info", text: "Agent defaults updated for new tasks." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not update agent settings: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setSaving(false);
    }
  };

  const saveKey = async () => {
    const draft = keyDraft()?.trim();
    if (!draft) return;
    setKeyBusy(true);
    try {
      const res = await api.putProviderKey(provider() || providers()?.current || "", draft);
      setKeyDraft(null);
      // A new key can reach a different set of models.
      try {
        const fresh = await api.discoverModels(provider() || providers()?.current || "");
        setCatalog(fresh.models);
        setCatalogNote(fresh.models.length ? null : "provider returned no models");
      } catch (e) {
        setCatalogNote(e instanceof Error ? e.message : String(e));
      }
      await Promise.all([loadProviders(), loadHealth()]);
      setNotice({ kind: "info", text: `Key stored locally (${res.env_var}).` });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not store key: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setKeyBusy(false);
    }
  };

  const removeKey = async () => {
    const name = provider() || providers()?.current || "";
    setKeyBusy(true);
    try {
      const res = await api.removeProviderKey(name);
      await Promise.all([loadProviders(), loadHealth()]);
      // The catalogue is meaningless without a key; re-derive it.
      setCatalog([]);
      setCatalogNote(null);
      setNotice(
        res.shadowed_by_env
          ? { kind: "error", text: `Removed the stored key, but ${res.env_var} is still set in your environment, so ${name} stays authenticated.` }
          : { kind: "info", text: `Key removed (${res.env_var}).` },
      );
    } catch (error) {
      setNotice({ kind: "error", text: `Could not remove key: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setKeyBusy(false);
    }
  };

  const changePermission = async (mode: ConfigSnapshot["permission_mode"]) => {
    const wire = mode === "ReadOnly" ? "read-only" : mode === "WorkspaceWrite" ? "workspace-write" : "full-access";
    try {
      await api.patchConfig({ permission_mode: wire });
      setConfig((current) => current ? { ...current, permission_mode: mode } : current);
      await loadHealth();
    } catch (error) {
      setNotice({ kind: "error", text: `Could not update permissions: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  const openProjectConfig = async () => {
    const relative = ".vakcoder/config.toml";
    try {
      await api.readFile(relative);
    } catch {
      const c = config();
      await api.writeFile(relative, `provider = "${c?.provider ?? "anthropic"}"\nmodel = "${c?.model ?? "claude-sonnet-4-5"}"\npermission_mode = "workspace-write"\n`);
    }
    openInEditor(relative);
    setSettingsOpen(false);
  };

  return (
    <div class="settings-shell" role="dialog" aria-modal="true" aria-label="Settings">
      <aside class="settings-nav">
        <button class="settings-back" onClick={() => setSettingsOpen(false)}><Icon name="chevron" /><span>Back to VakCoder</span></button>
        <div class="settings-search"><Icon name="search" /><input aria-label="Search settings" placeholder="Search settings…" value={query()} onInput={(event) => setQuery(event.currentTarget.value)} /></div>
        <div class="settings-nav-label">Workspace</div>
        <nav>
          <For each={visiblePages()} fallback={<div class="settings-no-results">No matching settings</div>}>
            {(item) => <button classList={{ active: page() === item.id }} onClick={() => { setPage(item.id); setQuery(""); }}><Icon name={item.icon} /><span>{item.label}</span></button>}
          </For>
        </nav>
        <Show when={showArchivedPage()}>
          <div class="settings-nav-label archived-nav-label">Archived</div>
          <nav>
            <button classList={{ active: page() === "archived" }} onClick={() => { setPage("archived"); setQuery(""); }}><Icon name="archive" /><span>Archived tasks</span></button>
          </nav>
        </Show>
        <div class="settings-nav-foot"><div class="settings-app-mark"><img src="/vakcoder-icon.png" alt="" /></div><div><strong>VakCoder</strong><span>Version 0.2.0</span></div></div>
      </aside>

      <main class="settings-main">
        <div class="settings-content">
          <Show when={!loading()} fallback={<div class="settings-loading"><span /><span /><span /></div>}>
            <Show when={page() === "general"}>
              <header><h1>General</h1><p>Choose how VakCoder behaves across projects.</p></header>
              <Group title="Experience">
                <Row title="Desktop notifications" description="Notify when the active task finishes while VakCoder is in the background."><Switch label="Desktop notifications" checked={uiPreferences.notifications} onChange={(value) => updateUiPreference("notifications", value)} /></Row>
                <Row title="Suggested prompts" description="Show useful starting points when a task has no conversation yet."><Switch label="Suggested prompts" checked={uiPreferences.suggestions} onChange={(value) => updateUiPreference("suggestions", value)} /></Row>
                <Row title="Transcript detail" description="Control how much agent activity appears in conversations."><select value={density()} onChange={(event) => setDensity(event.currentTarget.value as Density)}><option value="summary">Summary</option><option value="normal">Normal</option><option value="verbose">Verbose</option></select></Row>
                <Row title="Keyboard shortcuts" description="See every shortcut for navigation, tasks, and workspace tools."><button class="settings-button" onClick={() => { setSettingsOpen(false); setShowShortcuts(true); }}>View shortcuts</button></Row>
              </Group>
              <Group title="Project">
                <Row title="Current workspace" description={config()?.paths.cwd ?? ""}><span class="settings-value">Local</span></Row>
                <Row title="Project configuration" description="Persistent agent and tool settings for this repository."><button class="settings-button" onClick={() => void openProjectConfig()}>Open config</button></Row>
              </Group>
            </Show>

            <Show when={page() === "archived"}>
              <header class="archived-header"><div><h1>Archived tasks</h1><p>Hidden from the sidebar until you restore them.</p></div><button class="settings-button danger" disabled={!archivedSessions().length} onClick={() => void deleteAllArchived()}><Icon name="trash" size={14} /> Delete all</button></header>
              <div class="settings-callout"><Icon name="archive" /><div><strong>Archive is reversible</strong><span>Restore a task any time. Deleting removes it from VakCoder’s task history; the append-only session ledger remains untouched on disk.</span></div></div>
              <Show when={archivedSessions().length} fallback={<div class="archived-empty"><Icon name="archive" size={24} /><strong>No archived tasks</strong><span>Tasks you archive from the sidebar will appear here.</span></div>}>
                <section class="archived-list" aria-label="Archived tasks">
                  <For each={archivedSessions()}>{(session) => <div class="archived-item"><span class="archived-item-icon"><Icon name="chat" size={15} /></span><span class="archived-item-copy"><strong>{session.title || "Untitled task"}</strong><span>{session.updated_at ? new Date(session.updated_at).toLocaleString() : ""} · {session.entries ?? 0} events</span></span><button class="settings-button" onClick={() => void restoreTask(session.session_id)}><Icon name="restore" size={13} /> Restore</button><button class="icon-button subtle danger has-tooltip" data-tooltip="Delete task" aria-label={`Delete ${session.title || "untitled task"}`} onClick={() => void deleteTask(session.session_id)}><Icon name="trash" size={14} /></button></div>}</For>
                </section>
              </Show>
            </Show>

            <Show when={page() === "appearance"}>
              <header><h1>Appearance</h1><p>Make the workspace comfortable for long sessions.</p></header>
              <Group title="Theme">
                <div class="theme-grid"><For each={[{ id: "warm", label: "Warm dark" }, { id: "dark", label: "Midnight" }, { id: "contrast", label: "High contrast" }] as const}>{(theme) => <button class="theme-choice" classList={{ active: uiPreferences.theme === theme.id }} onClick={() => updateUiPreference("theme", theme.id)}><span class={`theme-preview ${theme.id}`}><i /><i /><i /></span><strong>{theme.label}</strong><Show when={uiPreferences.theme === theme.id}><Icon name="check" /></Show></button>}</For></div>
              </Group>
              <Group title="Layout and text">
                <Row title="Text size" description="Conversation and interface text."><div class="range-control"><input type="range" min="90" max="120" step="5" value={uiPreferences.textScale} onInput={(event) => updateUiPreference("textScale", Number(event.currentTarget.value))} /><span>{uiPreferences.textScale}%</span></div></Row>
                <Row title="Code size" description="Code blocks, diffs, editor, and terminal labels."><div class="range-control"><input type="range" min="90" max="125" step="5" value={uiPreferences.codeScale} onInput={(event) => updateUiPreference("codeScale", Number(event.currentTarget.value))} /><span>{uiPreferences.codeScale}%</span></div></Row>
                <Row title="Compact task list" description="Fit more tasks in the sidebar with tighter rows."><Switch label="Compact task list" checked={uiPreferences.compactSidebar} onChange={(value) => updateUiPreference("compactSidebar", value)} /></Row>
                <Row title="Reduce motion" description="Disable pulsing, smooth scrolling, and animated transitions."><Switch label="Reduce motion" checked={uiPreferences.reduceMotion} onChange={(value) => updateUiPreference("reduceMotion", value)} /></Row>
              </Group>
            </Show>

            <Show when={page() === "agent"}>
              <header><h1>Agent</h1><p>Configure the model used when starting new tasks.</p></header>
              <div class="settings-callout"><Icon name="spark" /><div><strong>Runtime defaults</strong><span>Applied changes take effect on the next task. Add them to the project config to keep them across restarts.</span></div></div>
              <Group title="Model">
                <Row title="Provider" description={currentProviderInfo()?.env_var ? `Authenticated via ${currentProviderInfo()?.env_var}` : "The API provider used for new sessions."}>
                  <select
                    class="settings-input"
                    value={provider()}
                    onChange={(event) => {
                      const next = event.currentTarget.value;
                      setProvider(next);
                      setKeyDraft(null);
                      // The catalogue effect re-runs off provider() and
                      // reconciles the model against what the key reaches.
                    }}
                  >
                    <For each={providers()?.providers ?? []}>{(p) => <option value={p.name}>{p.name}{p.configured ? " ✓" : ""}</option>}</For>
                    <Show when={provider() && !providers()?.providers.some((p) => p.name === provider())}><option value={provider()}>{provider()}</option></Show>
                  </select>
                </Row>
                <Row
                  title="Model"
                  description={catalogNote() ?? `${catalog().length} models available for this key.`}
                >
                  <Show
                    when={!customModel()}
                    fallback={
                      <span class="key-edit">
                        <input
                          class="settings-input wide"
                          placeholder="exact model id"
                          value={model()}
                          onInput={(event) => setModel(event.currentTarget.value)}
                        />
                        <button class="settings-button" onClick={() => setCustomModel(false)}>Choose from list</button>
                      </span>
                    }
                  >
                    <select
                      class="settings-input wide"
                      value={model()}
                      onChange={(event) => {
                        const next = event.currentTarget.value;
                        if (next === CUSTOM_MODEL) setCustomModel(true);
                        else setModel(next);
                      }}
                    >
                      {/* The configured model may predate this key or be a
                          bare id the provider accepts but does not list. */}
                      <Show when={model() && !catalog().includes(model())}>
                        <option value={model()}>{model()}</option>
                      </Show>
                      <For each={catalog()}>{(m) => <option value={m}>{m}</option>}</For>
                      <option value={CUSTOM_MODEL}>Enter a model id…</option>
                    </select>
                  </Show>
                </Row>
                <Row title="Maximum turns" description="Hard limit for one task before the agent stops."><input class="settings-number" type="number" min="1" max="1000" value={maxTurns()} onInput={(event) => setMaxTurns(Number(event.currentTarget.value))} /></Row>
                <Row title="Subagents" description="Allow the agent to delegate bounded parallel work."><span class="settings-status good">{config()?.subagents ? "Enabled" : "Disabled in config"}</span></Row>
              </Group>
              <Group title="Credentials">
                <Row
                  title={`${keyProvider()} — ${currentProviderInfo()?.env_var ?? "no key needed"}`}
                  description={
                    currentProviderInfo()?.requires_key
                      ? `${currentProviderInfo()?.configured ? "Saved on this device" : "Not set yet"} · stored in ~/.vakcoder/.env with owner-only permissions. A real environment variable takes precedence.`
                      : `${keyProvider()} runs locally and needs no key.`
                  }
                >
                  <Show
                    when={keyDraft() === null}
                    fallback={
                      <span class="key-edit">
                        <input type="password" autocomplete="off" spellcheck={false} placeholder={`paste ${currentProviderInfo()?.env_var ?? "API key"}`} aria-label={`${currentProviderInfo()?.env_var ?? "API key"} for ${keyProvider()}`} value={keyDraft() ?? ""} onInput={(e) => setKeyDraft(e.currentTarget.value)} onKeyDown={(e) => e.key === "Enter" && void saveKey()} />
                        <button class="btn primary sm" disabled={keyBusy() || !keyDraft()?.trim()} onClick={() => void saveKey()}>{keyBusy() ? "Saving…" : "Save"}</button>
                        <button class="settings-button" onClick={() => setKeyDraft(null)}>Cancel</button>
                      </span>
                    }
                  >
                    <Show when={currentProviderInfo()?.requires_key} fallback={<span class="settings-status good">Not required</span>}>
                      <span class="key-edit">
                        <button class="settings-button" onClick={() => setKeyDraft("")}>{currentProviderInfo()?.configured ? "Replace key" : "Add key"}</button>
                        <Show when={currentProviderInfo()?.configured}>
                          <button class="settings-button danger" disabled={keyBusy()} onClick={() => void removeKey()}>Remove key</button>
                        </Show>
                      </span>
                    </Show>
                  </Show>
                </Row>
              </Group>
              <div class="settings-actions">
                <button class="btn primary" disabled={saving() || !agentDirty() || !provider().trim() || !model().trim()} onClick={() => void applyAgent()}>{saving() ? "Applying…" : "Apply changes"}</button>
                <button class="settings-button" onClick={() => void openProjectConfig()}>Edit persistent config</button>
                {/* Selecting in the dropdowns changes nothing until this is
                    pressed; without a marker that reads as a silent no-op. */}
                <Show when={agentDirty()} fallback={<span class="settings-status good">Saved</span>}>
                  <span class="settings-status warn">Unsaved changes — press Apply</span>
                </Show>
              </div>
            </Show>

            <Show when={page() === "permissions"}>
              <header><h1>Permissions</h1><p>Set the trust boundary for tool calls in this workspace.</p></header>
              <div class="permission-options"><For each={[{ id: "ReadOnly", title: "Read only", text: "Inspect files and search the workspace without making changes.", icon: "preview" as IconName }, { id: "WorkspaceWrite", title: "Workspace write", text: "Edit files inside this project and ask before sensitive actions.", icon: "code" as IconName }, { id: "FullAccess", title: "Full access", text: "Run unrestricted commands and access files outside the workspace.", icon: "shield" as IconName }] as const}>{(mode) => <button classList={{ active: config()?.permission_mode === mode.id, danger: mode.id === "FullAccess" }} onClick={() => void changePermission(mode.id)}><span class="permission-icon"><Icon name={mode.icon} /></span><span><strong>{mode.title}</strong><small>{mode.text}</small></span><span class="permission-check"><Show when={config()?.permission_mode === mode.id}><Icon name="check" /></Show></span></button>}</For></div>
              <Group title="Sandbox">
                <Row title="Workspace boundary" description="File tools are confined to the selected project and symlinks are resolved before access."><span class="settings-status good">Protected</span></Row>
                <Row title="Permission rules" description="Configure allow, ask, and deny patterns in the project configuration."><button class="settings-button" onClick={() => void openProjectConfig()}>Edit rules</button></Row>
              </Group>
            </Show>

            <Show when={page() === "reliability"}>
              <header><h1>Reliability</h1><p>Understand how VakCoder recovers from provider and task failures.</p></header>
              <Group title="Request recovery">
                <Row title="Provider retries" description={`Initial backoff ${fmt(config()?.retry_base_backoff_ms ?? 0)} ms.`}><span class="metric">{config()?.max_retries}</span></Row>
                <Row title="Request watchdog" description="Maximum time for a single provider step."><span class="metric">{config()?.request_timeout_secs}s</span></Row>
                <Row title="Run endurance" description={`Backoff starts at ${fmt(config()?.run_retry_base_backoff_ms ?? 0)} ms.`}><span class="metric">{config()?.run_retry_attempts} attempts</span></Row>
              </Group>
              <Group title="Circuit breaker">
                <Row title="Failure threshold" description="Blind failures before new requests fail fast."><span class="metric">{config()?.circuit_breaker_threshold}</span></Row>
                <Row title="Cooldown" description="Time before a half-close probe is allowed."><span class="metric">{config()?.circuit_breaker_cooldown_secs}s</span></Row>
                <Row title="Completion guard" description="Blocks premature completion and asks the agent to verify work."><span class="settings-status good">{config()?.stop_policy.enabled ? "Enabled" : "Disabled"}</span></Row>
              </Group>
              <Group title="Route ladder">
                <Row title="Objective" description={`How fallback legs are ordered: ${config()?.route.objective === "auto" ? "derived from request demand (utility / balanced / quality-critical)." : `fixed to ${config()?.route.objective}.`}`}><span class="metric">{config()?.route.objective}</span></Row>
                <Row
                  title="Cross-model fallbacks"
                  description={config()?.route.fallback_models.length
                    ? `Allowed models, admitted only when discovery reaches them: ${config()!.route.fallback_models.join(", ")}.`
                    : "Same model on other providers only. Add route.fallback_models in project config to allow named alternates."}
                >
                  <span class="metric">{config()?.route.fallback_models.length ?? 0}</span>
                </Row>
                <Row title="Ladder length cap" description="Maximum frozen legs per session, including your primary choice — the primary never loses its head position."><span class="metric">{config()?.route.max_fallbacks}</span></Row>
              </Group>
              <button class="settings-button" onClick={() => void openProjectConfig()}>Tune in project config</button>
            </Show>

            <Show when={page() === "services"}>
              <header><h1>Services</h1><p>Gateway and bridge run in the background — the menu-bar dot and this panel always show the same truth.</p></header>
              <Group title="Background services">
                <For each={["gateway", "telegram"] as const}>
                  {(svc) => (
                    <Row
                      title={svc === "gateway" ? "Gateway" : "Telegram bridge"}
                      description={
                        svc === "gateway"
                          ? `Serves sessions, gateway routing and routines on port 8901.${ops()?.gateway_healthy ? "" : " Not answering right now."}`
                          : "Bridges your Telegram bot to the gateway."
                      }
                    >
                      <span class="settings-status" classList={{ good: ops()?.[svc]?.state === "running", bad: ops()?.[svc]?.state !== "running" }}>
                        {ops()?.[svc]?.state ?? "?"}
                      </span>
                      <button class="settings-button" disabled={ops()?.[svc]?.state !== "running"} onClick={() => void runOp(svc, "stop")}>Stop</button>
                      <button class="settings-button" disabled={ops()?.[svc]?.state === "running"} onClick={() => void runOp(svc, "start")}>Start</button>
                      <button class="settings-button" onClick={() => void runOp(svc, "restart")}>Restart</button>
                    </Row>
                  )}
                </For>
              </Group>
              <Group title="Maintenance">
                <Row title="Auto-refresh" description="This panel polls service state every 5 seconds while open."><span class="settings-status good">Live</span></Row>
                <Row title="Menu-bar controller" description="vakcoder-tray shows a colour-coded dot with the same controls and a crash watchdog."><span class="settings-status good">Available</span></Row>
                <Row title="Install as services" description="Installs launchd/systemd units so both processes survive reboots (scripts/install_gateway_service.sh --with-tray)."><button class="settings-button" onClick={() => void runOp("gateway", "install")}>Install</button><button class="settings-button" onClick={() => void runOp("gateway", "uninstall")}>Uninstall</button></Row>
              </Group>
            </Show>

            <Show when={page() === "learning"}>
              <header><h1>Learning</h1><p>What the agent has remembered across sessions, and skill drafts waiting for your approval.</p></header>
              <Group title={`Skill proposals (${proposals().length})`}>
                <Show
                  when={proposals().length > 0}
                  fallback={<Row title="Queue is empty" description="Reflection and /propose_skill add drafts here; nothing reaches the agent until you promote it."><span class="settings-status good">Clean</span></Row>}
                >
                  <For each={proposals()}>
                    {(p) => (
                      <div class="setting-row">
                        <div class="setting-copy">
                          <strong>{p.name}</strong>
                          <span>{p.description}</span>
                        </div>
                        <div class="setting-control">
                          <button class="settings-button" onClick={() => void promote(p.id)}>Promote</button>
                          <button class="settings-button" onClick={() => void reject(p.id)}>Reject</button>
                        </div>
                      </div>
                    )}
                  </For>
                </Show>
              </Group>
              <Group title={`Memory notes (${notes().length})`}>
                <Show
                  when={notes().length > 0}
                  fallback={<Row title="No notes yet" description="Chat with reflection enabled — durable decisions land here as plain markdown you can edit in ~/.vakcoder/memory/."><span class="settings-status good">Ready</span></Row>}
                >
                  <div class="archived-list" aria-label="Memory notes">
                    <For each={notes().slice().reverse().slice(0, 20)}>
                      {(n) => (
                        <div class="task-row">
                          <div>
                            <strong>{n.tag || n.kind}</strong>
                            <small>{new Date(n.ts).toLocaleString()} · {n.kind} · from {n.session_id.slice(0, 8)}</small>
                            <p>{n.text}</p>
                          </div>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </Group>
            </Show>

            <Show when={page() === "integrations"}>
              <header><h1>Integrations</h1><p>Extend VakCoder with tools, lifecycle automation, and reusable expertise.</p></header>
              <Group title="MCP servers">
                <Show
                  when={mcpServers()}
                  fallback={<Row title="Loading…" description="Reading the effective MCP table."><span /></Row>}
                >
                  <div class="mcp-editor">
                    <For each={Object.entries(mcpServers() ?? {})}>
                      {([name, def]) => (
                        <div class="mcp-row" data-name={name}>
                          <div class="mcp-fields">
                            <input
                              class="mcp-name"
                              value={name}
                              aria-label="Server name"
                              onChange={(e) => renameServer(name, e.currentTarget.value.trim())}
                            />
                            <input
                              class="mcp-command"
                              placeholder="/path/to/command"
                              value={def.command}
                              aria-label="Command"
                              onInput={(e) => updateServer(name, { command: e.currentTarget.value })}
                            />
                            <input
                              class="mcp-args"
                              placeholder="args (space separated)"
                              value={def.args.join(" ")}
                              aria-label="Arguments"
                              onInput={(e) => updateServer(name, { args: e.currentTarget.value.split(" ").filter(Boolean) })}
                            />
                          </div>
                          <div class="mcp-controls">
                            <label class="mcp-network" title="Allow outbound network for this server">
                              <Switch checked={def.network} label={`Network for ${name}`} onChange={(v) => updateServer(name, { network: v })} />
                              network
                            </label>
                            <button class="settings-button danger" aria-label={`Remove ${name}`} onClick={() => removeServer(name)}>
                              <Icon name="trash" /> Remove
                            </button>
                          </div>
                        </div>
                      )}
                    </For>
                    <div class="settings-actions">
                      <button class="btn" onClick={addServer}><Icon name="add" /> Add server</button>
                      <button class="btn primary" disabled={!mcpDirty() || mcpSaving()} onClick={() => void saveMcp()}>
                        {mcpSaving() ? "Saving…" : "Save & apply"}
                      </button>
                      <Show when={mcpDirty()}><span class="mcp-dirty">unsaved changes</span></Show>
                    </div>
                    <p class="settings-hint">Servers start sandboxed; “network” allows outbound connections. Env values may reference $SECRETS via $&#123;&#125;. Applies to new turns immediately and persists to .vakcoder/config.toml.</p>
                  </div>
                </Show>
              </Group>
              <div class="integration-grid">
                <div class="integration-card"><span><Icon name="tune" /></span><strong>Hooks</strong><p>Run commands before or after agent lifecycle events.</p><em>{config()?.integrations.hooks ?? 0} configured</em></div>
                <div class="integration-card"><span><Icon name="spark" /></span><strong>Skills</strong><p>Reusable instructions discovered from the project and user home.</p><em>{config()?.integrations.skills.length ?? 0} discovered</em><For each={config()?.integrations.skills.slice(0, 4)}>{(name) => <code>{name}</code>}</For></div>
              </div>
              <div class="settings-actions"><button class="btn primary" onClick={() => void openProjectConfig()}>Configure hooks in config.toml</button></div>
            </Show>

            <Show when={page() === "advanced"}>
              <header><h1>Advanced</h1><p>Inspect effective limits, paths, and configuration diagnostics.</p></header>
              <Group title="Context">
                <Row title="Context window" description="Maximum model input budget before compaction."><span class="metric">{fmt(config()?.context_window ?? 0)} tokens</span></Row>
                <Row title="Maximum output" description="Provider output-token ceiling."><span class="metric">{fmt(config()?.max_tokens ?? 0)} tokens</span></Row>
              </Group>
              <Group title="Paths">
                <Row title="Project config" description={config()?.paths.project_config ?? ""}><button class="settings-button" onClick={() => void openProjectConfig()}>Open</button></Row>
                <Row title="Global config" description={config()?.paths.global_config ?? "Not configured"}><button class="settings-button" onClick={() => void navigator.clipboard.writeText(config()?.paths.global_config ?? "")}>Copy path</button></Row>
                <Row title="Session store" description={config()?.paths.sessions_home ?? ""}><button class="settings-button" onClick={() => void navigator.clipboard.writeText(config()?.paths.sessions_home ?? "")}>Copy path</button></Row>
              </Group>
              <Show when={(config()?.warnings.length ?? 0) > 0}><Group title="Configuration warnings"><For each={config()?.warnings}>{(warning) => <div class="settings-warning">{warning}</div>}</For></Group></Show>
            </Show>
          </Show>
        </div>
      </main>
    </div>
  );
}
