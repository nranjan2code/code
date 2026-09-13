import { trapFocus } from "../focusTrap";
import { createEffect, createMemo, createResource, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { host } from "../host";
import {
  density,
  openInEditor,
  pendingSettingsPage,
  providers,
  setDensity,
  setNotice,
  setProviders,
  setSettingsOpen,
  setSettingsScope,
  settingsScope,
  setShowShortcuts,
  uiPreferences,
  updateUiPreference,
  sessions,
  backend,
  type Density,
} from "../store";
import type { ConfigSnapshot } from "../types";
import * as api from "../api";
import { relTime } from "../time";
import { loadHealth, refreshSessions } from "../App";
import Icon, { type IconName } from "./Icon";
import ConfirmModal, { type ConfirmConfig } from "./ConfirmModal";
import OperationsPanel from "./OperationsPanel";
import DigestCard from "./DigestCard";
import AgentsPanel from "./AgentsPanel";

/** Sentinel option that swaps the model select for a free-text field. */
const CUSTOM_MODEL = "\u0000custom";

type Page = "general" | "appearance" | "agent" | "prompts" | "permissions" | "reliability" | "integrations" | "services" | "learning" | "advanced" | "archived";

const pages: { id: Page; label: string; icon: IconName; hint: string; group: string }[] = [
  { id: "general", label: "General", icon: "gear", hint: "notifications suggestions", group: "Experience" },
  { id: "appearance", label: "Appearance", icon: "palette", hint: "theme text density motion", group: "Experience" },
  { id: "agent", label: "Agent", icon: "spark", hint: "provider model turns subagents", group: "Agent & access" },
  { id: "prompts", label: "Prompts", icon: "spark", hint: "system prompt identity rules guardrails persona", group: "Agent & access" },
  { id: "permissions", label: "Permissions", icon: "shield", hint: "access sandbox approvals", group: "Agent & access" },
  { id: "reliability", label: "Reliability", icon: "timer", hint: "retries timeout circuit breaker", group: "Agent & access" },
  { id: "integrations", label: "Integrations", icon: "plug", hint: "mcp hooks skills", group: "Connections & automation" },
  { id: "services", label: "Services", icon: "grid", hint: "gateway bridge tray watchdog background", group: "Connections & automation" },
  { id: "learning", label: "Learning", icon: "history", hint: "memory notes skill proposals review promote", group: "Connections & automation" },
  { id: "advanced", label: "Advanced", icon: "tune", hint: "paths context configuration", group: "Advanced" },
  { id: "archived", label: "Archived tasks", icon: "archive", hint: "restore delete history", group: "Advanced" },
];

const PROMPT_BLOCKS: { id: api.PromptBlock; label: string; help: string }[] = [
  { id: "identity", label: "Identity", help: "Who the agent is. The narrowest layer that sets this wins." },
  { id: "operating-rules", label: "Operating rules", help: "How it works. The narrowest layer that sets this wins." },
  { id: "guardrails", label: "Guardrails", help: "Every layer's guardrails apply together. Nothing narrower can remove one." },
  { id: "surface-note", label: "Surface note", help: "Appended after the generated Surface line — what this deployment knows about where the reply lands. Accumulates across layers." },
];

const PROMPT_LAYER_LABELS: Record<api.PromptLayerDescriptor["layer"], string> = {
  seed: "shipped default",
  shared: "Shared",
  workspace: "This workspace",
  surface: "surface",
  bot: "bot",
  chat: "chat",
  agent: "agent role",
};

function Switch(props: { checked: boolean; onChange: (next: boolean) => void; label: string }) {
  return <button type="button" class="switch" classList={{ on: props.checked }} role="switch" aria-checked={props.checked} aria-label={props.label} onClick={() => props.onChange(!props.checked)}><span /></button>;
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

type PresentationDefinition = api.PresentationLibraryResponse["definitions"][number];
type PresentationFilter = "all" | "active" | "inactive";

type PresentationType = {
  id: string;
  label: string;
  semanticType: string;
  definitions: PresentationDefinition[];
  latest: PresentationDefinition;
  activeRevision: number | null;
  active: boolean;
};

type PresentationGroup = {
  key: string;
  label: string;
  types: PresentationType[];
  definitionCount: number;
};

function presentationLabel(value: string): string {
  return value
    .split(/[._-]/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function presentationSemanticType(definition: PresentationDefinition): string {
  return definition.spec.accepts?.[0] ?? "general";
}

function presentationGroupKey(definition: PresentationDefinition): string {
  return presentationSemanticType(definition).split(".")[0] || "general";
}

async function copySettingText(value: string, label: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(value);
    setNotice({ kind: "info", text: `${label} copied to your clipboard.` });
  } catch {
    setNotice({ kind: "error", text: `Could not copy ${label.toLowerCase()}. Clipboard access was denied.` });
  }
}

export default function Settings() {
  let settingsRoot!: HTMLDivElement;
  const scope = () => settingsScope();
  const capabilityScope = () => scope() === "user" ? "user" as const : "workspace" as const;
  const [confirmConfig, setConfirmConfig] = createSignal<ConfirmConfig | null>(null);
  const [voiceProviders] = createResource(() => api.listVoiceProviders());
  const [presentationLibrary, { refetch: refetchPresentations }] = createResource(() => api.listPresentations());
  const [presentationQuery, setPresentationQuery] = createSignal("");
  const [presentationFilter, setPresentationFilter] = createSignal<PresentationFilter>("all");
  const [presentationExpanded, setPresentationExpanded] = createSignal<Set<string>>(new Set());
  const [presentationVersions, setPresentationVersions] = createSignal<Set<string>>(new Set());
  const [presentationBusy, setPresentationBusy] = createSignal<string | null>(null);
  const presentationScope = () => capabilityScope();
  const presentationOwner = () => presentationScope() === "user" ? "user" : config()?.paths.cwd ?? backend().cwd ?? "workspace";
  const presentationActivation = (id: string) => presentationLibrary()?.activations.find((entry) => entry.spec_id === id && entry.scope === presentationScope() && entry.owner === presentationOwner());
  const presentationCatalog = createMemo<PresentationGroup[]>(() => {
    const definitions = presentationLibrary()?.definitions ?? [];
    const byId = new Map<string, PresentationDefinition[]>();
    for (const definition of definitions) {
      const entries = byId.get(definition.spec.id) ?? [];
      entries.push(definition);
      byId.set(definition.spec.id, entries);
    }
    const groups = new Map<string, PresentationType[]>();
    for (const [id, entries] of byId) {
      const versions = [...entries].sort((a, b) => b.spec.revision - a.spec.revision);
      const latest = versions[0];
      const activeRevision = presentationActivation(id)?.revision ?? null;
      const type: PresentationType = {
        id,
        label: presentationLabel(id.replace(/^seed\./, "")),
        semanticType: presentationSemanticType(latest),
        definitions: versions,
        latest,
        activeRevision,
        active: activeRevision !== null,
      };
      const key = presentationGroupKey(latest);
      const group = groups.get(key) ?? [];
      group.push(type);
      groups.set(key, group);
    }
    return [...groups.entries()]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, types]) => ({
        key,
        label: presentationLabel(key),
        types: types.sort((a, b) => a.label.localeCompare(b.label)),
        definitionCount: types.reduce((count, type) => count + type.definitions.length, 0),
      }));
  });
  const presentationGroups = createMemo<PresentationGroup[]>(() => {
    const query = presentationQuery().trim().toLowerCase();
    const filter = presentationFilter();
    return presentationCatalog()
      .map((group) => ({
        ...group,
        types: group.types.filter((type) => {
          const matchesQuery = !query || [type.id, type.label, type.semanticType].some((value) => value.toLowerCase().includes(query));
          const matchesFilter = filter === "all" || (filter === "active" ? type.active : !type.active);
          return matchesQuery && matchesFilter;
        }),
      }))
      .filter((group) => group.types.length > 0);
  });
  const activePresentationCount = () => presentationCatalog().reduce((count, group) => count + group.types.filter((type) => type.active).length, 0);
  const togglePresentationGroup = (key: string) => {
    setPresentationExpanded((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key); else next.add(key);
      return next;
    });
  };
  const togglePresentationVersions = (id: string) => {
    setPresentationVersions((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };
  const presentationBusyFor = (key: string) => presentationBusy() === key;
  async function activatePresentation(definition: PresentationDefinition) {
    try {
      await api.activatePresentation(definition.spec.id, definition.spec.revision, presentationScope(), presentationOwner());
      await refetchPresentations();
      setNotice({ kind: "info", text: `${presentationLabel(definition.spec.id.replace(/^seed\./, ""))} activated` });
    } catch (e) {
      setNotice({ kind: "error", text: `Could not change presentation: ${e instanceof Error ? e.message : String(e)}` });
    }
  }
  async function deactivatePresentation(id: string) {
    try {
      await api.deactivatePresentation(id, presentationScope(), presentationOwner());
      await refetchPresentations();
      setNotice({ kind: "info", text: `${presentationLabel(id.replace(/^seed\./, ""))} deactivated` });
    } catch (e) {
      setNotice({ kind: "error", text: `Could not change presentation: ${e instanceof Error ? e.message : String(e)}` });
    }
  }
  async function resetPresentation(id: string) {
    try {
      await api.resetPresentation(id, presentationScope(), presentationOwner());
      await refetchPresentations();
      setNotice({ kind: "info", text: "Presentation reset to its original revision" });
    } catch (e) {
      setNotice({ kind: "error", text: `Could not reset presentation: ${e instanceof Error ? e.message : String(e)}` });
    }
  }
  async function runPresentationAction(key: string, action: () => Promise<void>) {
    if (presentationBusy()) return;
    setPresentationBusy(key);
    try {
      await action();
    } finally {
      setPresentationBusy(null);
    }
  }
  async function applyPresentationType(type: PresentationType) {
    const active = presentationActivation(type.id);
    if (active?.revision === type.latest.spec.revision) await deactivatePresentation(type.id);
    else await activatePresentation(type.latest);
  }
  async function applyPresentationGroup(group: PresentationGroup) {
    const latest = group.types;
    const allLatestActive = latest.every((type) => presentationActivation(type.id)?.revision === type.latest.spec.revision);
    for (const type of latest) {
      if (allLatestActive) await deactivatePresentation(type.id);
      else if (presentationActivation(type.id)?.revision !== type.latest.spec.revision) await activatePresentation(type.latest);
    }
  }
  async function exportPresentationPack() {
    try {
      const pack = await api.exportPresentations();
      const blob = new Blob([JSON.stringify(pack, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = "vak-presentation-pack.json";
      anchor.click();
      URL.revokeObjectURL(url);
      setNotice({ kind: "info", text: "Presentation pack exported" });
    } catch (e) {
      setNotice({ kind: "error", text: `Could not export pack: ${e instanceof Error ? e.message : String(e)}` });
    }
  }
  async function importPresentationPack(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    try {
      await api.importPresentations(JSON.parse(await file.text()));
      await refetchPresentations();
      setNotice({ kind: "info", text: "Presentation pack imported as disabled previews" });
    } catch (e) {
      setNotice({ kind: "error", text: `Could not import pack: ${e instanceof Error ? e.message : String(e)}` });
    }
  }
  const discoveredVoices = () => voiceProviders()?.providers.flatMap((provider) => provider.voices) ?? [];
  async function updateVoice(patch: Record<string, unknown>) {
    try { await api.patchConfig(patch); await Promise.all([load(), loadHealth()]); setNotice({ kind: "info", text: "Voice settings saved" }); } catch (e) { setNotice({ kind: "error", text: `Could not save voice settings: ${(e as Error).message}` }); }
  }

  // Prompt layers (docs/design/45). The layer resource is keyed on scope so
  // switching Shared/This project reloads the editable layer, while the
  // effective composition is scope-independent — it is what the model gets.
  const [promptLayer, { refetch: refetchPromptLayer }] = createResource(scope, (s) => api.getPromptLayer(s));
  const [promptEffective, { refetch: refetchPromptEffective }] = createResource(() => api.getPromptEffective());
  const [promptEditing, setPromptEditing] = createSignal<api.PromptBlock | null>(null);
  const [promptDraft, setPromptDraft] = createSignal("");
  const promptLayerPath = () => promptLayer()?.path ?? "";
  const promptBlockText = (block: api.PromptBlock): string | null => {
    const l = promptLayer()?.layer;
    if (!l) return null;
    if (block === "identity") return l.identity ?? null;
    if (block === "operating-rules") return l.operating_rules ?? null;
    const rules = (block === "guardrails" ? l.guardrails : l.surface_notes) ?? [];
    return rules.length ? rules.map((r) => `- ${r}`).join("\n") : null;
  };
  const promptSources = (block: api.PromptBlock): string =>
    (promptEffective()?.layers ?? [])
      .filter((d) => d.block === block)
      .map((d) => PROMPT_LAYER_LABELS[d.layer])
      .join(", ");
  async function savePromptBlock(block: api.PromptBlock, text: string | null) {
    try {
      await api.putPromptBlock(scope(), block, text);
      setPromptEditing(null);
      await Promise.all([refetchPromptLayer(), refetchPromptEffective()]);
      setNotice({
        kind: "info",
        text: text === null
          ? `${block} reset; it is inherited again.`
          : `${block} saved. Applies to new sessions.`,
      });
    } catch (e) {
      setNotice({ kind: "error", text: `Could not save ${block}: ${(e as Error).message}` });
    }
  }
  const [page, setPage] = createSignal<Page>(pendingSettingsPage() ?? "general");
  const [query, setQuery] = createSignal("");
  onMount(() => {
    const previouslyFocused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // The modal is inserted through a lazy Suspense boundary; the shared
    // focus trap can run before the first control has a layout box. Explicitly
    // hand focus to the modal once its component is mounted.
    requestAnimationFrame(() => requestAnimationFrame(() => {
      settingsRoot.querySelector<HTMLElement>("button, input, select, textarea, [tabindex]")?.focus({ preventScroll: true });
    }));
    onCleanup(() => {
      if (previouslyFocused && document.contains(previouslyFocused)) previouslyFocused.focus({ preventScroll: true });
    });
  });
  const [config, setConfig] = createSignal<ConfigSnapshot | null>(null);
  const [evidenceAgeHours, setEvidenceAgeHours] = createSignal(24);
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
      setNotice({ kind: "info", text: tier() === "profile" ? "Profile note saved — recalled in every workspace." : "Workspace note saved." });
    } catch (e) {
      setNotice({ kind: "error", text: `Could not append note: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  // MCP manager state: loaded when the integrations page opens; edits are
  // local until Save pushes the whole table.
  const [mcpServers, setMcpServers] = createSignal<Record<string, api.McpServerDef> | null>(null);
  const [inheritedMcpServers, setInheritedMcpServers] = createSignal<Record<string, api.McpServerDef>>({});
  const [mcpDirty, setMcpDirty] = createSignal(false);
  const [mcpSaving, setMcpSaving] = createSignal(false);
  const [skills, setSkills] = createSignal<api.DiscoveredSkill[]>([]);
  const [plugins, setPlugins] = createSignal<api.InstalledPlugin[]>([]);
  const [inheritedPlugins, setInheritedPlugins] = createSignal<api.InstalledPlugin[]>([]);
  const [pluginSources, setPluginSources] = createSignal<api.MarketplaceSource[]>([]);
  const [marketplaceEntries, setMarketplaceEntries] = createSignal<api.MarketplaceEntry[]>([]);
  const [marketplaceQuery, setMarketplaceQuery] = createSignal("");
  const [sourcePath, setSourcePath] = createSignal("");
  const [sourceKeyId, setSourceKeyId] = createSignal("");
  const [sourcePublicKey, setSourcePublicKey] = createSignal("");
  const [sourceSignature, setSourceSignature] = createSignal("");
  const [pluginPath, setPluginPath] = createSignal("");
  const [pluginBusy, setPluginBusy] = createSignal(false);
  const [hooks, setHooks] = createSignal<api.HookConfig[]>([]);
  const [inheritedHooks, setInheritedHooks] = createSignal<api.HookConfig[]>([]);
  const [hooksDirty, setHooksDirty] = createSignal(false);
  const [hooksSaving, setHooksSaving] = createSignal(false);
  const [capabilityTab, setCapabilityTab] = createSignal<"mcp" | "skills" | "hooks" | "plugins">("mcp");
  const visibleSkills = createMemo(() =>
    scope() === "user" ? skills().filter((skill) => skill.scope === "user") : skills(),
  );

  const totalMcpCount = () => {
    const local = Object.keys(mcpServers() ?? {}).length;
    if (scope() === "user") return local;
    const inherited = Object.keys(inheritedMcpServers()).filter((k) => !(k in (mcpServers() ?? {}))).length;
    return local + inherited;
  };

  const totalHooksCount = () => {
    const local = hooks().length;
    if (scope() === "user") return local;
    return local + inheritedHooks().length;
  };

  const totalPluginsCount = () => {
    const local = plugins().length;
    if (scope() === "user") return local;
    return local + inheritedPlugins().length;
  };

  async function refreshMcp() {
    try {
      if (scope() === "user") {
        const res = await api.getGlobalMcpServers();
        setMcpServers(res.servers ?? {});
        setInheritedMcpServers({});
      } else {
        const [projectRes, globalRes] = await Promise.all([
          api.getMcpServers(),
          api.getGlobalMcpServers(),
        ]);
        setMcpServers(projectRes.servers ?? {});
        setInheritedMcpServers(globalRes.servers ?? {});
      }
      setMcpDirty(false);
    } catch (e) {
      setNotice({ kind: "error", text: `Could not load MCP servers: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  async function refreshCapabilities() {
    try {
      if (scope() === "user") {
        const [skillResult, hookResult, pluginResult, sourceResult, catalogResult] = await Promise.all([
          api.listSkills(),
          api.getGlobalHooks(),
          api.listPlugins("user"),
          api.listPluginSources("user"),
          api.listPluginCatalog(marketplaceQuery(), "user"),
        ]);
        setSkills(skillResult.skills ?? []);
        setHooks(hookResult.hooks ?? []);
        setInheritedHooks([]);
        setPlugins(pluginResult.plugins ?? []);
        setInheritedPlugins([]);
        setPluginSources(sourceResult.sources ?? []);
        setMarketplaceEntries(catalogResult.entries ?? []);
      } else {
        const [skillResult, hookResult, globalHookResult, pluginResult, globalPluginResult, sourceResult, catalogResult] = await Promise.all([
          api.listSkills(),
          api.getHooks(),
          api.getGlobalHooks(),
          api.listPlugins("workspace"),
          api.listPlugins("user"),
          api.listPluginSources("workspace"),
          api.listPluginCatalog(marketplaceQuery(), "workspace"),
        ]);
        setSkills(skillResult.skills ?? []);
        setHooks(hookResult.hooks ?? []);
        setInheritedHooks(globalHookResult.hooks ?? []);
        setPlugins(pluginResult.plugins ?? []);
        setInheritedPlugins(globalPluginResult.plugins ?? []);
        setPluginSources(sourceResult.sources ?? []);
        setMarketplaceEntries(catalogResult.entries ?? []);
      }
      setHooksDirty(false);
    } catch (e) {
      setNotice({ kind: "error", text: `Could not load capabilities: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  async function mutatePlugin(name: string, action: "enable" | "disable" | "rollback" | "remove") {
    if (pluginBusy()) return;
    setPluginBusy(true);
    try {
      if (action === "remove") await api.removePlugin(name, capabilityScope());
      else await api.pluginAction(name, action, capabilityScope());
      await refreshCapabilities();
      setNotice({ kind: "info", text: `${name} ${action === "remove" ? "removed" : `${action}d`}.` });
    } catch (e) {
      setNotice({ kind: "error", text: `Plugin action failed: ${e instanceof Error ? e.message : String(e)}` });
    } finally { setPluginBusy(false); }
  }

  async function installPlugin(update = false) {
    const path = pluginPath().trim();
    if (!path || pluginBusy()) return;
    setPluginBusy(true);
    try {
      const pluginScope = scope() === "user" ? "user" : "workspace";
      if (update) await api.updatePlugin(path, pluginScope);
      else await api.installPlugin(path, pluginScope);
      setPluginPath("");
      await refreshCapabilities();
      setNotice({ kind: "info", text: update ? "Plugin generation staged disabled." : "Plugin installed disabled." });
    } catch (e) {
      setNotice({ kind: "error", text: `Plugin install failed: ${e instanceof Error ? e.message : String(e)}` });
    } finally { setPluginBusy(false); }
  }

  async function togglePluginNetwork(name: string, on: boolean) {
    const scope = capabilityScope();
    if (pluginBusy()) return;
    const plugin = plugins().find((p) => p.name === name);
    // Deny always wins: a plugin on plugins.network_deny cannot be granted
    // egress from the toggle, because the config tier re-evaluates the deny
    // list on every admission.
    if (on && plugin?.network_denied) {
      setNotice({ kind: "error", text: `${name} is blocked by plugins.network_deny; remove the deny entry to grant sandbox egress.` });
      return;
    }
    setPluginBusy(true);
    try {
      // Preserve any existing grant for other plugins: toggling one plugin
      // must never silently revoke egress another already enjoys. The
      // effective allowlist is reported per plugin by the server.
      const current = plugin?.network_allow ?? [];
      const set = new Set(on ? [...current, name] : current.filter((n) => n !== name));
      const grant = [...set].sort();
      // Grants are privileged: the route below is the same patch the server
      // validates, and it refuses a grant into an untrusted project layer.
      if (scope === "user") await api.patchGlobalConfig({ plugins_network_allow: grant });
      else await api.patchConfig({ plugins_network_allow: grant });
      await refreshCapabilities();
      setNotice({ kind: "info", text: on ? `${name} may now reach the network from its sandbox (applies from the next turn).` : `${name} sandbox egress blocked.` });
    } catch (e) {
      setNotice({ kind: "error", text: `Network setting failed: ${e instanceof Error ? e.message : String(e)}` });
    } finally { setPluginBusy(false); }
  }

  async function registerPluginSource() {
    const path = sourcePath().trim();
    if (!path || pluginBusy()) return;
    setPluginBusy(true);
    try {
      const signed = sourceKeyId() && sourcePublicKey() && sourceSignature() ? { key_id: sourceKeyId(), public_key: sourcePublicKey(), signature: sourceSignature() } : undefined;
      await api.registerPluginSource(path, "Desktop catalog", signed);
      setSourcePath("");
      setSourceKeyId(""); setSourcePublicKey(""); setSourceSignature("");
      await refreshCapabilities();
      setNotice({ kind: "info", text: "Catalog source registered disabled." });
    } catch (e) {
      setNotice({ kind: "error", text: `Catalog registration failed: ${e instanceof Error ? e.message : String(e)}` });
    } finally { setPluginBusy(false); }
  }

  async function saveHooks() {
    setHooksSaving(true);
    try {
      if (scope() === "user") await api.putGlobalHooks(hooks());
      else await api.putHooks(hooks());
      await refreshCapabilities();
      setHooksDirty(false);
      setNotice({ kind: "info", text: scope() === "user" ? `Saved ${hooks().length} shared hook${hooks().length === 1 ? "" : "s"} — inherited by workspaces` : `Saved ${hooks().length} workspace hook${hooks().length === 1 ? "" : "s"} — active for new turns` });
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
    setHooks((current) => [...current, { event: "pre_tool_use", matcher: "", command: "", timeout_ms: 10000, enabled: true, failure_mode: "open" }]);
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
      await refreshCapabilities();
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

  async function runOp(service: "gateway" | "bridges", action: "start" | "stop" | "restart" | "install" | "uninstall") {
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

  const loadProviders = async () => {
    try {
      const p = await api.listProviders();
      setProviders(p);
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
    scope();
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
      setEvidenceAgeHours(Math.max(0, Math.round((next.intent_evidence_max_age_secs ?? 86400) / 3600)));
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
  const pageGroups = createMemo(() => {
    const groups = new Map<string, typeof pages>();
    for (const item of visiblePages()) groups.set(item.group, [...(groups.get(item.group) ?? []), item]);
    return [...groups.entries()];
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

  const deleteTask = (id: string) => {
    const task = archivedSessions().find((session) => session.session_id === id);
    setConfirmConfig({
      title: `Permanently delete “${task?.title || "Untitled task"}”?`,
      description: "This task and its conversation events will be deleted from Vak's ledger. This action cannot be undone.",
      confirmLabel: "Delete Task",
      cancelLabel: "Cancel",
      isDanger: true,
      onConfirm: async () => {
        try {
          await api.deleteSession(id);
          await refreshSessions();
          setNotice({ kind: "info", text: "Task deleted from history." });
        } catch (error) {
          setNotice({ kind: "error", text: `Could not delete that task: ${error instanceof Error ? error.message : String(error)}` });
        }
      },
    });
  };

  const deleteAllArchived = () => {
    if (!archivedSessions().length) return;
    setConfirmConfig({
      title: `Delete all ${archivedSessions().length} archived tasks?`,
      description: "All archived tasks and their events will be permanently removed from Vak's ledger. This action cannot be undone.",
      confirmLabel: "Delete All Archived",
      cancelLabel: "Cancel",
      isDanger: true,
      onConfirm: async () => {
        try {
          const result = await api.deleteAllArchived();
          await refreshSessions();
          setNotice({ kind: "info", text: `${result.deleted} archived task${result.deleted === 1 ? "" : "s"} deleted.` });
        } catch (error) {
          setNotice({ kind: "error", text: `Could not delete archived tasks: ${error instanceof Error ? error.message : String(error)}` });
        }
      },
    });
  };

  // ---- Data & backup (docs/design/29-personal-os.md P3) ----------------------
  const [backupDir, setBackupDir] = createSignal("");
  const [includeSecrets, setIncludeSecrets] = createSignal(false);
  const [importDir, setImportDir] = createSignal("");
  const [conflict, setConflict] = createSignal<"skip" | "rename">("skip");
  const [backupBusy, setBackupBusy] = createSignal(false);

  const pickDirectory = async (current: string): Promise<string> => {
    try {
      const dir = await host.pickWorkspace();
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
    return provider() !== c.provider || model() !== c.model || maxTurns() !== c.max_turns || evidenceAgeHours() * 3600 !== (c.intent_evidence_max_age_secs ?? 86400);
  };

  const applyAgent = async () => {
    setSaving(true);
    try {
      if (scope() === "user") await api.patchGlobalConfig({ provider: provider(), model: model(), max_turns: maxTurns() });
      else await api.patchConfig({ provider: provider(), model: model(), max_turns: maxTurns() });
      const saved = await api.patchEvidencePolicy(evidenceAgeHours() * 3600, capabilityScope());
      setConfig((current) => current ? { ...current, intent_evidence_max_age_secs: saved.seconds } : current);
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
      if (scope() === "user") await api.patchGlobalConfig({ permission_mode: wire });
      else await api.patchConfig({ permission_mode: wire });
      setConfig((current) => current ? { ...current, permission_mode: mode } : current);
      await loadHealth();
    } catch (error) {
      setNotice({ kind: "error", text: `Could not update permissions: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  /// How an `Ask` is resolved. Separate control from the permission mode
  /// above and deliberately narrower: it never widens the boundary and never
  /// switches off the sandbox — a denied call stays denied under every one
  /// of these. The desktop had no control for it at all, so the only way to
  /// change it was the browser console.
  const changeApproval = async (mode: ConfigSnapshot["approval_mode"]) => {
    try {
      if (scope() === "user") await api.patchGlobalConfig({ approval_mode: mode });
      else await api.patchConfig({ approval_mode: mode });
      setConfig((current) => (current ? { ...current, approval_mode: mode } : current));
    } catch (error) {
      setNotice({
        kind: "error",
        text: `Could not update approvals: ${error instanceof Error ? error.message : String(error)}`,
      });
    }
  };

  const openWorkspaceConfig = async () => {
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
    <div ref={settingsRoot} class="settings-shell" role="dialog" aria-modal="true" aria-label="Settings" use:trapFocus>
      <aside class="settings-nav">
        <button type="button" class="settings-back" onClick={() => setSettingsOpen(false)}><Icon name="chevron" /><span>Back to Vak</span></button>
        <div class="settings-search"><Icon name="search" /><input aria-label="Search settings" placeholder="Search settings…" value={query()} onInput={(event) => setQuery(event.currentTarget.value)} /></div>
        <div class="settings-nav-label">Settings scope</div>
        <div class="settings-scope-toggle" role="group" aria-label="Settings scope">
          <button type="button" aria-pressed={scope() === "user"} classList={{ active: scope() === "user" }} onClick={() => setSettingsScope("user")}>Shared</button>
          <button type="button" aria-pressed={scope() === "workspace"} classList={{ active: scope() === "workspace" }} onClick={() => setSettingsScope("workspace")}>This workspace</button>
        </div>
        <p class="settings-scope-copy">Shared is your default. This workspace only changes what belongs to this folder.</p>
        <For each={pageGroups()} fallback={<div class="settings-no-results">No matching settings</div>}>
          {([group, items]) => <div class="settings-nav-group"><div class="settings-nav-label">{group}</div><nav><For each={items}>{(item) => <button classList={{ active: page() === item.id }} onClick={() => { setPage(item.id); setQuery(""); }}><Icon name={item.icon} /><span>{item.label}</span></button>}</For></nav></div>}
        </For>
        <Show when={showArchivedPage()}>
          <div class="settings-nav-label archived-nav-label">Archived</div>
          <nav>
            <button type="button" classList={{ active: page() === "archived" }} onClick={() => { setPage("archived"); setQuery(""); }}><Icon name="archive" /><span>Archived tasks</span></button>
          </nav>
        </Show>
        <div class="settings-nav-foot"><div class="settings-app-mark"><img src={`${import.meta.env.BASE_URL}vak-icon.png`} alt="" /></div><div><strong>Vak</strong><span>{backend().version ? `Version ${backend().version}` : "Version unavailable"}</span></div></div>
      </aside>

      <main class="settings-main">
        <div class="settings-content">
          <Show when={voiceProviders.error || presentationLibrary.error || promptLayer.error || promptEffective.error}>
            <div class="settings-load-error" role="alert">
              Some settings could not be loaded. Existing values were kept; use the relevant section's refresh or reopen Settings to retry.
              <Show when={voiceProviders.error}><span> Voice discovery: {String(voiceProviders.error)}</span></Show>
              <Show when={presentationLibrary.error}><span> Presentations: {String(presentationLibrary.error)}</span></Show>
              <Show when={promptLayer.error || promptEffective.error}><span> Prompt configuration is unavailable.</span></Show>
            </div>
          </Show>
          <Show when={!loading()} fallback={<div class="settings-loading"><span /><span /><span /></div>}>
            <div class="settings-callout scope-callout" role="status">
              <Icon name={scope() === "user" ? "layers" : "folder"} />
              <div>
                <strong>{scope() === "user" ? "Editing Shared defaults" : "Editing this workspace"}</strong>
                <span>{scope() === "user" ? "Inherited by every workspace unless it overrides the value." : (config()?.paths.cwd ?? backend().cwd ?? "Current workspace")}</span>
              </div>
            </div>
            <Show when={page() === "general"}>
              <header><h1>{scope() === "user" ? "User settings" : "Workspace settings"}</h1><p>{scope() === "user" ? "Shared defaults and capabilities inherited by your workspaces." : "Overrides for this folder. Unchanged settings inherit your user defaults."}</p></header>
              <Group title="Experience">
                <Row title="Desktop notifications" description="Notify when the active task finishes while Vak is in the background."><Switch label="Desktop notifications" checked={uiPreferences.notifications} onChange={(value) => updateUiPreference("notifications", value)} /></Row>
                <Row title="Quiet hours" description="Suppress background completion and update notifications overnight. Approval requests remain interruptive because work is paused until you decide."><select aria-label="Quiet hours" value={uiPreferences.quietHours} onChange={(event) => updateUiPreference("quietHours", event.currentTarget.value as "off" | "22-07")}><option value="off">Off</option><option value="22-07">22:00–07:00</option></select></Row>
                <Row title="Sound cues" description="Short chime when a task starts working and when it finishes."><Switch label="Sound cues" checked={uiPreferences.soundCues} onChange={(value) => updateUiPreference("soundCues", value)} /></Row>
                <Row title="Suggested prompts" description="Show useful starting points when a task has no conversation yet."><Switch label="Suggested prompts" checked={uiPreferences.suggestions} onChange={(value) => updateUiPreference("suggestions", value)} /></Row>
                <Row title="Transcript detail" description="Control how much agent activity appears in conversations."><select aria-label="Transcript detail" value={density()} onChange={(event) => setDensity(event.currentTarget.value as Density)}><option value="outcome">Outcome</option><option value="balanced">Balanced</option><option value="audit">Audit</option></select></Row>
                <Row title="Reusable presentations" description="Validated experience-pack cards that Vak can use when rendering work. Choose only the families you want available to this scope."><span class="settings-value">{presentationLibrary.loading ? "Loading…" : `${activePresentationCount()} active · ${presentationLibrary()?.definitions.length ?? 0} definitions`}</span></Row>
                <Row title="Presentation packs" description="Share validated definitions without sharing task results. Imported packs stay disabled until you activate them."><span class="settings-actions"><button class="settings-button" onClick={() => void exportPresentationPack()}>Export</button><label class="settings-button">Import<input type="file" accept="application/json,.json" hidden onChange={importPresentationPack} /></label></span></Row>
                <Show when={!presentationLibrary.loading && (presentationLibrary()?.definitions.length ?? 0) > 0}>
                  <section class="presentation-library" aria-label="Reusable presentations">
                    <div class="presentation-toolbar">
                      <label class="presentation-search"><Icon name="search" size={14} /><input aria-label="Search presentations" placeholder="Search by name or capability…" value={presentationQuery()} onInput={(event) => setPresentationQuery(event.currentTarget.value)} /></label>
                      <div class="presentation-filters" role="group" aria-label="Presentation filter">
                        <For each={[{ id: "all", label: "All" }, { id: "active", label: "Active" }, { id: "inactive", label: "Inactive" }] as const}>{(filter) => <button type="button" classList={{ active: presentationFilter() === filter.id }} aria-pressed={presentationFilter() === filter.id} onClick={() => setPresentationFilter(filter.id)}>{filter.label}</button>}</For>
                      </div>
                    </div>
                    <div class="presentation-scope-note"><Icon name={presentationScope() === "user" ? "layers" : "folder"} /><span>Managing <strong>{presentationScope() === "user" ? "Shared" : "This workspace"}</strong>. These activations are selected before broader workspace fallbacks.</span></div>
                    <Show when={presentationGroups().length > 0} fallback={<div class="presentation-empty">No presentation packs match this filter.</div>}>
                      <div class="presentation-groups">
                        <For each={presentationGroups()}>{(group, index) => {
                          const open = () => presentationExpanded().has(group.key) || (!presentationExpanded().size && !presentationQuery() && presentationFilter() === "all" && index() === 0);
                          const allLatestActive = () => group.types.every((type) => presentationActivation(type.id)?.revision === type.latest.spec.revision);
                          const groupBusy = () => presentationBusyFor(`group:${group.key}`);
                          return <section class="presentation-group" classList={{ open: open() }}>
                            <div class="presentation-group-header">
                              <button type="button" class="presentation-disclosure" aria-expanded={open()} onClick={() => togglePresentationGroup(group.key)}><Icon name="chevron" /><span><strong>{group.label}</strong><small>{group.types.length} pack{group.types.length === 1 ? "" : "s"} · {group.definitionCount} versions</small></span></button>
                              <span class="presentation-group-count">{group.types.filter((type) => type.active).length} active</span>
                              <button type="button" class="settings-button" disabled={groupBusy()} onClick={() => void runPresentationAction(`group:${group.key}`, () => applyPresentationGroup(group))}>{groupBusy() ? "Working…" : allLatestActive() ? "Deactivate group" : "Activate latest"}</button>
                            </div>
                            <Show when={open()}>
                              <div class="presentation-group-body">
                                <For each={group.types}>{(type) => {
                                  const activeRevision = () => presentationActivation(type.id)?.revision ?? null;
                                  const busy = () => presentationBusyFor(type.id);
                                  const latestActive = () => activeRevision() === type.latest.spec.revision;
                                  const versionsOpen = () => presentationVersions().has(type.id);
                                  return <div class="presentation-type">
                                    <div class="presentation-type-main">
                                      <div class="presentation-type-copy"><strong>{type.label}</strong><span>{type.semanticType} · {type.latest.origin.plugin_id ?? "Built-in"}</span></div>
                                      <Show when={type.active}><span class="settings-status good">v{activeRevision()} active</span></Show>
                                      <button type="button" class="settings-button" disabled={busy()} onClick={() => void runPresentationAction(type.id, () => applyPresentationType(type))}>{busy() ? "Working…" : latestActive() ? "Deactivate" : type.active ? "Use latest" : "Activate"}</button>
                                      <button type="button" class="settings-button subtle" disabled={busy()} onClick={() => void runPresentationAction(`reset:${type.id}`, () => resetPresentation(type.id))}>Reset</button>
                                    </div>
                                    <Show when={type.definitions.length > 1}>
                                      <button type="button" class="presentation-versions-toggle" aria-expanded={versionsOpen()} onClick={() => togglePresentationVersions(type.id)}><Icon name="chevron" />{versionsOpen() ? "Hide" : "Show"} {type.definitions.length} versions</button>
                                      <Show when={versionsOpen()}>
                                        <div class="presentation-versions"><For each={type.definitions}>{(definition) => <div class="presentation-version"><span>v{definition.spec.revision}{definition.spec.revision === type.latest.spec.revision ? " · latest" : ""}</span><Show when={activeRevision() === definition.spec.revision}><span class="settings-status good">active</span></Show><button type="button" class="settings-button" disabled={busy() || activeRevision() === definition.spec.revision} onClick={() => void runPresentationAction(type.id, () => activatePresentation(definition))}>{activeRevision() === definition.spec.revision ? "Using" : "Use this"}</button></div>}</For></div>
                                      </Show>
                                    </Show>
                                  </div>;
                                }}</For>
                              </div>
                            </Show>
                          </section>;
                        }}</For>
                      </div>
                    </Show>
                  </section>
                </Show>
                <Row title="Keyboard shortcuts" description="See every shortcut for navigation, tasks, and workspace tools."><button class="settings-button" onClick={() => { setSettingsOpen(false); setShowShortcuts(true); }}>View shortcuts</button></Row>
              </Group>
              <Group title="Voice">
                <Row title="Enable voice conversations" description="Allow microphone sessions and governed speech input/output."><Switch label="Enable voice conversations" checked={config()?.voice?.enabled ?? false} onChange={(value) => void updateVoice({ voice_enabled: value })} /></Row>
                <Row title="Speak agent actions out loud" description="Narrate turn completions and permission prompts through the voice pipeline."><Switch label="Speak agent actions out loud" checked={uiPreferences.voiceEnabled} onChange={(value) => updateUiPreference("voiceEnabled", value)} /></Row>
                <Row title="Session limit" description="Maximum duration for one voice session, in seconds."><input type="number" min="1" max="86400" value={config()?.voice?.max_session_secs ?? 900} onChange={(e) => void updateVoice({ voice_max_session_secs: Number(e.currentTarget.value) })} /></Row>
                <Row title="Concurrent sessions" description="Maximum simultaneous voice sessions for this scope."><input type="number" min="1" max="64" value={config()?.voice?.max_concurrent ?? 2} onChange={(e) => void updateVoice({ voice_max_concurrent: Number(e.currentTarget.value) })} /></Row>
                <Row title="Inbound audio budget" description="Maximum audio bytes accepted per session."><input type="number" min="1" max={256 * 1024 * 1024} value={config()?.voice?.max_audio_bytes ?? 16 * 1024 * 1024} onChange={(e) => void updateVoice({ voice_max_audio_bytes: Number(e.currentTarget.value) })} /></Row>
                <Row title="Voice providers" description="Providers are discovered from the server and reflect installed integrations and reachable credentials."><span class="settings-value">{voiceProviders.loading ? "Discovering…" : (voiceProviders()?.providers.map((provider) => `${provider.name}${provider.configured ? " · ready" : " · credential needed"}`).join(", ") || "None configured")}</span></Row>
                <Row title="Voice provider route" description="Choose the provider for new voice sessions; blank uses the configured default."><select value={config()?.voice?.provider ?? ""} onChange={(e) => void updateVoice({ voice_provider: e.currentTarget.value || null })}><option value="">Configured default</option><For each={voiceProviders()?.providers ?? []}>{(provider) => <option value={provider.name}>{provider.name}</option>}</For></select></Row>
                <Row title="Transcription model" description="Model used for batch speech-to-text."><input type="text" value={config()?.voice?.transcription_model ?? ""} placeholder="Provider default" onChange={(e) => void updateVoice({ voice_transcription_model: e.currentTarget.value.trim() || null })} /></Row>
                <Row title="Synthesis model" description="Model used for text-to-speech."><input type="text" value={config()?.voice?.synthesis_model ?? ""} placeholder="Provider default" onChange={(e) => void updateVoice({ voice_synthesis_model: e.currentTarget.value.trim() || null })} /></Row>
                <Row title="Realtime model" description="Model used for live voice sessions."><input type="text" value={config()?.voice?.realtime_model ?? ""} placeholder="Provider default" onChange={(e) => void updateVoice({ voice_realtime_model: e.currentTarget.value.trim() || null })} /></Row>
                <Show when={uiPreferences.voiceEnabled}>
                  <Row title="Voice" description="The synthesized voice discovered from configured providers."><select value={uiPreferences.voiceName} onChange={(event) => updateUiPreference("voiceName", event.currentTarget.value)}><option value="">Provider default</option><For each={discoveredVoices()}>{(voice) => <option value={voice}>{voice}</option>}</For></select></Row>
                  <Row title="Persona" description="Optional style directive for how narration sounds."><input value={uiPreferences.voicePersona} placeholder="e.g. calm and concise" onInput={(event) => updateUiPreference("voicePersona", event.currentTarget.value)} /></Row>
                </Show>
              </Group>
              <Group title={scope() === "user" ? "Desktop" : "Workspace"}>
                <Row title={scope() === "user" ? "Inheritance" : "Current workspace"} description={scope() === "user" ? "Workspaces inherit this scope unless their own settings override a value." : (config()?.paths.cwd ?? "")}><span class="settings-value">{scope() === "user" ? "Shared" : "Local"}</span></Row>
                <Show when={scope() === "workspace"}><Row title="Workspace configuration" description="Persistent agent and tool settings for this repository."><button class="settings-button" onClick={() => void openWorkspaceConfig()}>Open config</button></Row></Show>
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
                <div class="theme-grid"><For each={[{ id: "system", label: "Match system" }, { id: "light", label: "Warm light" }, { id: "warm", label: "Warm dark" }, { id: "dark", label: "Midnight" }, { id: "contrast", label: "High contrast" }, { id: "sage", label: "Quiet sage" }, { id: "paper", label: "Soft paper" }, { id: "mist", label: "Morning mist" }, { id: "dawn", label: "Soft dawn" }] as const}>{(theme) => <button class="theme-choice" classList={{ active: uiPreferences.theme === theme.id }} onClick={() => updateUiPreference("theme", theme.id)}><span class={`theme-preview ${theme.id}`}><i /><i /><i /></span><strong>{theme.label}</strong><Show when={uiPreferences.theme === theme.id}><Icon name="check" /></Show></button>}</For></div>
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
              <AgentsPanel />
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
                <Row title="Evidence freshness" description="How long a successful tool receipt remains fresh for outcome verification."><input class="settings-number" type="number" min="0" max="8760" value={evidenceAgeHours()} onInput={(event) => setEvidenceAgeHours(Number(event.currentTarget.value) || 0)} /><span class="settings-status">hours</span></Row>
                <Row title="Subagents" description="Allow the agent to delegate bounded parallel work."><span class="settings-status good">{config()?.subagents ? "Enabled" : "Disabled in config"}</span></Row>
              </Group>
              <Group title="Credentials">
                <Row
                  title={`${keyProvider()} — ${currentProviderInfo()?.env_var ?? "no key needed"}`}
                  description={
                    currentProviderInfo()?.requires_key
                      ? `${currentProviderInfo()?.configured ? "Saved on this device" : "Not set yet"} · ${currentProviderInfo()?.pool_size ?? 0} credential${(currentProviderInfo()?.pool_size ?? 0) === 1 ? "" : "s"} in the routing pool · stored in ~/.vak/.env with owner-only permissions. A real environment variable takes precedence.`
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
                {/* A chat credential belongs to a bot, not to a surface
                    (AGENTS.md invariant 23), and several bots can share a
                    transport. The per-surface token field that used to sit
                    here could only ever describe one bot per transport, so
                    it is gone: bots are created and credentialed in the
                    admin console, which is also where a chat is approved. */}
                <Row
                  title="Chat bots"
                  description="Telegram, Discord, and Slack bots — each with its own token, policy, and route — are managed in the admin console."
                >
                  <span class="key-edit">
                    <button class="settings-button" onClick={() => host.openAdmin("#/gateway")}>Open admin console</button>
                  </span>
                </Row>
              </Group>
              <div class="settings-actions">
                <button class="btn primary" disabled={saving() || !agentDirty() || !provider().trim() || !model().trim()} onClick={() => void applyAgent()}>{saving() ? "Applying…" : "Apply changes"}</button>
                <button class="settings-button" onClick={() => void openWorkspaceConfig()}>Edit persistent config</button>
                {/* Selecting in the dropdowns changes nothing until this is
                    pressed; without a marker that reads as a silent no-op. */}
                <Show when={agentDirty()} fallback={<span class="settings-status good">Saved</span>}>
                  <span class="settings-status warn">Unsaved changes — press Apply</span>
                </Show>
              </div>
            </Show>

            <Show when={page() === "permissions"}>
              <header><h1>Permissions</h1><p>Set the trust boundary for tool calls in this workspace.</p></header>
              <div class="permission-options"><For each={[{ id: "ReadOnly", title: "Read only", text: "Inspect files and search the workspace without making changes.", icon: "preview" as IconName }, { id: "WorkspaceWrite", title: "Workspace write", text: "Edit files inside this workspace and ask before sensitive actions.", icon: "code" as IconName }, { id: "FullAccess", title: "Full access", text: "Run unrestricted commands and access files outside the workspace.", icon: "shield" as IconName }] as const}>{(mode) => <button classList={{ active: config()?.permission_mode === mode.id, danger: mode.id === "FullAccess" }} onClick={() => void changePermission(mode.id)}><span class="permission-icon"><Icon name={mode.icon} /></span><span><strong>{mode.title}</strong><small>{mode.text}</small></span><span class="permission-check"><Show when={config()?.permission_mode === mode.id}><Icon name="check" /></Show></span></button>}</For></div>
              <Group title="Approvals">
                <p class="settings-group-copy">
                  When something needs your say-so, this decides who answers. It cannot widen the
                  boundary above or switch off the sandbox — anything refused there stays refused.
                </p>
                <div class="permission-options">
                  <For
                    each={
                      [
                        { id: "ask", title: "Ask me every time", text: "Pause and wait for you before anything that needs approval.", icon: "shield" as IconName },
                        { id: "approve-safe", title: "Approve safe actions", text: "Reads and edits inside this workspace go ahead; the web and outside access still ask.", icon: "check" as IconName },
                        { id: "auto-approve", title: "Approve automatically", text: "Ordinary requests go ahead. Your own rules and the circuit breaker still stop and ask.", icon: "code" as IconName },
                      ] as const
                    }
                  >
                    {(mode) => (
                      <button
                        classList={{ active: config()?.approval_mode === mode.id }}
                        onClick={() => void changeApproval(mode.id)}
                      >
                        <span class="permission-icon"><Icon name={mode.icon} /></span>
                        <span><strong>{mode.title}</strong><small>{mode.text}</small></span>
                        <span class="permission-check">
                          <Show when={config()?.approval_mode === mode.id}><Icon name="check" /></Show>
                        </span>
                      </button>
                    )}
                  </For>
                </div>
              </Group>
              <Group title="Sandbox">
                <Row title="Workspace boundary" description="File tools are confined to the selected workspace and symlinks are resolved before access."><span class="settings-status good">Protected</span></Row>
                <Row title="Containment" description="How tool processes are confined on this machine."><span class="settings-status good">{config()?.sandbox ?? "…"}</span></Row>
              </Group>
              <Group title="Rules">
                <p class="settings-group-copy">
                  These decide specific calls before the setting above applies. “Never” always wins.
                </p>
                <For
                  each={
                    [
                      { key: "deny", title: "Never allow", empty: "Nothing is blocked outright." },
                      { key: "ask", title: "Always ask first", empty: "Nothing is singled out to ask about." },
                      { key: "allow", title: "Always allow", empty: "Nothing is pre-approved." },
                    ] as const
                  }
                >
                  {(section) => (
                    <Row title={section.title} description={
                      (config()?.permissions?.[section.key]?.length ?? 0) > 0
                        ? config()!.permissions[section.key].join(", ")
                        : section.empty
                    }>
                      <span />
                    </Row>
                  )}
                </For>
                <Row title="Permission rules" description="Add or remove allow, ask, and deny patterns in the workspace configuration."><button class="settings-button" onClick={() => void openWorkspaceConfig()}>Edit rules</button></Row>
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
                    : "Same model on other providers only. Add route.fallback_models in workspace config to allow named alternates."}
                >
                  <span class="metric">{config()?.route.fallback_models.length ?? 0}</span>
                </Row>
                <Row title="Ladder length cap" description="Maximum frozen legs per session, including your primary choice — the primary never loses its head position."><span class="metric">{config()?.route.max_fallbacks}</span></Row>
              </Group>
              <button class="settings-button" onClick={() => void openWorkspaceConfig()}>Tune in workspace config</button>
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
                  <p class="settings-hint memory-hint">Global tier — these notes are recalled in every workspace.</p>
                </Show>
                <Show
                  when={!addingNote()}
                  fallback={
                    <div class="memory-add">
                      <div class="memory-add-fields">
                        <label>Kind<input value={noteKind()} aria-label="Note kind" onInput={(e) => setNoteKind(e.currentTarget.value)} /></label>
                        <label>Tag <span class="label-hint">optional</span><input value={noteTag()} aria-label="Note tag" onInput={(e) => setNoteTag(e.currentTarget.value)} /></label>
                      </div>
                      <textarea rows={2} placeholder={tier() === "profile" ? "Something that should hold across every workspace…" : "Something that should hold in this workspace…"} aria-label="Note text" value={noteText()} onInput={(e) => setNoteText(e.currentTarget.value)} />
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
              <header><h1>Capabilities</h1><p>{scope() === "user" ? "Shared capabilities are available to every workspace. A workspace may add its own capability or override a same-named MCP server." : "Edit only this folder’s capabilities. Shared capabilities remain available unless this workspace deliberately replaces a same-named MCP server."}</p></header>
              <div class="settings-callout scope-callout"><Icon name={scope() === "user" ? "layers" : "folder"} /><div><strong>{scope() === "user" ? "Shared defaults apply everywhere." : "You are changing this workspace only."}</strong><span>{scope() === "user" ? "Workspace-specific changes never rewrite these shared settings." : "Use Shared above to change your defaults for every workspace."}</span></div></div>
              <nav class="capability-tabs" aria-label="Capability types">
                <button classList={{ active: capabilityTab() === "mcp" }} onClick={() => setCapabilityTab("mcp")}><Icon name="plug" /><span>MCP servers</span><em>{totalMcpCount()}</em></button>
                <button classList={{ active: capabilityTab() === "skills" }} onClick={() => setCapabilityTab("skills")}><Icon name="spark" /><span>Skills</span><em>{visibleSkills().length}</em></button>
                <button classList={{ active: capabilityTab() === "hooks" }} onClick={() => setCapabilityTab("hooks")}><Icon name="tune" /><span>Hooks</span><em>{totalHooksCount()}</em></button>
                <button classList={{ active: capabilityTab() === "plugins" }} onClick={() => setCapabilityTab("plugins")}><Icon name="grid" /><span>Plugins</span><em>{totalPluginsCount()}</em></button>
              </nav>
              <Show when={capabilityTab() === "mcp"}>
                <Group title="Tool servers"><Show when={mcpServers()} fallback={<Row title="Loading servers…" description="Reading the effective MCP configuration."><span /></Row>}>
                  <div class="mcp-editor">
                    <Show when={Object.keys(mcpServers() ?? {}).length > 0} fallback={<div class="capability-empty"><Icon name="plug" /><strong>{scope() === "user" ? "No shared MCP servers connected" : (Object.keys(inheritedMcpServers()).length > 0 ? "No project-specific MCP overrides" : "No MCP servers connected")}</strong><span>{scope() === "user" ? "Add a shared server to make it available across all your projects." : (Object.keys(inheritedMcpServers()).length > 0 ? "This project is using the shared MCP servers listed below. Add a server here to create a project-specific override or tool." : "Add a local server to give the agent tools such as search, browser, or data access.")}</span></div>}>
                      <For each={Object.entries(mcpServers() ?? {})}>{([name, def]) => <div class="mcp-row">
                        <div class="mcp-row-head"><div><strong>{name}</strong><span class="capability-state ready">Configured</span></div><button class="settings-button danger" onClick={() => removeServer(name)}><Icon name="trash" /> Remove</button></div>
                        <div class="mcp-fields"><label>Server name<input value={name} aria-label="Server name" onChange={(e) => renameServer(name, e.currentTarget.value.trim())} /></label><label>Command<input placeholder="/path/to/command" value={def.command} aria-label="Command" onInput={(e) => updateServer(name, { command: e.currentTarget.value })} /></label><label>Arguments<input placeholder="Space-separated arguments" value={def.args.join(" ")} aria-label="Arguments" onInput={(e) => updateServer(name, { args: e.currentTarget.value.split(" ").filter(Boolean) })} /></label></div>
                        <div class="mcp-controls"><label class="mcp-network"><Switch checked={def.network} label={`Allow network for ${name}`} onChange={(v) => updateServer(name, { network: v })} /><span>Allow outbound network</span></label><span class="settings-hint">Health check runs automatically when this server is first used in a task.</span></div>
                      </div>}</For>
                    </Show>
                    <div class="settings-actions"><button class="btn" onClick={addServer}><Icon name="add" /> Add server</button><button class="btn primary" disabled={!mcpDirty() || mcpSaving()} onClick={() => void saveMcp()}>{mcpSaving() ? "Saving…" : "Save & apply"}</button><Show when={mcpDirty()}><span class="mcp-dirty">Unsaved changes</span></Show></div>
                    <p class="settings-hint">{scope() === "user" ? "These are your shared server definitions. Projects inherit them. Secrets are referenced by name and never shown here." : "A same-named project server replaces the shared one for this folder only. Network is off until you turn it on; secrets stay user-owned."}</p>

                    <Show when={scope() === "workspace" && Object.keys(inheritedMcpServers()).length > 0}>
                      <div class="inherited-capabilities-group">
                        <div class="inherited-capabilities-head">
                          <Icon name="layers" />
                          <div>
                            <strong>Shared MCP servers ({Object.keys(inheritedMcpServers()).length})</strong>
                            <span>Inherited from your global settings. Available in every task in this workspace unless replaced by a same-named server above.</span>
                          </div>
                        </div>
                        <div class="capability-list">
                          <For each={Object.entries(inheritedMcpServers())}>{([name, def]) => {
                            const isOverridden = () => !!mcpServers()?.[name];
                            return (
                              <div class="capability-item" classList={{ "capability-item-overridden": isOverridden() }}>
                                <div class="inherited-row">
                                  <div class="inherited-info">
                                    <strong>{name}</strong>
                                    <code>{def.command} {def.args.join(" ")}</code>
                                    <small>{def.network ? "Network allowed" : "Local only"}</small>
                                  </div>
                                  <div class="inherited-actions">
                                    <Show when={isOverridden()} fallback={
                                      <>
                                        <span class="capability-state inherited">Inherited (Active)</span>
                                        <button class="settings-button" onClick={() => {
                                          updateServer(name, { command: def.command, args: [...def.args], env: { ...def.env }, network: def.network });
                                          setNotice({ kind: "info", text: `Copied ${name} to project settings. You can now edit it.` });
                                        }}>Customize for project</button>
                                      </>
                                    }>
                                      <span class="capability-state muted">Overridden by project</span>
                                    </Show>
                                  </div>
                                </div>
                              </div>
                            );
                          }}</For>
                        </div>
                      </div>
                    </Show>
                  </div>
                </Show></Group>
              </Show>
              <Show when={capabilityTab() === "skills"}>
                <Group title={`Discovered skills (${visibleSkills().length})`}><Show when={visibleSkills().length > 0} fallback={<div class="capability-empty"><Icon name="spark" /><strong>No skills discovered</strong><span>{scope() === "user" ? "Add a SKILL.md to your Vak home to make it available everywhere." : "Add a SKILL.md to this project or use Shared to add one everywhere."}</span></div>}><p class="settings-hint">{scope() === "user" ? "These are shared skills. They are inherited by every project." : "Shared skills and this project’s skills are both available here. Each item shows where it came from."}</p><div class="capability-list"><For each={visibleSkills()}>{(skill) => <details class="capability-item"><summary><span><strong>{skill.name}</strong><small>{skill.scope === "user" ? "Shared" : "This project"}</small></span><span class="capability-state ready">Available</span></summary><div class="capability-detail"><p>{skill.description || "No description provided."}</p><Show when={skill.source}><code>{skill.source}</code></Show><button type="button" class="settings-button" onClick={async () => { try { if (!navigator.clipboard) throw new Error("Clipboard access is unavailable"); await navigator.clipboard.writeText(`/skill ${skill.name} `); setNotice({ kind: "info", text: `Copied /skill ${skill.name} to your clipboard. Open a task and paste it into the composer.` }); } catch { setNotice({ kind: "error", text: "Could not copy the skill command. Clipboard access was denied." }); } }}>Copy to composer</button></div></details>}</For></div></Show></Group>
                <Group title={`Pending proposals (${proposals().length})`}><Show when={proposals().length > 0} fallback={<Row title="No proposals waiting" description="The agent can suggest reusable skills; they stay inactive until you review them."><span class="settings-status good">Clear</span></Row>}><For each={proposals()}>{(proposal) => <div class="setting-row"><div class="setting-copy"><strong>{proposal.name}</strong><span>{proposal.description}</span></div><div class="setting-control"><button class="settings-button" onClick={() => void promote(proposal.id)}>Review & promote</button><button class="settings-button danger" onClick={() => void reject(proposal.id)}>Reject</button></div></div>}</For></Show></Group>
              </Show>
              <Show when={capabilityTab() === "hooks"}>
                <Group title="Lifecycle automation"><div class="settings-callout"><Icon name="shield" /><div><strong>Hooks run commands at controlled lifecycle points.</strong><span>Use closed failure handling for guards where a timeout or unavailable script must stop the operation.</span></div></div><Show when={hooks().length > 0} fallback={<div class="capability-empty"><Icon name="tune" /><strong>{scope() === "user" ? "No shared hooks configured" : (inheritedHooks().length > 0 ? "No project-specific hooks" : "No hooks configured")}</strong><span>{scope() === "user" ? "Add a hook to run a safe, repeatable action at session or tool lifecycle events across all projects." : (inheritedHooks().length > 0 ? "This project is using the shared hooks listed below. Add a hook here to run project-specific actions." : "Add a hook to run a safe, repeatable action at session or tool lifecycle events.")}</span></div>}><div class="hook-editor"><For each={hooks()}>{(hook, index) => <div class="hook-row"><div class="hook-row-head"><span class="capability-state" classList={{ ready: hook.enabled !== false, muted: hook.enabled === false }}>{hook.enabled === false ? "Disabled" : "Enabled"}</span><Switch checked={hook.enabled !== false} label={`Enable hook ${index() + 1}`} onChange={(v) => updateHook(index(), { enabled: v })} /><button class="settings-button danger" aria-label={`Remove hook ${index() + 1}`} onClick={() => removeHook(index())}><Icon name="trash" /></button></div><label>Lifecycle event<select value={hook.event} onChange={(e) => updateHook(index(), { event: e.currentTarget.value })}><option value="session_start">Session start</option><option value="pre_tool_use">Before a tool runs</option><option value="post_tool_use">After a tool runs</option><option value="stop">Task stop</option></select></label><label>Tool matcher <span class="label-hint">optional</span><input value={hook.matcher ?? ""} placeholder="bash, write, or leave blank" onInput={(e) => updateHook(index(), { matcher: e.currentTarget.value })} /></label><label>Command<input class="hook-command" value={hook.command} placeholder="e.g. cargo fmt --all --check" onInput={(e) => updateHook(index(), { command: e.currentTarget.value })} /></label><label>Timeout (ms)<input type="number" min="100" max="120000" value={hook.timeout_ms ?? 10000} onInput={(e) => updateHook(index(), { timeout_ms: Number(e.currentTarget.value) || 10000 })} /></label><label>On hook failure<select value={hook.failure_mode ?? "open"} onChange={(e) => updateHook(index(), { failure_mode: e.currentTarget.value as "open" | "closed" })}><option value="open">Continue and report</option><option value="closed">Block the operation</option></select></label></div>}</For></div></Show><div class="settings-actions"><button class="btn" onClick={addHook}><Icon name="add" /> Add hook</button><button class="btn primary" disabled={!hooksDirty() || hooksSaving()} onClick={() => void saveHooks()}>{hooksSaving() ? "Saving…" : "Save hooks"}</button><Show when={hooksDirty()}><span class="mcp-dirty">Unsaved changes</span></Show></div>
                <Show when={scope() === "workspace" && inheritedHooks().length > 0}>
                  <div class="inherited-capabilities-group">
                    <div class="inherited-capabilities-head">
                      <Icon name="layers" />
                      <div>
                        <strong>Shared lifecycle hooks ({inheritedHooks().length})</strong>
                        <span>Inherited from your global settings. These run automatically alongside project-specific hooks.</span>
                      </div>
                    </div>
                    <div class="capability-list">
                      <For each={inheritedHooks()}>{(hook) => (
                        <div class="capability-item">
                          <div class="inherited-row">
                            <div class="inherited-info">
                              <strong>{hook.event}{hook.matcher ? ` (${hook.matcher})` : ""}</strong>
                              <code>{hook.command}</code>
                              <small>Timeout: {hook.timeout_ms ?? 10000}ms · On failure: {hook.failure_mode ?? "open"}</small>
                            </div>
                            <div class="inherited-actions">
                              <span class="capability-state inherited">{hook.enabled === false ? "Disabled" : "Inherited (Active)"}</span>
                              <button class="settings-button" onClick={() => {
                                setHooks((current) => [...current, { ...hook }]);
                                setHooksDirty(true);
                                setNotice({ kind: "info", text: `Copied hook to project settings.` });
                              }}>Copy to project</button>
                            </div>
                          </div>
                        </div>
                      )}</For>
                    </div>
                  </div>
                </Show>
                </Group>
              </Show>
              <Show when={capabilityTab() === "plugins"}>
                <Group title="Installed plugins">
                  <div class="settings-callout"><Icon name="shield" /><div><strong>Plugins are installed disabled.</strong><span>Each generation is content-addressed and remains inactive until you explicitly enable it. Review the digest, publisher, and capabilities first.</span></div></div>
                  <div class="mcp-fields"><label>Package directory<input class="mono" placeholder="/path/to/plugin" value={pluginPath()} onInput={(e) => setPluginPath(e.currentTarget.value)} /></label><div class="settings-actions"><button type="button" class="btn" disabled={!pluginPath().trim() || pluginBusy()} onClick={() => void installPlugin(false)}>Install disabled</button><button type="button" class="btn primary" disabled={!pluginPath().trim() || pluginBusy()} onClick={() => void installPlugin(true)}>Stage update</button><button type="button" class="settings-button" disabled={pluginBusy()} onClick={async () => { const picked = await host.pickWorkspace(); if (typeof picked === "string") setPluginPath(picked); }}>Choose…</button></div></div>
                  <div class="mcp-fields"><label>Catalog directory<input class="mono" placeholder="/path/to/catalog" value={sourcePath()} onInput={(e) => setSourcePath(e.currentTarget.value)} /></label><div class="settings-actions"><button class="settings-button" disabled={!sourcePath().trim() || pluginBusy()} onClick={() => void registerPluginSource()}>Register catalog source</button></div></div>
                  <details class="advanced"><summary>Detached Ed25519 evidence (optional)</summary><div class="mcp-fields"><label>Key ID<input class="mono" value={sourceKeyId()} onInput={(e) => setSourceKeyId(e.currentTarget.value)} /></label><label>Public key (base64)<input class="mono" value={sourcePublicKey()} onInput={(e) => setSourcePublicKey(e.currentTarget.value)} /></label><label>Signature (base64)<input class="mono" value={sourceSignature()} onInput={(e) => setSourceSignature(e.currentTarget.value)} /></label></div></details>
                  <Show when={pluginSources().length > 0}><div class="capability-list"><For each={pluginSources()}>{(source) => <div class="capability-item"><strong>{source.label}</strong><small>{source.format} · {source.trust} · {source.enabled ? "Enabled" : "Disabled"} · {source.signature ? (source.signature.verified ? "Signed" : "Signature invalid") : "Unsigned"}</small><code title={source.catalog_digest}>sha256:{source.catalog_digest.slice(0, 16)}</code><div class="settings-actions"><button type="button" class="settings-button" disabled={pluginBusy()} onClick={async () => { setPluginBusy(true); try { await api.pluginSourceAction(source.id, source.enabled ? "disable" : "enable", capabilityScope()); await refreshCapabilities(); } catch (e) { setNotice({ kind: "error", text: `Source action failed: ${e instanceof Error ? e.message : String(e)}` }); } finally { setPluginBusy(false); } }}>{source.enabled ? "Disable source" : "Enable source"}</button><Show when={source.signature}><button type="button" class="settings-button danger" disabled={pluginBusy()} onClick={async () => { setPluginBusy(true); try { await api.pluginKeyAction(source.signature!.key_id, source.signature!.revoked ? "restore" : "revoke", capabilityScope()); await refreshCapabilities(); } catch (e) { setNotice({ kind: "error", text: `Key action failed: ${e instanceof Error ? e.message : String(e)}` }); } finally { setPluginBusy(false); } }}>{source.signature!.revoked ? "Restore key" : "Revoke key"}</button></Show></div></div>}</For></div></Show>
                  <div class="mcp-fields"><label>Search marketplace entries<input value={marketplaceQuery()} placeholder="frontend, testing, release…" onInput={(e) => setMarketplaceQuery(e.currentTarget.value)} onChange={() => void refreshCapabilities()} /></label></div><Show when={marketplaceEntries().length > 0}><div class="capability-list"><For each={marketplaceEntries()}>{(entry) => <div class="capability-item"><strong>{entry.name}</strong><small>{entry.source_label} · {entry.source_enabled ? "Source enabled" : "Source disabled"}{entry.version ? ` · v${entry.version}` : ""}</small><p>{entry.description || "No description"}</p><code title={entry.catalog_digest}>sha256:{entry.catalog_digest.slice(0, 16)}</code><small>{entry.license ? `License: ${entry.license}` : "License: not declared"}</small></div>}</For></div></Show>
                  <Show when={plugins().length > 0} fallback={<div class="capability-empty"><Icon name="grid" /><strong>{scope() === "user" ? "No shared plugins installed" : (inheritedPlugins().length > 0 ? "No project-specific plugins installed" : "No plugins installed")}</strong><span>{scope() === "user" ? "Install a reviewed package to make its skills, commands, and integrations available everywhere." : (inheritedPlugins().length > 0 ? "This project inherits the shared plugins listed below. Install a project-specific package below if needed." : "Install a reviewed local package to make its skills, commands, and integrations available.")}</span></div>}>
                    <div class="capability-list"><For each={plugins()}>{(plugin) => <details class="capability-item"><summary><span><strong>{plugin.name}</strong><small>v{plugin.version} · {plugin.scope} · {plugin.format}</small></span><span class="capability-state" classList={{ ready: plugin.enabled, muted: !plugin.enabled }}>{plugin.enabled ? "Enabled" : "Disabled"}</span></summary><div class="capability-detail"><p>{plugin.description || "No description provided."}</p><code title={plugin.digest}>sha256:{plugin.digest.slice(0, 16)}</code><code>{plugin.trace_id}</code><div class="mcp-controls"><Show when={!plugin.network_denied} fallback={<span class="capability-state muted">Blocked by plugins.network_deny</span>}><label class="mcp-network"><Switch checked={plugin.network_allowed} label={`Allow network for ${plugin.name}`} onChange={(v) => void togglePluginNetwork(plugin.name, v)} /><span title="Deny entries (plugins.network_deny) always win; the allowlist grants egress otherwise. Privileged: set at the trusted (user) level for an untrusted workspace.">{plugin.network_allowed ? "Network allowed from sandbox" : "Local only"}</span></label></Show></div><div class="settings-actions"><button class="settings-button" disabled={pluginBusy()} onClick={() => void mutatePlugin(plugin.name, plugin.enabled ? "disable" : "enable")}>{plugin.enabled ? "Disable" : "Enable"}</button><button class="settings-button" disabled={pluginBusy()} onClick={() => void mutatePlugin(plugin.name, "rollback")}>Rollback</button><button class="settings-button danger" disabled={pluginBusy()} onClick={() => void mutatePlugin(plugin.name, "remove")}>Remove</button></div></div></details>}</For></div>
                  </Show>
                  <Show when={scope() === "workspace" && inheritedPlugins().length > 0}>
                    <div class="inherited-capabilities-group">
                      <div class="inherited-capabilities-head">
                        <Icon name="layers" />
                        <div>
                          <strong>Shared user plugins ({inheritedPlugins().length})</strong>
                          <span>Installed globally. Available across all projects.</span>
                        </div>
                      </div>
                      <div class="capability-list">
                        <For each={inheritedPlugins()}>{(plugin) => (
                          <details class="capability-item">
                            <summary>
                              <span>
                                <strong>{plugin.name}</strong>
                                <small>v{plugin.version} · user · {plugin.format}</small>
                              </span>
                              <span class="capability-state inherited">{plugin.enabled ? "Inherited (Active)" : "Disabled (User)"}</span>
                            </summary>
                            <div class="capability-detail">
                              <p>{plugin.description || "No description provided."}</p>
                              <code title={plugin.digest}>sha256:{plugin.digest.slice(0, 16)}</code>
                              <code>{plugin.trace_id}</code>
                            </div>
                          </details>
                        )}</For>
                      </div>
                    </div>
                  </Show>
                </Group>
              </Show>
            </Show>

            <Show when={page() === "prompts"}>
              <div class="prompt-page">
              <header><h1>Prompts</h1><p>Shape the agent’s voice and working rules without changing what it is allowed to do.</p></header>
              <div class="prompt-scope-note"><Icon name="layers" /><div><strong>{scope() === "user" ? "Shared prompt layer" : "Project prompt layer"}</strong><span>{scope() === "user" ? "Used as the default across your projects. A project can override individual blocks." : "Applies only in this project. Unchanged blocks continue to inherit from Shared."}</span><Show when={promptLayerPath()}><code>{promptLayerPath()}</code></Show></div></div>
              <Group title="Editable blocks">
                <For each={PROMPT_BLOCKS}>{(block) => {
                  const own = () => promptBlockText(block.id);
                  return (
                    <div class="prompt-block">
                      <div class="prompt-block-head">
                        <strong>{block.label}</strong>
                        <span class="capability-state" classList={{ ready: own() !== null, muted: own() === null }}>
                          {own() === null ? "Inherited" : "Set here"}
                        </span>
                      </div>
                      <span class="settings-hint prompt-block-help">{block.help}</span>
                      <Show when={promptEditing() === block.id} fallback={
                        <>
                          <Show when={own() !== null} fallback={<p class="settings-hint prompt-inherited">Inherited from {promptSources(block.id) || "the shipped default"}.</p>}>
                            <pre class="prompt-preview">{own()}</pre>
                          </Show>
                          <div class="settings-actions">
                            <button class="settings-button" onClick={() => { setPromptDraft(own() ?? ""); setPromptEditing(block.id); }}>
                              {own() === null ? "Override" : "Edit"}
                            </button>
                            <Show when={own() !== null}>
                              <button class="settings-button" onClick={() => void savePromptBlock(block.id, null)}>Reset to inherited</button>
                            </Show>
                          </div>
                        </>
                      }>
                        <textarea class="prompt-editor" rows={block.id === "identity" ? 8 : 12} aria-label={`Edit ${block.label} prompt`} value={promptDraft()} onInput={(e) => setPromptDraft(e.currentTarget.value)} placeholder={block.id === "guardrails" ? "- one guardrail per line" : block.id === "surface-note" ? "- this is a public channel; assume anyone can read the reply" : "Plain text or markdown"} />
                        <Show when={block.id === "guardrails"}>
                          {/* Prevents the dangerous belief that adding text here
                              sandboxes anything, which would invite relaxing a
                              real permission rule. */}
                          <div class="settings-callout"><Icon name="shield" /><div><strong>Guardrails instruct the model; they do not enforce anything.</strong><span>A model can misread or be argued out of one. Permissions and the sandbox are the enforcement boundary.</span></div></div>
                        </Show>
                        <div class="settings-actions">
                          <button class="btn primary" onClick={() => void savePromptBlock(block.id, promptDraft())}>Save to {scope() === "user" ? "Shared" : "this project"}</button>
                          <button class="settings-button" onClick={() => setPromptEditing(null)}>Cancel</button>
                        </div>
                      </Show>
                    </div>
                  );
                }}</For>
              </Group>
              <Group title="Effective prompt">
                <p class="settings-hint prompt-effective-intro">What the model actually receives, and where each part came from. Changes apply to new sessions; a running turn keeps the prompt it started with.</p>
                <Row title="Estimated size" description="Spent on every turn of every session."><span class="metric">~{promptEffective()?.estimated_tokens ?? 0} tokens</span></Row>
                <div class="capability-list">
                  <For each={promptEffective()?.layers ?? []}>{(d) => (
                    <div class="capability-inheritance-row">
                      <span><strong>{d.block}</strong><small>{PROMPT_LAYER_LABELS[d.layer]}</small></span>
                      <span class="capability-state ready">{d.bytes}B</span>
                    </div>
                  )}</For>
                </div>
                <pre class="prompt-preview prompt-full">{promptEffective()?.text ?? ""}</pre>
              </Group>
              <Group title="Not editable">
                <div class="settings-callout"><Icon name="shield" /><div><strong>The capability contract, the Surface line, and the skill and MCP lists are code-owned.</strong><span>They describe the callable interface as it actually is. Editing them could only make the model wrong about its own tools.</span></div></div>
              </Group>
              </div>
            </Show>

            <Show when={page() === "advanced"}>
              <header><h1>Advanced</h1><p>Inspect effective limits, paths, and configuration diagnostics.</p></header>
              <Group title="Context">
                <Row title="Context window" description="Maximum model input budget before compaction."><span class="metric">{fmt(config()?.context_window ?? 0)} tokens</span></Row>
                <Row title="Maximum output" description="Provider output-token ceiling."><span class="metric">{fmt(config()?.max_tokens ?? 0)} tokens</span></Row>
              </Group>
              <Group title="Paths">
                <Row title="Project config" description={config()?.paths.project_config ?? ""}><button class="settings-button" onClick={() => void openWorkspaceConfig()}>Open</button></Row>
                <Row title="Global config" description={config()?.paths.global_config ?? "Not configured"}><button type="button" class="settings-button" onClick={() => void copySettingText(config()?.paths.global_config ?? "", "Global config path")}>Copy path</button></Row>
                <Row title="Session store" description={config()?.paths.sessions_home ?? ""}><button type="button" class="settings-button" onClick={() => void copySettingText(config()?.paths.sessions_home ?? "", "Session store path")}>Copy path</button></Row>
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
      <ConfirmModal config={confirmConfig()} onClose={() => setConfirmConfig(null)} />
    </div>
  );
}
