import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  density,
  openInEditor,
  pendingSettingsPage,
  providers,
  setDensity,
  setNotice,
  setProviders,
  setSettingsOpen,
  settingsScope,
  setSetupNeeded,
  setShowShortcuts,
  uiPreferences,
  updateUiPreference,
  sessions,
  type Density,
} from "../store";
import type { ConfigSnapshot } from "../types";
import * as api from "../api";
import { relTime } from "../time";
import { loadHealth, refreshSessions } from "../App";
import Icon, { type IconName } from "./Icon";
import OperationsPanel from "./OperationsPanel";
import DigestCard from "./DigestCard";

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
  const scope = () => settingsScope();
  const [page, setPage] = createSignal<Page>(pendingSettingsPage() ?? "general");
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
  // Memory tiers (docs/design/29-personal-os.md P1): per-project MEMORY.md
  // vs the global USER.md profile that follows the user everywhere.
  const [tier, setTier] = createSignal<api.MemoryScope>("workspace");
  const [editingId, setEditingId] = createSignal<string | null>(null);
  const [editText, setEditText] = createSignal("");
  const [noteTag, setNoteTag] = createSignal("");
  const [noteKind, setNoteKind] = createSignal("preference");
  const [noteText, setNoteText] = createSignal("");
  const [addingNote, setAddingNote] = createSignal(false);

  const tierNotes = createMemo(() =>
    notes().filter((n) => (n.scope ?? "workspace") === tier()),
  );

  // Activity feed (docs/design/29): the newest notes across BOTH tiers, so a
  // glance answers "what has the agent been learning lately?" without
  // flipping tabs.
  const recentNotes = createMemo(() =>
    [...notes()].sort((a, b) => b.ts.localeCompare(a.ts)).slice(0, 8),
  );
  const [recentPickedId, setRecentPickedId] = createSignal<string | null>(null);

  /** Switch to the note's tier and bring its row into view below. */
  function jumpToNote(note: api.NoteBlock) {
    setTier((note.scope ?? "workspace") as api.MemoryScope);
    setRecentPickedId(note.id);
    requestAnimationFrame(() => {
      document
        .querySelector(`[data-note-id="${CSS.escape(note.id)}"]`)
        ?.scrollIntoView({ behavior: uiPreferences.reduceMotion ? "auto" : "smooth", block: "center" });
    });
  }

  async function refreshLearning() {
    try {
      setNotes((await api.listMemory()).notes);
      setProposals((await api.listProposals()).proposals);
    } catch {
      /* gateway down — lists simply stay stale */
    }
  }

  async function forgetNote(id: string) {
    if (!window.confirm("Forget this memory note? The block is removed from the markdown store; this cannot be undone.")) return;
    try {
      await api.forgetMemory(id, tier());
      await refreshLearning();
    } catch (e) {
      setNotice({ kind: "error", text: `Could not forget note: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  async function amendNote(id: string) {
    const text = editText().trim();
    if (!text) return;
    try {
      await api.amendMemory(id, tier(), text);
      setEditingId(null);
      await refreshLearning();
    } catch (e) {
      setNotice({ kind: "error", text: `Could not amend note: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  async function addMemoryNote() {
    const text = noteText().trim();
    if (!text) return;
    try {
      await api.appendMemory(tier(), text, noteKind().trim() || "fact", noteTag().trim());
      setNoteText("");
      setNoteTag("");
      setAddingNote(false);
      await refreshLearning();
      setNotice({ kind: "info", text: tier() === "profile" ? "Profile note saved — recalled in every project." : "Workspace note saved." });
    } catch (e) {
      setNotice({ kind: "error", text: `Could not append note: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  // MCP manager state: loaded when the integrations page opens; edits are
  // local until Save pushes the whole table.
  const [mcpServers, setMcpServers] = createSignal<Record<string, api.McpServerDef> | null>(null);
  const [mcpDirty, setMcpDirty] = createSignal(false);
  const [mcpSaving, setMcpSaving] = createSignal(false);
  const [skills, setSkills] = createSignal<api.DiscoveredSkill[]>([]);
  const [hooks, setHooks] = createSignal<api.HookConfig[]>([]);
  const [hooksDirty, setHooksDirty] = createSignal(false);
  const [hooksSaving, setHooksSaving] = createSignal(false);
  const [capabilityTab, setCapabilityTab] = createSignal<"mcp" | "skills" | "hooks">("mcp");

  async function refreshMcp() {
    try {
      const res = scope() === "user" ? await api.getGlobalMcpServers() : await api.getMcpServers();
      setMcpServers(res.servers ?? {});
      setMcpDirty(false);
    } catch (e) {
      setNotice({ kind: "error", text: `Could not load MCP servers: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  async function refreshCapabilities() {
    try {
      const [skillResult, hookResult] = await Promise.all([api.listSkills(), scope() === "user" ? api.getGlobalHooks() : api.getHooks()]);
      setSkills(skillResult.skills ?? []);
      setHooks(hookResult.hooks ?? []);
      setHooksDirty(false);
    } catch (e) {
      setNotice({ kind: "error", text: `Could not load capabilities: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  async function saveHooks() {
    setHooksSaving(true);
    try {
      if (scope() === "user") await api.putGlobalHooks(hooks());
      else await api.putHooks(hooks());
      setHooksDirty(false);
      setNotice({ kind: "info", text: scope() === "user" ? `Saved ${hooks().length} shared hook${hooks().length === 1 ? "" : "s"} — inherited by projects` : `Saved ${hooks().length} project hook${hooks().length === 1 ? "" : "s"} — active for new turns` });
    } catch (e) {
      setNotice({ kind: "error", text: e instanceof Error ? e.message : String(e) });
    } finally {
      setHooksSaving(false);
    }
  }

  function updateHook(index: number, patch: Partial<api.HookConfig>) {
    setHooks((current) => current.map((hook, i) => i === index ? { ...hook, ...patch } : hook));
    setHooksDirty(true);
  }

  function addHook() {
    setHooks((current) => [...current, { event: "pre_tool_use", matcher: "", command: "", timeout_ms: 10000, enabled: true }]);
    setHooksDirty(true);
  }

  function removeHook(index: number) {
    setHooks((current) => current.filter((_, i) => i !== index));
    setHooksDirty(true);
  }

  async function saveMcp() {
    const servers = mcpServers() ?? {};
    setMcpSaving(true);
    try {
      if (scope() === "user") await api.putGlobalMcpServers(servers);
      else await api.putMcpServers(servers);
      setMcpDirty(false);
      setNotice({ kind: "info", text: scope() === "user" ? `Saved ${Object.keys(servers).length} shared MCP server(s) — inherited by new projects and applied here now` : `Saved ${Object.keys(servers).length} project MCP server(s) — applied to new turns` });
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
  // Per-surface token drafts, keyed by surface name — one Telegram-shaped
  // field per chat bridge (docs/design/34 Phase 3) instead of three copies.
  const [tokenDraft, setTokenDraft] = createSignal<Record<string, string>>({});
  const [tokenBusy, setTokenBusy] = createSignal<string | null>(null);

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
    void refreshCapabilities();
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
    if (!window.confirm(`Delete “${task?.title || "Untitled task"}”? This cannot be undone in Vak.`)) return;
    try {
      await api.deleteSession(id);
      await refreshSessions();
      setNotice({ kind: "info", text: "Task deleted from history." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not delete that task: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  const deleteAllArchived = async () => {
    if (!archivedSessions().length || !window.confirm(`Delete all ${archivedSessions().length} archived tasks? This cannot be undone in Vak.`)) return;
    try {
      const result = await api.deleteAllArchived();
      await refreshSessions();
      setNotice({ kind: "info", text: `${result.deleted} archived task${result.deleted === 1 ? "" : "s"} deleted.` });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not delete archived tasks: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  // ---- Data & backup (docs/design/29-personal-os.md P3) ----------------------
  const [backupDir, setBackupDir] = createSignal("");
  const [includeSecrets, setIncludeSecrets] = createSignal(false);
  const [importDir, setImportDir] = createSignal("");
  const [conflict, setConflict] = createSignal<"skip" | "rename">("skip");
  const [backupBusy, setBackupBusy] = createSignal(false);

  const pickDirectory = async (current: string): Promise<string> => {
    try {
      const dir = await openFileDialog({ directory: true, multiple: false, title: "Choose a folder", defaultPath: current || undefined });
      if (typeof dir === "string") return dir;
    } catch {
      /* no native dialog in this environment — the text input remains */
    }
    return current;
  };

  const runExport = async () => {
    const dest = backupDir().trim();
    if (!dest) return;
    setBackupBusy(true);
    try {
      const res = await api.backupExport(dest, includeSecrets());
      const m = res.manifest;
      setNotice({
        kind: "info",
        text: `Exported ${m.file_count} file${m.file_count === 1 ? "" : "s"} (${(m.total_bytes / 1024).toFixed(0)} KB) to ${dest}${res.included_secrets ? " — INCLUDING SECRETS" : ""}`,
      });
    } catch (error) {
      setNotice({ kind: "error", text: `Export failed: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setBackupBusy(false);
    }
  };

  const runImport = async () => {
    const src = importDir().trim();
    if (!src) return;
    setBackupBusy(true);
    try {
      const report = await api.backupImport(src, conflict());
      setNotice({
        kind: "info",
        text: `Imported from ${src} — ${report.copied} copied · ${report.renamed} renamed · ${report.skipped} skipped.`,
      });
    } catch (error) {
      setNotice({ kind: "error", text: `Import failed: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setBackupBusy(false);
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
      if (scope() === "user") await api.patchGlobalConfig({ provider: provider(), model: model(), max_turns: maxTurns() });
      else await api.patchConfig({ provider: provider(), model: model(), max_turns: maxTurns() });
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

  const draftFor = (surface: string) => tokenDraft()[surface];

  const setDraftFor = (surface: string, value: string | null) =>
    setTokenDraft((prev) => {
      const next = { ...prev };
      if (value === null) delete next[surface];
      else next[surface] = value;
      return next;
    });

  const saveBotToken = async (surface: string, managed: boolean) => {
    const draft = draftFor(surface)?.trim();
    if (!draft) return;
    setTokenBusy(surface);
    try {
      const res = await api.putBotToken(surface, draft);
      setDraftFor(surface, null);
      await load();
      setNotice({
        kind: "info",
        text: res.restarted
          ? `${surface} token stored locally (${res.env_var}) and the bridge was restarted with it.`
          : managed
            ? `${surface} token stored locally (${res.env_var}). The bridge isn't installed as a service yet — start it from Operations to use it.`
            : `${surface} token stored locally (${res.env_var}). Start the bridge with \`vak ${surface} --server <gateway-url>\` to use it.`,
      });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not store ${surface} token: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setTokenBusy(null);
    }
  };

  const removeBotToken = async (surface: string) => {
    setTokenBusy(surface);
    try {
      const res = await api.removeBotToken(surface);
      await load();
      setNotice({
        kind: "info",
        text: res.shadowed_by_env
          ? `Removed from ~/.vak/.env, but ${res.env_var} is still set in the real environment.`
          : res.restarted
            ? `${surface} token removed and the bridge was restarted.`
            : `${surface} token removed.`,
      });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not remove ${surface} token: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setTokenBusy(null);
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
      if (scope() === "user") await api.patchGlobalConfig({ permission_mode: wire });
      else await api.patchConfig({ permission_mode: wire });
      setConfig((current) => current ? { ...current, permission_mode: mode } : current);
      await loadHealth();
    } catch (error) {
      setNotice({ kind: "error", text: `Could not update permissions: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  const openProjectConfig = async () => {
    const relative = ".vak/config.toml";
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
        <button class="settings-back" onClick={() => setSettingsOpen(false)}><Icon name="chevron" /><span>Back to Vak</span></button>
        <div class="settings-search"><Icon name="search" /><input aria-label="Search settings" placeholder="Search settings…" value={query()} onInput={(event) => setQuery(event.currentTarget.value)} /></div>
        <div class="settings-nav-label">{scope() === "user" ? "User settings" : "Project settings"}</div>
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
        <div class="settings-nav-foot"><div class="settings-app-mark"><img src="/vak-icon.png" alt="" /></div><div><strong>Vak</strong><span>Version 0.2.0</span></div></div>
      </aside>

      <main class="settings-main">
        <div class="settings-content">
          <Show when={!loading()} fallback={<div class="settings-loading"><span /><span /><span /></div>}>
            <Show when={page() === "general"}>
              <header><h1>{scope() === "user" ? "User settings" : "Project settings"}</h1><p>{scope() === "user" ? "Shared defaults and capabilities inherited by your projects." : "Overrides for this folder. Unchanged settings inherit your user defaults."}</p></header>
              <Group title="Experience">
                <Row title="Desktop notifications" description="Notify when the active task finishes while Vak is in the background."><Switch label="Desktop notifications" checked={uiPreferences.notifications} onChange={(value) => updateUiPreference("notifications", value)} /></Row>
                <Row title="Sound cues" description="Short chime when a task starts working and when it finishes."><Switch label="Sound cues" checked={uiPreferences.soundCues} onChange={(value) => updateUiPreference("soundCues", value)} /></Row>
                <Row title="Suggested prompts" description="Show useful starting points when a task has no conversation yet."><Switch label="Suggested prompts" checked={uiPreferences.suggestions} onChange={(value) => updateUiPreference("suggestions", value)} /></Row>
                <Row title="Transcript detail" description="Control how much agent activity appears in conversations."><select value={density()} onChange={(event) => setDensity(event.currentTarget.value as Density)}><option value="outcome">Outcome</option><option value="balanced">Balanced</option><option value="audit">Audit</option></select></Row>
                <Row title="Keyboard shortcuts" description="See every shortcut for navigation, tasks, and workspace tools."><button class="settings-button" onClick={() => { setSettingsOpen(false); setShowShortcuts(true); }}>View shortcuts</button></Row>
              </Group>
              <Group title="Voice">
                <Row title="Speak agent actions out loud" description="Narrate turn completions and permission prompts through the voice pipeline."><Switch label="Speak agent actions out loud" checked={uiPreferences.voiceEnabled} onChange={(value) => updateUiPreference("voiceEnabled", value)} /></Row>
                <Show when={uiPreferences.voiceEnabled}>
                  <Row title="Voice" description="The synthesized voice used for narration."><select value={uiPreferences.voiceName} onChange={(event) => updateUiPreference("voiceName", event.currentTarget.value)}><option value="Kore">Kore</option><option value="Puck">Puck</option><option value="Zephyr">Zephyr</option><option value="Charon">Charon</option><option value="Fenrir">Fenrir</option><option value="Aoede">Aoede</option></select></Row>
                  <Row title="Persona" description="Optional style directive for how narration sounds."><input value={uiPreferences.voicePersona} placeholder="e.g. calm and concise" onInput={(event) => updateUiPreference("voicePersona", event.currentTarget.value)} /></Row>
                </Show>
              </Group>
              <Group title={scope() === "user" ? "Desktop" : "Project"}>
                <Row title={scope() === "user" ? "Inheritance" : "Current workspace"} description={scope() === "user" ? "Projects inherit this scope unless their project settings override a value." : (config()?.paths.cwd ?? "")}><span class="settings-value">{scope() === "user" ? "Shared" : "Local"}</span></Row>
                <Show when={scope() === "project"}><Row title="Project configuration" description="Persistent agent and tool settings for this repository."><button class="settings-button" onClick={() => void openProjectConfig()}>Open config</button></Row></Show>
              </Group>
            </Show>

            <Show when={page() === "archived"}>
              <header class="archived-header"><div><h1>Archived tasks</h1><p>Hidden from the sidebar until you restore them.</p></div><button class="settings-button danger" disabled={!archivedSessions().length} onClick={() => void deleteAllArchived()}><Icon name="trash" size={14} /> Delete all</button></header>
              <div class="settings-callout"><Icon name="archive" /><div><strong>Archive is reversible</strong><span>Restore a task any time. Deleting removes it from Vak’s task history; the append-only session ledger remains untouched on disk.</span></div></div>
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
                <Row title="Rich output" description="Render link previews, metrics, media, and other typed presentation items."><Switch label="Rich output" checked={uiPreferences.richPreviews} onChange={(value) => updateUiPreference("richPreviews", value)} /></Row>
                <Row title="External media" description="Allow safe images and media from approved HTTP(S) sources."><Switch label="External media" checked={uiPreferences.externalMedia} onChange={(value) => updateUiPreference("externalMedia", value)} /></Row>
                <Row title="Autoplay media" description="Never enabled by default; turn on only for trusted media sources."><Switch label="Autoplay media" checked={uiPreferences.autoplayMedia} onChange={(value) => updateUiPreference("autoplayMedia", value)} /></Row>
                <Row title="Experimental skills" description="Allow sandboxed, not-yet-promoted presentation skills to render with fallback diagnostics."><Switch label="Experimental skills" checked={uiPreferences.experimentalSkills} onChange={(value) => updateUiPreference("experimentalSkills", value)} /></Row>
              </Group>
            </Show>

            <Show when={page() === "agent"}>
              <header><h1>Agent</h1><p>Configure the model used when starting new tasks.</p></header>
              <div class="settings-callout"><Icon name="spark" /><div><strong>Saved workspace defaults</strong><span>Applied changes are persisted to this workspace and take effect for new tasks. Existing tasks retain their frozen provider/model contract.</span></div></div>
              <Group title="Model">
                <Row title="Provider" description={`${currentProviderInfo()?.env_var ? `Authenticated via ${currentProviderInfo()?.env_var}` : "The API provider used for new sessions."} · saved source: ${config()?.provider_source ?? "unknown"}`}>
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
                  description={catalogNote() ?? `${catalog().length} models available for this key. Saved source: ${config()?.model_source ?? "unknown"}.`}
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
                      ? `${currentProviderInfo()?.configured ? "Saved on this device" : "Not set yet"} · stored in ~/.vak/.env with owner-only permissions. A real environment variable takes precedence.`
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
                {/* docs/design/34 Phase 3: one token field per chat
                    surface, all stored the same way. Falls back to the
                    Telegram-only shape when talking to an older server. */}
                <For each={config()?.chat_surfaces ?? [{ surface: "telegram" as const, env_var: config()?.telegram.env_var ?? "TELEGRAM_BOT_TOKEN", configured: !!config()?.telegram.configured, managed_service: true }]}>{(chat) => (
                <Row
                  title={`${chat.surface} bridge — ${chat.env_var}`}
                  description={`${chat.configured ? "Saved on this device" : "Not set yet"} · stored in ~/.vak/.env with owner-only permissions. ${chat.managed_service ? "Saving restarts the bridge automatically." : `Start it with \`vak ${chat.surface} --server <gateway-url>\` after saving.`}`}
                >
                  <Show
                    when={draftFor(chat.surface) === undefined}
                    fallback={
                      <span class="key-edit">
                        <input type="password" autocomplete="off" spellcheck={false} placeholder={`paste ${chat.surface} bot token`} aria-label={`${chat.surface} bot token`} value={draftFor(chat.surface) ?? ""} onInput={(e) => setDraftFor(chat.surface, e.currentTarget.value)} onKeyDown={(e) => e.key === "Enter" && void saveBotToken(chat.surface, chat.managed_service)} />
                        <button class="btn primary sm" disabled={tokenBusy() === chat.surface || !draftFor(chat.surface)?.trim()} onClick={() => void saveBotToken(chat.surface, chat.managed_service)}>{tokenBusy() === chat.surface ? "Saving…" : "Save"}</button>
                        <button class="settings-button" onClick={() => setDraftFor(chat.surface, null)}>Cancel</button>
                      </span>
                    }
                  >
                    <span class="key-edit">
                      <button class="settings-button" onClick={() => setDraftFor(chat.surface, "")}>{chat.configured ? "Replace token" : "Add token"}</button>
                      <Show when={chat.configured}>
                        <button class="settings-button danger" disabled={tokenBusy() === chat.surface} onClick={() => void removeBotToken(chat.surface)}>Remove token</button>
                      </Show>
                    </span>
                  </Show>
                </Row>
                )}</For>
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
              <header><h1>Reliability</h1><p>Understand how Vak recovers from provider and task failures.</p></header>
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
              <OperationsPanel onNotice={(text) => setNotice({ kind: "error", text })} />
              <DigestCard />
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
              <Show when={recentNotes().length > 0}>
                <section class="memory-recent" aria-label="Recent memory activity">
                  <span class="memory-recent-label">Recent</span>
                  <div class="memory-recent-chips">
                    <For each={recentNotes()}>
                      {(n) => (
                        <button
                          class="memory-recent-chip"
                          classList={{ picked: recentPickedId() === n.id }}
                          title={n.text}
                          onClick={() => jumpToNote(n)}
                        >
                          <span class="badge">{n.kind}</span>
                          <Show when={(n.scope ?? "workspace") === "profile"}>
                            <span class="memory-recent-scope">profile</span>
                          </Show>
                          <span class="memory-recent-time">{relTime(n.ts)}</span>
                        </button>
                      )}
                    </For>
                  </div>
                </section>
              </Show>
              <nav class="capability-tabs memory-tabs" aria-label="Memory tiers">
                <button classList={{ active: tier() === "workspace" }} onClick={() => setTier("workspace")}><Icon name="folder" /><span>Workspace</span><em>{notes().filter((n) => (n.scope ?? "workspace") === "workspace").length}</em></button>
                <button classList={{ active: tier() === "profile" }} onClick={() => setTier("profile")}><Icon name="spark" /><span>Profile</span><em>{notes().filter((n) => n.scope === "profile").length}</em></button>
              </nav>
              <Group title={tier() === "profile" ? "Profile memories (USER.md)" : "Workspace memories (MEMORY.md)"}>
                <Show when={tier() === "profile"}>
                  <p class="settings-hint memory-hint">Global tier — these notes are recalled in every project.</p>
                </Show>
                <Show
                  when={!addingNote()}
                  fallback={
                    <div class="memory-add">
                      <div class="memory-add-fields">
                        <label>Kind<input value={noteKind()} aria-label="Note kind" onInput={(e) => setNoteKind(e.currentTarget.value)} /></label>
                        <label>Tag <span class="label-hint">optional</span><input value={noteTag()} aria-label="Note tag" onInput={(e) => setNoteTag(e.currentTarget.value)} /></label>
                      </div>
                      <textarea rows={2} placeholder={tier() === "profile" ? "Something that should hold across every project…" : "Something that should hold in this workspace…"} aria-label="Note text" value={noteText()} onInput={(e) => setNoteText(e.currentTarget.value)} />
                      <div class="task-add-row">
                        <button class="btn primary" disabled={!noteText().trim() || !noteKind().trim()} onClick={() => void addMemoryNote()}>Append note</button>
                        <button class="btn" onClick={() => setAddingNote(false)}>Cancel</button>
                      </div>
                    </div>
                  }
                >
                  <div class="task-add-row"><button class="btn" onClick={() => setAddingNote(true)}><Icon name="add" /> Add {tier() === "profile" ? "profile" : "workspace"} note</button></div>
                </Show>
                <Show
                  when={tierNotes().length > 0}
                  fallback={<Row title={tier() === "profile" ? "No profile notes yet" : "No notes yet"} description={tier() === "profile" ? "Add a preference once and every workspace benefits." : "Chat with reflection enabled — durable decisions land here as plain markdown you can edit in ~/.vak/memory/."}><span class="settings-status good">{tier() === "profile" ? "Ready" : "Ready"}</span></Row>}
                >
                  <div class="archived-list" aria-label="Memory notes">
                    <For each={tierNotes().slice().reverse()}>
                      {(n) => (
                        <div class="task-row memory-note" data-note-id={n.id} classList={{ picked: recentPickedId() === n.id }}>
                          <div class="task-main">
                            <div class="task-name">
                              <span class="badge memory-id" title={`Note id ${n.id}`}>{n.id.slice(0, 8)}</span>
                              {n.tag || n.kind}
                              <span class="badge">{n.kind}</span>
                              <Show when={n.tag && n.tag !== n.kind}><span class="badge">tag: {n.tag}</span></Show>
                            </div>
                            <small class="memory-meta">{new Date(n.ts).toLocaleString()} · from {n.session_id.slice(0, 8)}</small>
                            <Show
                              when={editingId() === n.id}
                              fallback={<p class="memory-text">{n.text}</p>}
                            >
                              <textarea
                                class="memory-amend"
                                rows={3}
                                aria-label={`Amend note ${n.id.slice(0, 8)}`}
                                value={editText()}
                                onInput={(e) => setEditText(e.currentTarget.value)}
                              />
                              <div class="task-add-row">
                                <button class="btn primary sm" disabled={!editText().trim()} onClick={() => void amendNote(n.id)}>Save</button>
                                <button class="btn sm" onClick={() => setEditingId(null)}>Cancel</button>
                              </div>
                            </Show>
                          </div>
                          <div class="task-actions">
                            <Show when={editingId() !== n.id}>
                              <button class="chip sm" onClick={() => { setEditingId(n.id); setEditText(n.text); }}>amend</button>
                            </Show>
                            <button class="chip sm danger-chip" onClick={() => void forgetNote(n.id)}>forget</button>
                          </div>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </Group>
            </Show>

            <Show when={page() === "integrations"}>
              <header><h1>Capabilities</h1><p>{scope() === "user" ? "Shared capabilities inherited by every project. Secrets stay in the protected user store." : "This project's effective capabilities. Add or change a server here only when this project needs an override."}</p></header>
              <nav class="capability-tabs" aria-label="Capability types">
                <button classList={{ active: capabilityTab() === "mcp" }} onClick={() => setCapabilityTab("mcp")}><Icon name="plug" /><span>MCP servers</span><em>{Object.keys(mcpServers() ?? {}).length}</em></button>
                <button classList={{ active: capabilityTab() === "skills" }} onClick={() => setCapabilityTab("skills")}><Icon name="spark" /><span>Skills</span><em>{skills().length}</em></button>
                <button classList={{ active: capabilityTab() === "hooks" }} onClick={() => setCapabilityTab("hooks")}><Icon name="tune" /><span>Hooks</span><em>{hooks().length}</em></button>
              </nav>
              <Show when={capabilityTab() === "mcp"}>
                <Group title="Tool servers"><Show when={mcpServers()} fallback={<Row title="Loading servers…" description="Reading the effective MCP configuration."><span /></Row>}>
                  <div class="mcp-editor">
                    <Show when={Object.keys(mcpServers() ?? {}).length > 0} fallback={<div class="capability-empty"><Icon name="plug" /><strong>No MCP servers connected</strong><span>Add a local server to give the agent tools such as search, browser, or data access.</span></div>}>
                      <For each={Object.entries(mcpServers() ?? {})}>{([name, def]) => <div class="mcp-row">
                        <div class="mcp-row-head"><div><strong>{name}</strong><span class="capability-state ready">Configured</span></div><button class="settings-button danger" onClick={() => removeServer(name)}><Icon name="trash" /> Remove</button></div>
                        <div class="mcp-fields"><label>Server name<input value={name} aria-label="Server name" onChange={(e) => renameServer(name, e.currentTarget.value.trim())} /></label><label>Command<input placeholder="/path/to/command" value={def.command} aria-label="Command" onInput={(e) => updateServer(name, { command: e.currentTarget.value })} /></label><label>Arguments<input placeholder="Space-separated arguments" value={def.args.join(" ")} aria-label="Arguments" onInput={(e) => updateServer(name, { args: e.currentTarget.value.split(" ").filter(Boolean) })} /></label></div>
                        <div class="mcp-controls"><label class="mcp-network"><Switch checked={def.network} label={`Allow network for ${name}`} onChange={(v) => updateServer(name, { network: v })} /><span>Allow outbound network</span></label><button class="settings-button" onClick={() => setNotice({ kind: "info", text: "MCP health checks run when the server is first used in a task." })}>Check on next use</button></div>
                      </div>}</For>
                    </Show>
                    <div class="settings-actions"><button class="btn" onClick={addServer}><Icon name="add" /> Add server</button><button class="btn primary" disabled={!mcpDirty() || mcpSaving()} onClick={() => void saveMcp()}>{mcpSaving() ? "Saving…" : "Save & apply"}</button><Show when={mcpDirty()}><span class="mcp-dirty">Unsaved changes</span></Show></div>
                    <p class="settings-hint">{scope() === "user" ? "These are user-owned server definitions, shared by projects. Secrets are referenced by name and never shown here." : "Project definitions override a same-named shared server. Network access is off unless explicitly enabled; secrets remain user-owned."}</p>
                  </div>
                </Show></Group>
              </Show>
              <Show when={capabilityTab() === "skills"}>
                <Group title={`Discovered skills (${skills().length})`}><Show when={skills().length > 0} fallback={<div class="capability-empty"><Icon name="spark" /><strong>No skills discovered</strong><span>Add a SKILL.md under the project or your Vak home directory, then reload this page.</span></div>}><p class="settings-hint">Skills are source-controlled instruction files. Their displayed scope is real: user skills are inherited; project skills belong only to this folder.</p><div class="capability-list"><For each={skills()}>{(skill) => <details class="capability-item"><summary><span><strong>{skill.name}</strong><small>{skill.scope ?? "Project or user"}</small></span><span class="capability-state ready">Available</span></summary><div class="capability-detail"><p>{skill.description || "No description provided."}</p><Show when={skill.source}><code>{skill.source}</code></Show><button class="settings-button" onClick={() => setNotice({ kind: "info", text: `${skill.name} is available from the task composer.` })}>Use in a task</button></div></details>}</For></div></Show></Group>
                <Group title={`Pending proposals (${proposals().length})`}><Show when={proposals().length > 0} fallback={<Row title="No proposals waiting" description="The agent can suggest reusable skills; they stay inactive until you review them."><span class="settings-status good">Clear</span></Row>}><For each={proposals()}>{(proposal) => <div class="setting-row"><div class="setting-copy"><strong>{proposal.name}</strong><span>{proposal.description}</span></div><div class="setting-control"><button class="settings-button" onClick={() => void promote(proposal.id)}>Review & promote</button><button class="settings-button danger" onClick={() => void reject(proposal.id)}>Reject</button></div></div>}</For></Show></Group>
              </Show>
              <Show when={capabilityTab() === "hooks"}>
                <Group title="Lifecycle automation"><div class="settings-callout"><Icon name="shield" /><div><strong>Hooks run commands at controlled lifecycle points.</strong><span>Keep commands short and review every change. A disabled hook never runs.</span></div></div><Show when={hooks().length > 0} fallback={<div class="capability-empty"><Icon name="tune" /><strong>No hooks configured</strong><span>Add a hook to run a safe, repeatable action at session or tool lifecycle events.</span></div>}><div class="hook-editor"><For each={hooks()}>{(hook, index) => <div class="hook-row"><div class="hook-row-head"><span class="capability-state" classList={{ ready: hook.enabled !== false, muted: hook.enabled === false }}>{hook.enabled === false ? "Disabled" : "Enabled"}</span><Switch checked={hook.enabled !== false} label={`Enable hook ${index() + 1}`} onChange={(v) => updateHook(index(), { enabled: v })} /><button class="settings-button danger" onClick={() => removeHook(index())}><Icon name="trash" /></button></div><label>Lifecycle event<select value={hook.event} onChange={(e) => updateHook(index(), { event: e.currentTarget.value })}><option value="session_start">Session start</option><option value="pre_tool_use">Before a tool runs</option><option value="post_tool_use">After a tool runs</option><option value="stop">Task stop</option></select></label><label>Tool matcher <span class="label-hint">optional</span><input value={hook.matcher ?? ""} placeholder="bash, write, or leave blank" onInput={(e) => updateHook(index(), { matcher: e.currentTarget.value })} /></label><label>Command<input class="hook-command" value={hook.command} placeholder="e.g. cargo fmt --all --check" onInput={(e) => updateHook(index(), { command: e.currentTarget.value })} /></label><label>Timeout (ms)<input type="number" min="100" max="120000" value={hook.timeout_ms ?? 10000} onInput={(e) => updateHook(index(), { timeout_ms: Number(e.currentTarget.value) || 10000 })} /></label></div>}</For></div></Show><div class="settings-actions"><button class="btn" onClick={addHook}><Icon name="add" /> Add hook</button><button class="btn primary" disabled={!hooksDirty() || hooksSaving()} onClick={() => void saveHooks()}>{hooksSaving() ? "Saving…" : "Save hooks"}</button><Show when={hooksDirty()}><span class="mcp-dirty">Unsaved changes</span></Show></div></Group>
              </Show>
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
              <Group title="Data & backup">
                <div class="settings-callout"><Icon name="shield" /><div><strong>Backups copy your Vak home.</strong><span>Sessions, memory, config, and checkpoints go to a plain folder you choose. Secrets are excluded unless you explicitly opt in below.</span></div></div>
                <Row
                  title="Export backup"
                  description="Pick a destination folder (or type a path), then export."
                >
                  <span class="key-edit">
                    <input
                      class="settings-input wide"
                      placeholder="/path/to/backup-folder"
                      aria-label="Backup destination folder"
                      value={backupDir()}
                      onInput={(e) => setBackupDir(e.currentTarget.value)}
                    />
                    <button class="settings-button" disabled={backupBusy()} onClick={async () => setBackupDir(await pickDirectory(backupDir()))}>Choose…</button>
                  </span>
                </Row>
                <Row
                  title="Include secrets"
                  description="Adds provider API keys from .env files. Anyone with this folder can spend your credits — keep it offline and delete it when restored."
                  danger
                >
                  <Switch label="Include secrets in export" checked={includeSecrets()} onChange={setIncludeSecrets} />
                </Row>
                <Show when={includeSecrets()}>
                  <div class="settings-warning" role="alert">The export will contain live API keys. Treat the folder like a password vault.</div>
                </Show>
                <div class="settings-actions">
                  <button class="btn primary" disabled={backupBusy() || !backupDir().trim()} onClick={() => void runExport()}>{backupBusy() ? "Working…" : "Export now"}</button>
                </div>
                <Row
                  title="Import backup"
                  description={`Restore a previously exported folder. Conflicts are ${conflict() === "skip" ? "skipped" : "renamed"} — the running home is never overwritten silently.`}
                >
                  <span class="key-edit">
                    <input
                      class="settings-input wide"
                      placeholder="/path/to/exported-folder"
                      aria-label="Backup source folder"
                      value={importDir()}
                      onInput={(e) => setImportDir(e.currentTarget.value)}
                    />
                    <button class="settings-button" disabled={backupBusy()} onClick={async () => setImportDir(await pickDirectory(importDir()))}>Choose…</button>
                    <select value={conflict()} onChange={(e) => setConflict(e.currentTarget.value as "skip" | "rename")} aria-label="Conflict policy">
                      <option value="skip">skip existing</option>
                      <option value="rename">rename incoming</option>
                    </select>
                  </span>
                </Row>
                <div class="settings-actions">
                  <button class="btn" disabled={backupBusy() || !importDir().trim()} onClick={() => void runImport()}>{backupBusy() ? "Working…" : "Import"}</button>
                </div>
              </Group>
              <Show when={(config()?.warnings.length ?? 0) > 0}><Group title="Configuration warnings"><For each={config()?.warnings}>{(warning) => <div class="settings-warning">{warning}</div>}</For></Group></Show>
            </Show>
          </Show>
        </div>
      </main>
    </div>
  );
}
