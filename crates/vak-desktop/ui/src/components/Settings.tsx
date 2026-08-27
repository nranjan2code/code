import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
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
  setSetupNeeded,
  setShowShortcuts,
  uiPreferences,
  updateUiPreference,
  type Density,
} from "../store";
import type { ConfigSnapshot } from "../types";
import * as api from "../api";
import { relTime } from "../time";
import { loadHealth } from "../App";
import Icon, { type IconName } from "./Icon";
import OperationsPanel from "./OperationsPanel";

/** Sentinel option that swaps the model select for a free-text field. */
const CUSTOM_MODEL = "\u0000custom";

type Page = "general" | "appearance" | "agent" | "permissions" | "reliability" | "skills" | "services" | "learning" | "advanced";

const pages: { id: Page; label: string; icon: IconName; hint: string }[] = [
  { id: "general", label: "General", icon: "gear", hint: "notifications suggestions" },
  { id: "appearance", label: "Appearance", icon: "palette", hint: "theme text density motion" },
  { id: "agent", label: "Agent", icon: "spark", hint: "provider model turns" },
  { id: "permissions", label: "Permissions", icon: "shield", hint: "access sandbox approvals" },
  { id: "reliability", label: "Reliability", icon: "timer", hint: "failures cancellation guarantees" },
  { id: "skills", label: "Skills", icon: "spark", hint: "discovered skills proposals" },
  { id: "services", label: "Services", icon: "grid", hint: "gateway bridge tray watchdog background" },
  { id: "learning", label: "Learning", icon: "history", hint: "memory notes skill proposals review promote" },
  { id: "advanced", label: "Advanced", icon: "tune", hint: "context configuration" },
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
  const [page, setPage] = createSignal<Page>(pendingSettingsPage() ?? "general");
  const [query, setQuery] = createSignal("");
  const [config, setConfig] = createSignal<ConfigSnapshot | null>(null);
  const [loading, setLoading] = createSignal(true);
  // Background-service states; polled while
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

  async function addProfileNote() {
    const text = noteText().trim();
    if (!text) return;
    try {
      await invoke<{ id: string; ts: string }>("append_profile_note", {
        draft: { kind: noteKind().trim() || "preference", tag: noteTag().trim(), text },
      });
      setNoteText("");
      setNoteTag("");
      setAddingNote(false);
      await refreshLearning();
      setNotice({ kind: "info", text: "Note appended to your USER.md profile — recalled in every project." });
    } catch (e) {
      setNotice({ kind: "error", text: `Could not append note: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  const [skills, setSkills] = createSignal<api.DiscoveredSkill[]>([]);

  async function refreshCapabilities() {
    try {
      const skillResult = await api.listSkills();
      setSkills(skillResult.skills ?? []);
    } catch (e) {
      setNotice({ kind: "error", text: `Could not load capabilities: ${e instanceof Error ? e.message : String(e)}` });
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

  async function runOp(service: "gateway", action: "start" | "stop" | "restart" | "install" | "uninstall") {
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
    if (page() !== "skills") return;
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
  });

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
    const relative = ".vakcoder/project.toml";
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
                <Row title="Current workspace" description="The registered project selected by this desktop session."><span class="settings-value">Local</span></Row>
                <Row title="Project configuration" description="Persistent agent and tool settings for this repository."><button class="settings-button" onClick={() => void openProjectConfig()}>Open config</button></Row>
              </Group>
            </Show>

            <Show when={page() === "appearance"}>
              <header><h1>Appearance</h1><p>Make the workspace comfortable for long sessions.</p></header>
              <Group title="Theme">
                <div class="theme-grid"><For each={[{ id: "warm", label: "Warm dark" }, { id: "dark", label: "Midnight" }, { id: "contrast", label: "High contrast" }] as const}>{(theme) => <button class="theme-choice" classList={{ active: uiPreferences.theme === theme.id }} onClick={() => updateUiPreference("theme", theme.id)}><span class={`theme-preview ${theme.id}`}><i /><i /><i /></span><strong>{theme.label}</strong><Show when={uiPreferences.theme === theme.id}><Icon name="check" /></Show></button>}</For></div>
              </Group>
              <Group title="Layout and text">
                <Row title="Text size" description="Conversation and interface text."><div class="range-control"><input type="range" min="90" max="120" step="5" value={uiPreferences.textScale} onInput={(event) => updateUiPreference("textScale", Number(event.currentTarget.value))} /><span>{uiPreferences.textScale}%</span></div></Row>
              <Row title="Code size" description="Code blocks, diffs, and editor labels."><div class="range-control"><input type="range" min="90" max="125" step="5" value={uiPreferences.codeScale} onInput={(event) => updateUiPreference("codeScale", Number(event.currentTarget.value))} /><span>{uiPreferences.codeScale}%</span></div></Row>
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
              </Group>
              <Group title="Credentials">
                <Row
                  title={`${keyProvider()} — ${currentProviderInfo()?.env_var ?? "no key needed"}`}
                  description={
                    currentProviderInfo()?.requires_key
                      ? `${currentProviderInfo()?.configured ? "Saved on this device" : "Not set yet"} · stored in <data_home>/.env with owner-only permissions. A real environment variable takes precedence.`
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
              <header><h1>Reliability</h1><p>Runtime keeps failures typed, cancellation explicit, and terminal outcomes durable.</p></header>
              <Group title="Run guarantees">
                <Row title="Provider and tool failures" description="Errors stay typed and do not become implicit permission grants."><span class="settings-status good">Fail closed</span></Row>
                <Row title="Cancellation" description="Provider and broker calls receive the run cancellation token; partial output is retained."><span class="settings-status good">Propagated</span></Row>
                <Row title="Terminal outcome" description="Runtime persists one terminal status even when completion and cancellation race."><span class="settings-status good">Exactly one</span></Row>
                <Row title="Capability revocation" description="Permission or sandbox changes cancel old-epoch work and reject stale approvals."><span class="settings-status good">Enforced</span></Row>
              </Group>
            </Show>

            <Show when={page() === "services"}>
              <OperationsPanel onNotice={(text) => setNotice({ kind: "error", text })} />
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
                      <textarea rows={2} placeholder="Something that should hold across every project…" aria-label="Note text" value={noteText()} onInput={(e) => setNoteText(e.currentTarget.value)} />
                      <div class="task-add-row">
                        <button class="btn primary" disabled={!noteText().trim() || !noteKind().trim()} onClick={() => void addProfileNote()}>Append note</button>
                        <button class="btn" onClick={() => setAddingNote(false)}>Cancel</button>
                      </div>
                    </div>
                  }
                >
                  <Show when={tier() === "profile"}>
                    <div class="task-add-row"><button class="btn" onClick={() => setAddingNote(true)}><Icon name="add" /> Add profile note</button></div>
                  </Show>
                </Show>
                <Show
                  when={tierNotes().length > 0}
                  fallback={<Row title={tier() === "profile" ? "No profile notes yet" : "No notes yet"} description={tier() === "profile" ? "Add a preference once and every workspace benefits." : "Chat with reflection enabled — durable decisions are stored by Runtime and remain editable through this surface."}><span class="settings-status good">{tier() === "profile" ? "Ready" : "Ready"}</span></Row>}
                >
                  <div class="memory-list" aria-label="Memory notes">
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

            <Show when={page() === "skills"}>
              <header><h1>Skills</h1><p>Review the instructions Runtime discovered for this workspace.</p></header>
              <Group title={`Discovered skills (${skills().length})`}>
                <Show when={skills().length > 0} fallback={<div class="capability-empty"><Icon name="spark" /><strong>No skills discovered</strong><span>Add a SKILL.md under the project or your VakCoder home directory, then reload this page.</span></div>}>
                  <div class="capability-list"><For each={skills()}>{(skill) => <details class="capability-item"><summary><span><strong>{skill.name}</strong><small>{skill.scope ?? "Project or user"}</small></span><span class="capability-state ready">Available</span></summary><div class="capability-detail"><p>{skill.description || "No description provided."}</p><Show when={skill.source}><code>{skill.source}</code></Show></div></details>}</For></div>
                </Show>
              </Group>
              <Group title={`Pending proposals (${proposals().length})`}>
                <Show when={proposals().length > 0} fallback={<Row title="No proposals waiting" description="No skill proposals require review."><span class="settings-status good">Clear</span></Row>}>
                  <For each={proposals()}>{(proposal) => <div class="setting-row"><div class="setting-copy"><strong>{proposal.name}</strong><span>{proposal.description}</span></div><div class="setting-control"><button class="settings-button" onClick={() => void promote(proposal.id)}>Promote</button><button class="settings-button danger" onClick={() => void reject(proposal.id)}>Reject</button></div></div>}</For>
                </Show>
              </Group>
            </Show>

            <Show when={page() === "advanced"}>
              <header><h1>Advanced</h1><p>Inspect effective limits and configuration diagnostics.</p></header>
              <Group title="Context">
                <Row title="Context limit" description="Maximum model input tokens configured for a run."><span class="metric">{fmt(config()?.context_tokens ?? 0)} tokens</span></Row>
                <Row title="Turn limit" description="Maximum agent turns configured for a run."><span class="metric">{fmt(config()?.max_turns ?? 0)}</span></Row>
                <Row title="Budget ceiling" description="Configured run budget in cents; zero means unset."><span class="metric">{fmt(config()?.budget_cents ?? 0)}¢</span></Row>
              </Group>
              <Group title="Configuration">
                <Row title="Sandbox" description="The effective containment backend is recorded in each session contract."><span class="settings-value">{config()?.sandbox}</span></Row>
                <Row title="Project config" description="Edit the registered project's .vakcoder/project.toml through the Runtime boundary."><button class="settings-button" onClick={() => void openProjectConfig()}>Open</button></Row>
              </Group>
              <Group title="Data & backup">
                <div class="settings-callout"><Icon name="shield" /><div><strong>Backups copy your VakCoder home.</strong><span>Sessions, memory, config, and checkpoints go to a plain folder you choose. Secrets are excluded unless you explicitly opt in below.</span></div></div>
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
