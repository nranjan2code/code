import { trapFocus } from "../focusTrap";
import { createEffect, createMemo, createResource, createSignal, For, onCleanup, onMount, Show, untrack, type JSX } from "solid-js";
import { host } from "../host";
import { canOfferSyntheticMailCalendar, setSyntheticMailCalendarEnabled, syntheticMailCalendarEnabled } from "../mailCalendarDemo";
import { SyntheticMailCalendarDemoControl } from "./SyntheticMailCalendarDemoControl";
import {
  technicalDetails,
  openConnect,
  openInEditor,
  pendingSettingsPage,
  setPendingSettingsPage,
  providers,
  setTechnicalDetails,
  setNotice,
  setProviders,
  setSettingsOpen,
  openArtifactCanvas,
  setSetupEpoch,
  setSettingsScope,
  settingsScope,
  setShowShortcuts,
  uiPreferences,
  updateUiPreference,
  sessions,
  setSessions,
  activeId,
  splitId,
  backend,
  activeAgentId,
  activeAgent,
  connection,
  health,
  setAgentPickerOpen,
  setAgentPickerTab,
  setTranscriptViewId,
} from "../store";
import type { SettingsPageId } from "../store";
import type { ConfigSnapshot, SessionSummary, Trigger } from "../types";
import * as api from "../api";
import { interfaceFonts, contentFonts, codeFonts } from "../typography";
import { watchConfig } from "../streamHub";
import { relTime } from "../time";
import { loadHealth, refreshSessions, openAgentChat, closeSplit } from "../App";
import { sortByRecent } from "../agentRecents";
import { capabilityHue, capabilityInitial } from "../capabilityIcon";
import Icon, { type IconName } from "./Icon";
import AgentMark from "./AgentMark";
import ConfirmModal, { type ConfirmConfig } from "./ConfirmModal";
import OperationsPanel from "./OperationsPanel";
import DigestCard from "./DigestCard";
import Skeleton from "./Skeleton";
import { headlessAuth } from "../headlessAuth";
import { setPendingRecoveryCodes } from "../ownerRecovery";

type Page = SettingsPageId;

// Everyday pages are what anyone changes; Agents holds one page per agent;
// Advanced is shown only with technical details on (docs/design/75 §6.3).
type NavGroup = "Everyday" | "Advanced";

const pages: { id: Page; label: string; icon: IconName; hint: string; group: NavGroup }[] = [
  { id: "general", label: "General", icon: "gear", hint: "suggestions technical details presentation styles shortcuts", group: "Everyday" },
  { id: "appearance", label: "Appearance", icon: "palette", hint: "theme text size motion cards previews", group: "Everyday" },
  { id: "voice", label: "Voice and sound", icon: "mic", hint: "voice speak aloud microphone sound cues chime", group: "Everyday" },
  { id: "notifications", label: "Notifications", icon: "bell", hint: "alerts quiet hours", group: "Everyday" },
  { id: "connections", label: "Capabilities", icon: "grid", hint: "discover skills plugins marketplace connections tools chat bots telegram discord slack mcp hooks", group: "Everyday" },
  { id: "social", label: "Social accounts", icon: "spark", hint: "social youtube linkedin reddit x twitter api key client id accounts", group: "Everyday" },
  { id: "privacy", label: "Privacy and safety", icon: "shield", hint: "permissions access approvals memory remembers archived trash history", group: "Everyday" },
  { id: "models", label: "Models and routing", icon: "layers", hint: "route ladder fallbacks", group: "Advanced" },
  { id: "reliability", label: "Reliability", icon: "timer", hint: "retries timeout circuit breaker", group: "Advanced" },
  { id: "prompts", label: "Prompts", icon: "spark", hint: "system prompt identity rules guardrails persona", group: "Advanced" },
  { id: "services", label: "Services and health", icon: "grid", hint: "gateway bridge tray watchdog background health", group: "Advanced" },
  { id: "storage", label: "Storage and backup", icon: "archive", hint: "paths configuration backup export import", group: "Advanced" },
];

/** Pages that read or write one agent's configuration, so the Shared
 * defaults switch applies to them. */
const SCOPED_PAGES = new Set<Page>(["agent", "connections", "mail-calendar", "social", "privacy", "prompts"]);

const PROMPT_BLOCKS: { id: api.PromptBlock; label: string; help: string }[] = [
  { id: "identity", label: "Identity", help: "Who the agent is. This agent's version replaces the shared one." },
  { id: "operating-rules", label: "Operating rules", help: "How it works. This agent's version replaces the shared one." },
  { id: "guardrails", label: "Guardrails", help: "Shared and agent guardrails all apply; none can be removed here." },
  { id: "surface-note", label: "Surface note", help: "What Vakyartha should know about where its replies appear. Notes from every level add up." },
];

const PROMPT_LAYER_LABELS: Record<api.PromptLayerDescriptor["layer"], string> = {
  seed: "shipped default",
  shared: "Shared",
  workspace: "This agent",
  surface: "surface",
  bot: "bot",
  chat: "chat",
  agent: "agent role",
};

function Switch(props: { checked: boolean; onChange: (next: boolean) => void; label: string }) {
  return <button type="button" class="switch" classList={{ on: props.checked }} role="switch" aria-checked={props.checked} aria-label={props.label} onClick={() => props.onChange(!props.checked)}><span /></button>;
}

function Row(props: { title: string; description: string; children: JSX.Element; danger?: boolean; className?: string }) {
  return <div class={`setting-row${props.className ? ` ${props.className}` : ""}`} classList={{ danger: props.danger }}><div class="setting-copy"><strong>{props.title}</strong><span>{props.description}</span></div><div class="setting-control">{props.children}</div></div>;
}

function Group(props: { title?: string; id?: string; children: JSX.Element }) {
  return <section id={props.id} class="settings-group"><Show when={props.title}><h2>{props.title}</h2></Show><div class="settings-card">{props.children}</div></section>;
}

/** A closed "Technical details" row whose values stay intact behind it
 * (docs/design/75 §6.3). */
function TechnicalRow(props: { children: JSX.Element }) {
  return <details class="settings-technical"><summary><span>Technical details</span><Icon name="chevron" /></summary>{props.children}</details>;
}

function CapabilityIcon(props: { name: string }) {
  return <span class="capability-icon" style={{ "--capability-hue": `oklch(62% 0.14 ${capabilityHue(props.name)})` }}>{capabilityInitial(props.name)}</span>;
}

function SocialIcon(props: { platform: string }) {
  const glyph = () => {
    switch (props.platform) {
      case "social-youtube": return <><rect x="3" y="6" width="18" height="12" rx="4" /><path d="M10 9.5v5l4.5-2.5z" fill="currentColor" stroke="none" /></>;
      case "social-linkedin": return <><rect x="3.5" y="3.5" width="17" height="17" rx="3" /><path d="M8 11v5M8 8v.01M12 16v-5M12 13a2.5 2.5 0 0 1 5 0v3" /></>;
      case "social-reddit": return <><circle cx="12" cy="13" r="6.5" /><circle cx="9.5" cy="12.5" r=".8" fill="currentColor" /><circle cx="14.5" cy="12.5" r=".8" fill="currentColor" /><path d="M9.5 15.5c1.4 1 3.6 1 5 0M12 6.5l1.2-3 3 .8" /></>;
      default: return <path d="M5 5l14 14M19 5L5 19" />;
    }
  };
  return <span class="social-icon" data-platform={props.platform} aria-hidden="true"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">{glyph()}</svg></span>;
}

/** A model limit in plain words: the number and who set it, or that nothing states one. */
function limitText(value: number | null | undefined, source: string | undefined): string {
  if (value == null) return "Not stated by the model";
  const by = source === "setting" ? "your setting" : "the model";
  return `${fmt(value)} tokens · set by ${by}`;
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

const MAIL_CALENDAR_CAPABILITY_LABELS: Record<api.MailCalendarCapability, string> = {
  mail_read: "Read email",
  mail_prepare: "Prepare email drafts",
  mail_send: "Send email",
  calendar_write: "Create or update calendar events after review",
  calendar_free_busy: "Check availability",
  calendar_read: "Read calendar events",
};

function describeMailCalendarCapabilities(capabilities: api.MailCalendarCapability[]): string {
  return capabilities.map((capability) => MAIL_CALENDAR_CAPABILITY_LABELS[capability]).join(", ") || "No access";
}

const DEFAULT_MAIL_CALENDAR_CAPABILITIES: api.MailCalendarCapability[] = ["mail_read", "calendar_free_busy"];

export default function Settings() {
  const [syntheticMailDemo, setSyntheticMailDemo] = createSignal(syntheticMailCalendarEnabled());
  // Presentation memos can run during component setup and read this signal.
  const [config, setConfig] = createSignal<ConfigSnapshot | null>(null);
  const [privacyLayer, setPrivacyLayer] = createSignal<Awaited<ReturnType<typeof api.getPrivacyConfigLayer>> | null>(null);
  const [recoveryBusy, setRecoveryBusy] = createSignal(false);
  const rotateRecovery = async () => {
    setRecoveryBusy(true);
    try {
      const codes = await headlessAuth.rotateRecovery();
      setPendingRecoveryCodes(codes);
      setSettingsOpen(false);
    } catch (error) {
      setNotice({ kind: "error", text: error instanceof Error ? error.message : String(error) });
    } finally {
      setRecoveryBusy(false);
    }
  };
  let settingsMain: HTMLElement | undefined;
  let settingsRoot!: HTMLDivElement;
  const scope = () => settingsScope();
  const capabilityScope = () => scope() === "user" ? "user" as const : "workspace" as const;
  const [confirmConfig, setConfirmConfig] = createSignal<ConfirmConfig | null>(null);
  const [voiceProviders, { refetch: refreshVoiceProviders }] = createResource(
    activeAgentId,
    (agent) => api.listVoiceProviders(agent || undefined),
  );
  const [voiceKeyDraft, setVoiceKeyDraft] = createSignal("");
  const [voiceKeyBusy, setVoiceKeyBusy] = createSignal(false);
  const [voiceTestBusy, setVoiceTestBusy] = createSignal(false);
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
      const storedVersions = [...entries].sort((a, b) => b.spec.revision - a.spec.revision);
      const latest = storedVersions[0];
      const activeRevision = presentationActivation(id)?.revision ?? null;
      // Retired built-in revisions remain stored for historical results, but
      // they are not separate choices in the everyday pack library.
      const versions = latest.origin.owner === "builtin"
        ? storedVersions.filter((entry) => entry.spec.revision === latest.spec.revision || entry.spec.revision === activeRevision)
        : storedVersions;
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
  async function activateAllPresentations() {
    if (presentationBusy()) return;
    setPresentationBusy("all");
    try {
      await api.activateAllPresentations(presentationScope(), presentationOwner());
      await refetchPresentations();
      setNotice({
        kind: "info",
        text: `All presentation packs activated for ${presentationScope() === "user" ? "Shared" : "This agent"}`,
      });
    } catch (e) {
      setNotice({ kind: "error", text: `Could not activate all presentations: ${e instanceof Error ? e.message : String(e)}` });
    } finally {
      setPresentationBusy(null);
    }
  }
  async function deactivateAllPresentations() {
    if (presentationBusy()) return;
    setPresentationBusy("all");
    try {
      await api.deactivateAllPresentations(presentationScope(), presentationOwner());
      await refetchPresentations();
      setNotice({
        kind: "info",
        text: `All presentation packs deactivated for ${presentationScope() === "user" ? "Shared" : "This agent"}`,
      });
    } catch (e) {
      setNotice({ kind: "error", text: `Could not deactivate presentations: ${e instanceof Error ? e.message : String(e)}` });
    } finally {
      setPresentationBusy(null);
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
  async function updateVoice(patch: Record<string, unknown>) {
    try { await api.patchConfig(patch, activeAgentId()); await Promise.all([load(), loadHealth()]); setNotice({ kind: "info", text: "Voice settings saved" }); } catch (e) { setNotice({ kind: "error", text: `Could not save voice settings: ${(e as Error).message}` }); }
  }

  const voiceProviderName = () => config()?.voice?.provider || "";
  const voiceCredentialProvider = () => voiceProviderName() === "gemini" ? "google" : voiceProviderName() === "openai" ? "openai" : "";
  const voiceModelProvider = () => voiceProviderName() === "gemini" ? "google" : voiceProviderName() === "openai" ? "openai" : "";
  const [voiceModels, { refetch: refreshVoiceModels }] = createResource(
    () => [voiceModelProvider(), activeAgentId()] as const,
    ([provider, agent]) => provider ? api.discoverModels(provider, agent || undefined) : Promise.resolve({ provider: "", models: [] }),
  );
  const voiceModelChoices = (operation: "transcription" | "synthesis") => {
    const models = voiceModels()?.models ?? [];
    const capabilities = voiceModels()?.capabilities;
    const advertised = capabilities?.[operation];
    if (voiceProviderName() === "local") return [];
    if (voiceProviderName() === "openai") {
      return models.filter((model) => advertised ? advertised.includes(model) : operation === "synthesis" ? /tts/i.test(model) : /transcri|whisper/i.test(model));
    }
    return models.filter((model) => advertised ? advertised.includes(model) : operation === "synthesis"
      ? /tts/i.test(model)
      : !/tts|image|embed|embedding/i.test(model));
  };
  async function saveVoiceKey() {
    const key = voiceKeyDraft().trim();
    if (!key || voiceKeyBusy()) return;
    if (!voiceCredentialProvider()) return;
    setVoiceKeyBusy(true);
    try {
      await api.putProviderKey(voiceCredentialProvider(), key, "user");
      setVoiceKeyDraft("");
      await refreshVoiceProviders();
      await refreshVoiceModels();
      setNotice({ kind: "info", text: "Voice service key saved to shared credentials. It is available to your Agents." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not save voice key: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setVoiceKeyBusy(false);
    }
  }
  async function removeVoiceKey() {
    if (voiceKeyBusy()) return;
    if (!voiceCredentialProvider()) return;
    setVoiceKeyBusy(true);
    try {
      const removed = await api.removeProviderKey(voiceCredentialProvider(), "user");
      await refreshVoiceProviders();
      setNotice({ kind: removed.shadowed_by_env ? "error" : "info", text: removed.shadowed_by_env
        ? "Saved key removed, but a process environment key is still active. Remove it from the server environment to disconnect voice."
        : "Shared voice service key removed." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not remove voice key: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setVoiceKeyBusy(false);
    }
  }
  async function testVoice() {
    if (voiceTestBusy()) return;
    setVoiceTestBusy(true);
    try {
      const blob = await api.speak("This is a voice test.", {
        provider: voiceProviderName(),
        transcriptionModel: config()?.voice?.transcription_model ?? undefined,
        synthesisModel: config()?.voice?.synthesis_model ?? undefined,
        voiceName: uiPreferences.voiceName || undefined,
        persona: uiPreferences.voicePersona || undefined,
      });
      const url = URL.createObjectURL(blob);
      const audio = new Audio(url);
      audio.addEventListener("ended", () => URL.revokeObjectURL(url), { once: true });
      await audio.play();
      setNotice({ kind: "info", text: "Voice test is playing." });
    } catch (error) {
      setNotice({ kind: "error", text: `Voice test failed: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setVoiceTestBusy(false);
    }
  }

  // Prompt layers (docs/design/45). The layer resource is keyed on scope so
  // switching Shared/This project reloads the editable layer, while the
  // effective composition is scope-independent — it is what the model gets.
  const [promptLayer, { refetch: refetchPromptLayer }] = createResource(scope, (s) => api.getPromptLayer(s));
  const [promptEffective, { refetch: refetchPromptEffective }] = createResource(() => api.getPromptEffective(activeAgentId()));
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
  // On a phone Settings is a list that opens each page; a deep link lands
  // on its page directly.
  const [phoneView, setPhoneView] = createSignal<"list" | "page">(pendingSettingsPage() ? "page" : "list");
  setPendingSettingsPage(null);

  const selectPage = (next: Page) => {
    setPage(next);
    setQuery("");
    setPhoneView("page");
    // Every settings destination is a new document. Retaining the previous
    // page's scroll position made headings disappear above the viewport and
    // made the first visible card look clipped or unstyled.
    queueMicrotask(() => settingsMain?.scrollTo({ top: 0, behavior: "auto" }));
  };
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
      const agent = activeAgentId();
      setNotes((await api.listMemory(agent)).notes);
      setProposals((await api.listProposals(agent)).proposals);
    } catch {
      /* gateway down — lists simply stay stale */
    }
  }

  async function forgetNote(id: string) {
    if (!window.confirm("Forget this memory note? The block is removed from the markdown store; this cannot be undone.")) return;
    try {
      await api.forgetMemory(id, tier(), activeAgentId());
      await refreshLearning();
    } catch (e) {
      setNotice({ kind: "error", text: `Could not forget note: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  async function amendNote(id: string) {
    const text = editText().trim();
    if (!text) return;
    try {
      await api.amendMemory(id, tier(), text, activeAgentId());
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
      await api.appendMemory(tier(), text, noteKind().trim() || "fact", noteTag().trim(), activeAgentId());
      setNoteText("");
      setNoteTag("");
      setAddingNote(false);
      await refreshLearning();
      setNotice({ kind: "info", text: tier() === "profile" ? "Profile note saved — recalled by every agent." : "Note saved for this agent." });
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
  const [inheritedPluginSources, setInheritedPluginSources] = createSignal<api.MarketplaceSource[]>([]);
  const [marketplaceEntries, setMarketplaceEntries] = createSignal<api.MarketplaceEntry[]>([]);
  const [marketplaceErrors, setMarketplaceErrors] = createSignal<string[]>([]);
  const [socialConnectors, setSocialConnectors] = createSignal<api.SocialConnector[]>([]);
  const [youtubeKeyConfigured, setYoutubeKeyConfigured] = createSignal(false);
  const [youtubeInherited, setYoutubeInherited] = createSignal(false);
  const [redditConfigured, setRedditConfigured] = createSignal(false);
  const [redditInherited, setRedditInherited] = createSignal(false);
  const [redditInput, setRedditInput] = createSignal("");
  const [redditQuery, setRedditQuery] = createSignal("");
  const [redditResults, setRedditResults] = createSignal<api.RedditPreviewItem[]>([]);
  const [redditBusy, setRedditBusy] = createSignal(false);
  const [xConfigured, setXConfigured] = createSignal(false);
  const [xInherited, setXInherited] = createSignal(false);
  const [xInput, setXInput] = createSignal("");
  const [xQuery, setXQuery] = createSignal("");
  const [xResults, setXResults] = createSignal<api.XPreviewItem[]>([]);
  const [xBusy, setXBusy] = createSignal(false);
  const [xUsage, setXUsage] = createSignal<api.XUsage | null>(null);
  const [xLimitInput, setXLimitInput] = createSignal("");
  const [linkedinInherited, setLinkedinInherited] = createSignal(false);
  const credScope = () => (scope() === "user" ? "user" as const : undefined);
  const [youtubeKeyInput, setYoutubeKeyInput] = createSignal("");
  const [youtubeQuery, setYoutubeQuery] = createSignal("");
  const [youtubeResults, setYoutubeResults] = createSignal<api.YoutubePreviewItem[]>([]);
  const [youtubeBusy, setYoutubeBusy] = createSignal(false);
  const [linkedinClientIdConfigured, setLinkedinClientIdConfigured] = createSignal(false);
  const [linkedinClientIdInput, setLinkedinClientIdInput] = createSignal("");
  const [linkedinAccount, setLinkedinAccount] = createSignal<api.LinkedinConnection>({ connected: false });
  const [linkedinBusy, setLinkedinBusy] = createSignal(false);
  const [capabilityView, setCapabilityView] = createSignal<"discover" | "mine" | "manage">("discover");
  const [discoveryQuery, setDiscoveryQuery] = createSignal("");
  const [discoveryKind, setDiscoveryKind] = createSignal<"all" | "skills" | "plugins">("all");
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
  const shownCapabilityTab = () => capabilityTab();
  const visibleSkills = createMemo(() =>
    scope() === "user" ? skills().filter((skill) => skill.scope === "user") : skills().filter((skill) => !skill.shadowed),
  );
  const inherits = (kind: "mcp" | "hooks" | "skills" | "commands" | "plugins") => scope() === "user" || config()?.capability_inheritance?.[kind] !== false;
  const [inheritanceBusy, setInheritanceBusy] = createSignal<string | null>(null);
  async function setCapabilityInheritance(kind: "mcp" | "hooks" | "skills" | "commands" | "plugins", enabled: boolean) {
    const key = `inherit_${kind}` as const;
    setInheritanceBusy(kind);
    try {
      await api.patchConfig({ [key]: enabled }, activeAgentId());
      await Promise.all([load(), refreshMcp(), refreshCapabilities()]);
      setNotice({ kind: "info", text: enabled ? `${kindLabel(kind)} now inherit from Shared.` : `${kindLabel(kind)} are now specific to ${agentName()}.` });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not update inheritance: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setInheritanceBusy(null);
    }
  }
  const kindLabel = (kind: "mcp" | "hooks" | "skills" | "commands" | "plugins") => ({ mcp: "Connected apps", hooks: "Hooks", skills: "Skills", commands: "Commands", plugins: "Add-ons" })[kind];
  const discoverableSkills = createMemo(() => visibleSkills().filter((skill) => {
    const needle = discoveryQuery().trim().toLowerCase();
    return (discoveryKind() === "all" || discoveryKind() === "skills") && (!needle || `${skill.name} ${skill.description}`.toLowerCase().includes(needle));
  }));
  const discoverablePlugins = createMemo(() => marketplaceEntries().filter((entry) => {
    const needle = discoveryQuery().trim().toLowerCase();
    return (discoveryKind() === "all" || discoveryKind() === "plugins") && (!needle || `${entry.name} ${entry.description ?? ""} ${entry.source_label}`.toLowerCase().includes(needle));
  }));

  const totalMcpCount = () => {
    const local = Object.keys(mcpServers() ?? {}).length;
    if (scope() === "user" || !inherits("mcp")) return local;
    const inherited = Object.keys(inheritedMcpServers()).filter((k) => !(k in (mcpServers() ?? {}))).length;
    return local + inherited;
  };

  const totalHooksCount = () => {
    const local = hooks().length;
    if (scope() === "user" || !inherits("hooks")) return local;
    return local + inheritedHooks().length;
  };

  const totalPluginsCount = () => {
    const local = plugins().length;
    if (scope() === "user" || !inherits("plugins")) return local;
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
          api.getMcpServers(activeAgentId()),
          api.getGlobalMcpServers(),
        ]);
        setMcpServers(projectRes.servers ?? {});
        setInheritedMcpServers(globalRes.servers ?? {});
      }
      setMcpDirty(false);
    } catch (e) {
      setNotice({ kind: "error", text: `Could not load connections: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  async function refreshCapabilities() {
    try {
      const [socialResult, youtubeKey, linkedinClientId, linkedinConnection, redditId, xToken, xUsageResult] = await Promise.all([
        api.listSocialConnectors(activeAgentId()),
        api.youtubeKeyStatus(activeAgentId(), credScope()),
        api.linkedinClientIdStatus(activeAgentId(), credScope()),
        api.linkedinAccountStatus(activeAgentId()),
        api.redditClientIdStatus(activeAgentId(), credScope()),
        api.xTokenStatus(activeAgentId(), credScope()),
        api.xUsageStatus(activeAgentId()).catch(() => null),
      ]);
      setSocialConnectors(socialResult.connectors ?? []);
      setYoutubeKeyConfigured(youtubeKey.configured);
      setYoutubeInherited(youtubeKey.inherited);
      setLinkedinInherited(linkedinClientId.inherited);
      setRedditConfigured(redditId.configured);
      setRedditInherited(redditId.inherited);
      setXConfigured(xToken.configured);
      setXInherited(xToken.inherited);
      setXUsage(xUsageResult);
      setLinkedinClientIdConfigured(linkedinClientId.configured);
      setLinkedinAccount(linkedinConnection);
      if (scope() === "user") {
        const [skillResult, hookResult, pluginResult, sourceResult, catalogResult] = await Promise.all([
          api.listSkills(activeAgentId()),
          api.getGlobalHooks(),
          api.listPlugins("user", activeAgentId()),
          api.listPluginSources("user", activeAgentId()),
          api.listPluginCatalog("", "user", activeAgentId()),
        ]);
        setSkills(skillResult.skills ?? []);
        setHooks(hookResult.hooks ?? []);
        setInheritedHooks([]);
        setPlugins(pluginResult.plugins ?? []);
        setInheritedPlugins([]);
        setPluginSources(sourceResult.sources ?? []);
        setInheritedPluginSources([]);
        setMarketplaceEntries(catalogResult.entries ?? []);
        setMarketplaceErrors((catalogResult.errors ?? []).map((item) => item.error));
      } else {
        const [skillResult, hookResult, globalHookResult, pluginResult, globalPluginResult, sourceResult, globalSourceResult, catalogResult] = await Promise.all([
          api.listSkills(activeAgentId()),
          api.getHooks(activeAgentId()),
          api.getGlobalHooks(),
          api.listPlugins("workspace", activeAgentId()),
          api.listPlugins("user", activeAgentId()),
          api.listPluginSources("workspace", activeAgentId()),
          api.listPluginSources("user", activeAgentId()),
          api.listPluginCatalog("", undefined, activeAgentId()),
        ]);
        setSkills(skillResult.skills ?? []);
        setHooks(hookResult.hooks ?? []);
        setInheritedHooks(globalHookResult.hooks ?? []);
        setPlugins(pluginResult.plugins ?? []);
        setInheritedPlugins(globalPluginResult.plugins ?? []);
        setPluginSources(sourceResult.sources ?? []);
        setInheritedPluginSources(globalSourceResult.sources ?? []);
        setMarketplaceEntries(catalogResult.entries ?? []);
        setMarketplaceErrors((catalogResult.errors ?? []).map((item) => item.error));
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
      if (action === "remove") await api.removePlugin(name, capabilityScope(), activeAgentId());
      else await api.pluginAction(name, action, capabilityScope(), activeAgentId());
      await refreshCapabilities();
      setNotice({ kind: "info", text: `${name} ${action === "remove" ? "removed" : `${action}d`}.` });
    } catch (e) {
      setNotice({ kind: "error", text: `Plugin action failed: ${e instanceof Error ? e.message : String(e)}` });
    } finally { setPluginBusy(false); }
  }

  async function installSocialGuidance(id: string, platform: string, installScope: "user" | "workspace" = capabilityScope(), update = false) {
    if (pluginBusy()) return;
    setPluginBusy(true);
    try {
      await api.installSocialConnector(id, installScope, activeAgentId());
      await refreshCapabilities();
      setNotice({ kind: "info", text: update ? `${platform} add-on updated. It is off until you review and turn it on again.` : `${platform} add-on installed. Turn it on when you are ready.` });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not ${update ? "update" : "install"} the ${platform} add-on: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setPluginBusy(false);
    }
  }

  async function toggleSocialGuidance(plugin: api.InstalledPlugin) {
    if (pluginBusy()) return;
    setPluginBusy(true);
    try {
      await api.pluginAction(plugin.name, plugin.enabled ? "disable" : "enable", plugin.scope, activeAgentId());
      await refreshCapabilities();
      setNotice({ kind: "info", text: `${plugin.name} guidance ${plugin.enabled ? "disabled" : "enabled"}.` });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not change ${plugin.name}: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setPluginBusy(false);
    }
  }

  async function installSocialPresentationPack(id: string) {
    if (pluginBusy()) return;
    setPluginBusy(true);
    try {
      const result = await api.installSocialPresentations(id, activeAgentId());
      setNotice({ kind: "info", text: `${result.registered} ${socialConnectors().find((connector) => connector.id === id)?.platform ?? ""} layouts added, all off. Choose one to use in Settings, General, Presentation styles.` });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not add presentation layouts: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setPluginBusy(false);
    }
  }

  const linkedinPlugin = () => [...plugins(), ...inheritedPlugins()].find((plugin) => plugin.name === "social-linkedin");

  async function saveLinkedinClientId() {
    const clientId = linkedinClientIdInput().trim();
    if (!clientId || linkedinBusy()) return;
    setLinkedinBusy(true);
    try {
      await api.saveLinkedinClientId(clientId, activeAgentId(), credScope());
      setLinkedinClientIdInput("");
      setLinkedinClientIdConfigured(true);
      setNotice({ kind: "info", text: "LinkedIn Client ID saved" + (scope() === "user" ? " for every agent" : " for this agent") + "." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not save LinkedIn Client ID: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setLinkedinBusy(false);
    }
  }

  async function connectLinkedinAccount() {
    if (linkedinBusy()) return;
    let popup: Window | null = null;
    if (host.kind !== "desktop") {
      popup = window.open("about:blank", "_blank");
      if (!popup) {
        setNotice({ kind: "error", text: "Allow a new window to continue to LinkedIn." });
        return;
      }
      popup.opener = null;
    }
    setLinkedinBusy(true);
    try {
      const result = await api.beginLinkedinOAuth(activeAgentId());
      if (host.kind === "desktop") {
        if (!host.openOAuthUrl) throw new Error("System browser support is unavailable.");
        await host.openOAuthUrl(result.authorization_url);
      } else if (popup) {
        popup.location.replace(result.authorization_url);
      }
      setNotice({ kind: "info", text: "Finish LinkedIn sign-in in your browser, then return to Vakyartha. Only owner-visible profile identity is connected; LinkedIn content search remains unavailable." });
    } catch (error) {
      popup?.close();
      setNotice({ kind: "error", text: `Could not start LinkedIn sign-in: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setLinkedinBusy(false);
    }
  }

  async function disconnectLinkedinAccount() {
    if (linkedinBusy()) return;
    setLinkedinBusy(true);
    try {
      await api.disconnectLinkedinAccount(activeAgentId());
      setLinkedinAccount({ connected: false });
      setNotice({ kind: "info", text: "LinkedIn account disconnected and its saved credential removed." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not disconnect LinkedIn: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setLinkedinBusy(false);
    }
  }

  const refreshLinkedinOnFocus = () => {
    if (page() === "social") {
      void Promise.all([
        api.linkedinClientIdStatus(activeAgentId(), credScope()).then((result) => { setLinkedinClientIdConfigured(result.configured); setLinkedinInherited(result.inherited); }),
        api.linkedinAccountStatus(activeAgentId()).then(setLinkedinAccount),
      ]).catch((error) => setNotice({ kind: "error", text: `Could not refresh LinkedIn connection: ${error instanceof Error ? error.message : String(error)}` }));
    }
  };
  onMount(() => {
    window.addEventListener("focus", refreshLinkedinOnFocus);
    onCleanup(() => window.removeEventListener("focus", refreshLinkedinOnFocus));
  });

  async function installPlugin(update = false) {
    const path = pluginPath().trim();
    if (!path || pluginBusy()) return;
    setPluginBusy(true);
    try {
      const pluginScope = scope() === "user" ? "user" : "workspace";
      if (update) await api.updatePlugin(path, pluginScope, activeAgentId());
      else await api.installPlugin(path, pluginScope, activeAgentId());
      setPluginPath("");
      await refreshCapabilities();
      setNotice({ kind: "info", text: update ? "Plugin generation staged disabled." : "Plugin installed disabled." });
    } catch (e) {
      setNotice({ kind: "error", text: `Plugin install failed: ${e instanceof Error ? e.message : String(e)}` });
    } finally { setPluginBusy(false); }
  }

  async function installCatalogPlugin(entry: api.MarketplaceEntry, update = false) {
    if (pluginBusy() || !entry.source_enabled) return;
    setPluginBusy(true);
    try {
      await api.installCatalogPlugin(entry, capabilityScope(), activeAgentId(), update);
      await refreshCapabilities();
      setNotice({ kind: "info", text: update ? `${entry.name} update staged disabled. Review its access under Manage before enabling it.` : `${entry.name} installed disabled. Review its access under Manage before enabling it.` });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not install ${entry.name}: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setPluginBusy(false);
    }
  }

  async function togglePluginNetwork(name: string, on: boolean) {
    const scope = capabilityScope();
    if (pluginBusy()) return;
    const plugin = plugins().find((p) => p.name === name);
    // Deny always wins: a plugin on plugins.network_deny cannot be granted
    // egress from the toggle, because the config tier re-evaluates the deny
    // list on every admission.
    if (on && plugin?.network_denied) {
      setNotice({ kind: "error", text: `${name} is blocked by plugins.network_deny; remove that entry to let it use the network.` });
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
      else await api.patchConfig({ plugins_network_allow: grant }, activeAgentId());
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
      await api.registerPluginSource(path, "Desktop catalog", capabilityScope(), signed, activeAgentId());
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
      else await api.putHooks(hooks(), activeAgentId());
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
      else await api.putMcpServers(servers, activeAgentId());
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
      await api.promoteProposal(id, activeAgentId());
      await refreshLearning();
    } catch (e) {
      setNotice({ kind: "error", text: e instanceof Error ? e.message : String(e) });
    }
  }

  async function reject(id: string) {
    try {
      await api.rejectProposal(id, activeAgentId());
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
  const [maxTurns, setMaxTurns] = createSignal(40);
  const [saving, setSaving] = createSignal(false);
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
    if (page() !== "privacy") return;
    void refreshLearning();
    const t = setInterval(() => void refreshLearning(), 8000);
    onCleanup(() => clearInterval(t));
  });
  createEffect(() => {
    if (page() !== "connections" && page() !== "social") return;
    scope();
    activeAgentId();
    if (page() === "connections") void refreshMcp();
    void refreshCapabilities();
  });

  const [sharedRoute, { refetch: refreshSharedRoute }] = createResource(() => api.getGlobalRoute());
  const displayedProvider = () => scope() === "user" ? sharedRoute()?.provider || "" : config()?.provider || "";
  const displayedModel = () => scope() === "user" ? sharedRoute()?.model || "" : config()?.model || "";
  const privacyPermissionMode = () => scope() === "user"
    ? privacyLayer()?.permission_mode ?? "WorkspaceWrite"
    : config()?.permission_mode;
  const privacyApprovalMode = () => scope() === "user"
    ? privacyLayer()?.approval_mode ?? "ask"
    : config()?.approval_mode;
  const privacyMemory = () => scope() === "user"
    ? {
        search_enabled: privacyLayer()?.memory?.search_enabled ?? true,
        write_enabled: privacyLayer()?.memory?.write_enabled ?? true,
        reflection: privacyLayer()?.memory?.reflection ?? false,
        skill_proposals: privacyLayer()?.memory?.skill_proposals ?? true,
      }
    : config()?.memory ?? { search_enabled: true, write_enabled: true, reflection: false, skill_proposals: true };
  const privacyRules = () => scope() === "user"
    ? {
        allow: privacyLayer()?.permissions?.allow ?? [],
        ask: privacyLayer()?.permissions?.ask ?? [],
        deny: privacyLayer()?.permissions?.deny ?? [],
      }
    : config()?.permissions ?? { allow: [], ask: [], deny: [] };
  const currentProviderInfo = () => providers()?.providers.find((p) => p.name === displayedProvider());
  const keyProviderLabel = () => api.providerLabel(providers()?.providers, displayedProvider());
  const changeService = () => {
    const targetScope = scope() === "user" ? "user" : "project";
    setSettingsOpen(false);
    openConnect(targetScope);
  };

  const load = async () => {
    setLoading(true);
    try {
      const next = await api.getConfig(activeAgentId());
      setConfig(next);
      if (page() === "privacy") {
        setPrivacyLayer(await api.getPrivacyConfigLayer(scope() === "user" ? "user" : "workspace", activeAgentId()));
      }
      setMaxTurns(next.max_turns);
      setEvidenceAgeHours(Math.max(0, Math.round((next.intent_evidence_max_age_secs ?? 86400) / 3600)));
    } catch (error) {
      setNotice({ kind: "error", text: `Could not load settings: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setLoading(false);
    }
  };
  createEffect(() => {
    if (page() !== "privacy") return;
    const currentScope = scope() === "user" ? "user" : "workspace";
    const agent = activeAgentId();
    void api.getPrivacyConfigLayer(currentScope, agent).then(setPrivacyLayer).catch(() => setPrivacyLayer(null));
  });
  onMount(() => void load());
  // Live-reflect config/credential writes made elsewhere (CLI `vak setup`,
  // another client) instead of only ever showing what was true at mount
  // time (docs/design/44-shared-config.md, "Liveness").
  onMount(() => {
    const stop = watchConfig(() => { void load(); void loadProviders(); void refreshSharedRoute(); });
    onCleanup(stop);
  });
  const matches = (text: string) => {
    const needle = query().trim().toLowerCase();
    return !needle || text.toLowerCase().includes(needle);
  };
  const pageGroups = createMemo(() => {
    const visible = pages.filter((item) => matches(`${item.label} ${item.hint}`) && (item.group === "Everyday" || technicalDetails()));
    return (["Everyday", "Advanced"] as const)
      .map((group) => [group, visible.filter((item) => item.group === group)] as const)
      .filter(([, items]) => items.length > 0);
  });

  // One Agents entry per agent: picking one opens that agent's page.
  const [settingsAgents, setSettingsAgents] = createSignal<api.Agent[]>([]);
  createEffect(() => {
    void api.listAgents().then((r) => setSettingsAgents(r.agents)).catch(() => setSettingsAgents([]));
  });
  const agentEntries = createMemo(() => [
    { id: "vak", name: "Vakyartha", character: "vak", animation: "subtle" as const },
    ...sortByRecent(settingsAgents().filter((a) => a.id !== "vak" && (a.lifecycle ?? "active") === "active")),
  ].filter((agent) => matches(`${agent.name} agent model ai service key turns helpers context`)));
  const openAgentPage = async (id: string) => {
    setSettingsScope("workspace");
    selectPage("agent");
    if (id === activeAgentId()) return;
    setLoading(true);
    await openAgentChat(id);
    await load();
  };
  const agentName = () => activeAgentId() === "vak" ? "Vakyartha" : activeAgent()?.name ?? "Vakyartha";
  const agentLook = () => activeAgentId() === "vak" ? { character: "vak", animation: "subtle" as const } : { character: activeAgent()?.character ?? "vak", animation: activeAgent()?.animation ?? "subtle" };
  const [mailCalendarAccounts, { refetch: refreshMailCalendarAccounts }] = createResource(
    () => page() === "mail-calendar" ? `${activeAgentId()}:${syntheticMailDemo()}` : null,
    (source) => source ? api.listMailCalendarAccounts(source.split(":")[0]) : Promise.resolve({ accounts: [] }),
  );
  const [mailCalendarTasks, { refetch: refreshMailCalendarTasks }] = createResource(
    () => page() === "mail-calendar" && !syntheticMailDemo() ? activeAgentId() : null,
    async (agentId) => (await api.listTriggers()).triggers.filter(
      (task) => !!task.scope && task.agent === agentId,
    ),
  );
  const [mailCalendarPausing, setMailCalendarPausing] = createSignal<string | null>(null);
  const refreshMailCalendarOnFocus = () => {
    if (page() === "mail-calendar") {
      void Promise.resolve(refreshMailCalendarAccounts()).then(() => api.notifyMailCalendarChanged());
    }
  };
  onMount(() => {
    window.addEventListener("focus", refreshMailCalendarOnFocus);
    onCleanup(() => window.removeEventListener("focus", refreshMailCalendarOnFocus));
  });
  const [mailCalendarBusy, setMailCalendarBusy] = createSignal(false);
  const [mailCalendarCapabilities, setMailCalendarCapabilities] = createSignal<api.MailCalendarCapability[]>([...DEFAULT_MAIL_CALENDAR_CAPABILITIES]);
  const [icloudEmail, setIcloudEmail] = createSignal("");
  const [icloudAppPassword, setIcloudAppPassword] = createSignal("");
  const [googleAppEmail, setGoogleAppEmail] = createSignal("");
  const [googleAppPassword, setGoogleAppPassword] = createSignal("");
  const [microsoftAppEmail, setMicrosoftAppEmail] = createSignal("");
  const [microsoftAppPassword, setMicrosoftAppPassword] = createSignal("");
  const canAddLocalAppPassword = () => host.kind === "desktop"
    || (typeof window !== "undefined" && ["localhost", "127.0.0.1", "::1", "[::1]"].includes(window.location.hostname));
  const connectMailCalendar = async (provider: api.MailCalendarProvider) => {
    if (host.kind === "desktop") {
      setMailCalendarBusy(true);
      try {
        const result = await api.beginMailCalendarOAuth(activeAgentId(), provider, mailCalendarCapabilities());
        if (!host.openOAuthUrl) throw new Error("System browser support is unavailable.");
        await host.openOAuthUrl(result.authorization_url);
        setNotice({ kind: "info", text: "Finish signing in in your system browser, then return here. The account list refreshes when this window regains focus." });
      } catch (error) {
        setNotice({ kind: "error", text: `Could not start account linking: ${error instanceof Error ? error.message : String(error)}` });
      } finally {
        setMailCalendarBusy(false);
      }
      return;
    }
    const popup = window.open("about:blank", "_blank");
    if (!popup) {
      setNotice({ kind: "error", text: "Allow a new window to continue to your provider." });
      return;
    }
    popup.opener = null;
    setMailCalendarBusy(true);
    try {
      const result = await api.beginMailCalendarOAuth(activeAgentId(), provider, mailCalendarCapabilities());
      popup.location.replace(result.authorization_url);
    } catch (error) {
      popup.close();
      setNotice({ kind: "error", text: `Could not start account linking: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setMailCalendarBusy(false);
    }
  };
  const connectIcloud = async () => {
    const email = icloudEmail();
    const selectedCapabilities = mailCalendarCapabilities();
    let appPassword = icloudAppPassword();
    // Do not keep the secret in reactive UI state while the request is in
    // flight; retain only the short-lived local needed for this submission.
    setIcloudAppPassword("");
    setIcloudEmail("");
    setMailCalendarBusy(true);
    try {
      await api.connectIcloudAccount(activeAgentId(), email, appPassword, selectedCapabilities);
      appPassword = "";
      await refreshMailCalendarAccounts();
      api.notifyMailCalendarChanged();
      setNotice({ kind: "info", text: `iCloud sign-in was verified separately for the selected access: ${describeMailCalendarCapabilities(selectedCapabilities)}. Only these read capabilities are enabled; revoke the app-specific password at Apple when you disconnect.` });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not connect the iCloud account: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      appPassword = "";
      setMailCalendarBusy(false);
    }
  };
  const connectGoogleAppPassword = async () => {
    const email = googleAppEmail();
    let password = googleAppPassword();
    setGoogleAppEmail("");
    setGoogleAppPassword("");
    setMailCalendarBusy(true);
    try {
      await api.connectGoogleAppPassword(activeAgentId(), email, password);
      password = "";
      await refreshMailCalendarAccounts();
      api.notifyMailCalendarChanged();
      setNotice({ kind: "info", text: "Gmail sign-in was verified. This account can read bounded inbox metadata only. Revoke the App Password from your Google Account security settings." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not connect Gmail with an App Password: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      password = "";
      setMailCalendarBusy(false);
    }
  };
  const connectMicrosoftAppPassword = async () => {
    const email = microsoftAppEmail();
    let password = microsoftAppPassword();
    setMicrosoftAppEmail("");
    setMicrosoftAppPassword("");
    setMailCalendarBusy(true);
    try {
      await api.connectMicrosoftAppPassword(activeAgentId(), email, password);
      password = "";
      await refreshMailCalendarAccounts();
      api.notifyMailCalendarChanged();
      setNotice({ kind: "info", text: "Outlook.com app-password sign-in was verified. This connection can read email only; calendar and provider changes are unavailable." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not connect the Outlook.com account: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      password = "";
      setMailCalendarBusy(false);
    }
  };
  const disconnectMailCalendar = (account: api.MailCalendarAccount) => setConfirmConfig({
    title: account.revoked_at ? "Finish disconnect cleanup?" : "Disconnect this account?",
    description: account.revoked_at
      ? "Retry removing any Vakyartha sign-in details left by an interrupted disconnect. This does not erase saved copies of messages or events in Vakyartha."
      : "Vakyartha will remove this Agent's saved sign-in details and try to revoke provider access where supported. This does not delete messages or events from your provider or erase copies already saved in Vakyartha.",
    confirmLabel: account.revoked_at ? "Finish cleanup" : "Disconnect account",
    isDanger: true,
    onConfirm: async () => {
      setMailCalendarBusy(false);
      const result = await api.disconnectMailCalendarAccount(activeAgentId(), account.id);
      await refreshMailCalendarAccounts();
      api.notifyMailCalendarChanged();
      setNotice({ kind: "info", text: result.already_disconnected
        ? "Local credential cleanup was retried. Provider revocation was not attempted again, and provider content was not erased."
        : result.provider_revocation === "confirmed"
          ? "The account was disconnected and its provider grant was revoked."
          : result.provider_revocation === "unsupported"
            ? "Local access was removed. This provider does not offer grant revocation through Vakyartha; manage connected-app access with the provider."
            : "Local access was removed, but the provider did not confirm revocation. Check connected-app access with the provider." });
    },
  });
  const refreshMailCalendarAccount = async (account: api.MailCalendarAccount) => {
    setMailCalendarBusy(true);
    try {
      await api.refreshMailCalendarAccount(activeAgentId(), account.id);
      await refreshMailCalendarAccounts();
      api.notifyMailCalendarChanged();
      setNotice({ kind: "info", text: "The provider sign-in was refreshed securely." });
    } catch (error) {
      await refreshMailCalendarAccounts();
      setNotice({ kind: "error", text: `Could not refresh the provider sign-in: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setMailCalendarBusy(false);
    }
  };
  const pauseMailCalendarTask = async (task: Trigger) => {
    await api.putTrigger(task.id, api.draftOf(task, { enabled: false }));
    const session = task.last_run?.status === "running" ? task.last_run.sessions?.[0] : undefined;
    if (session) await api.cancelRun(session);
  };
  const pauseMailCalendarRoutines = async (accountId?: string) => {
    const scope = accountId ?? "all";
    const selected = (mailCalendarTasks() ?? []).filter((task) =>
      task.enabled && (!accountId || task.scope?.account_id === accountId),
    );
    if (selected.length === 0) return;
    setMailCalendarPausing(scope);
    try {
      const results = await Promise.allSettled(selected.map(pauseMailCalendarTask));
      let refreshed = true;
      try {
        await refreshMailCalendarTasks();
      } catch {
        refreshed = false;
      }
      const paused = results.filter((result) => result.status === "fulfilled").length;
      const failed = results.length - paused;
      setNotice({
        kind: failed === 0 && refreshed ? "info" : "error",
        text: failed === 0 && refreshed
          ? `Paused ${paused} ${paused === 1 ? "routine" : "routines"}${accountId ? " for this account" : ""}. Any active run was asked to stop.`
          : `Paused ${paused} ${paused === 1 ? "routine" : "routines"}; ${failed ? `${failed} could not be fully paused` : "the updated status could not be reloaded"}. Check their status and retry.`,
      });
    } finally {
      setMailCalendarPausing(null);
    }
  };
  const archivedSessions = createMemo(() => sessions().filter((session) => session.archived));
  const [trashedSessions, setTrashedSessions] = createSignal<SessionSummary[]>([]);
  const refreshTrash = async () => {
    try {
      setTrashedSessions((await api.listTrash()).sessions);
    } catch {
      setTrashedSessions([]);
    }
  };
  createEffect(() => {
    if (page() === "archived") void refreshTrash();
  });
  // The client keeps an open conversation listed even when the server no
  // longer lists it, so a trashed one is dropped here and left for another.
  const leaveTrashed = async (ids: string[]) => {
    const trashed = new Set(ids);
    const agentId = activeAgentId();
    setSessions((current) => current.filter((session) => !trashed.has(session.session_id)));
    const split = splitId();
    if (split && trashed.has(split)) closeSplit();
    const active = activeId();
    if (active && trashed.has(active)) await openAgentChat(agentId);
  };
  const restoreFromTrash = async (id: string) => {
    try {
      await api.restoreFromTrash(id);
      await Promise.all([refreshSessions(), refreshTrash()]);
      setNotice({ kind: "info", text: "Task taken out of the trash. It is back in Archived tasks." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not restore that task: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  const restoreTask = async (id: string) => {
    try {
      await api.setArchived(id, false);
      await refreshSessions();
      setNotice({ kind: "info", text: "Task restored to the sidebar." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not restore that task: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  const trashTask = (id: string) => {
    const task = archivedSessions().find((session) => session.session_id === id);
    setConfirmConfig({
      title: `Move “${task?.title || "Untitled task"}” to the trash?`,
      description: "It will be hidden everywhere, including search and past-conversation recall. Nothing is erased: you can restore it from the trash.",
      confirmLabel: "Move to trash",
      cancelLabel: "Cancel",
      isDanger: true,
      onConfirm: async () => {
        try {
          await api.trashSession(id);
          await leaveTrashed([id]);
          await Promise.all([refreshSessions(), refreshTrash()]);
          setNotice({ kind: "info", text: "Task moved to the trash." });
        } catch (error) {
          setNotice({ kind: "error", text: `Could not move that task to the trash: ${error instanceof Error ? error.message : String(error)}` });
        }
      },
    });
  };

  const trashAllArchived = () => {
    if (!archivedSessions().length) return;
    setConfirmConfig({
      title: `Move all ${archivedSessions().length} archived tasks to the trash?`,
      description: "They will be hidden everywhere, including search and past-conversation recall. Nothing is erased: you can restore each one from the trash.",
      confirmLabel: "Move all to trash",
      cancelLabel: "Cancel",
      isDanger: true,
      onConfirm: async () => {
        try {
          const trashed = archivedSessions().map((session) => session.session_id);
          const result = await api.trashAllArchived();
          await leaveTrashed(trashed);
          await Promise.all([refreshSessions(), refreshTrash()]);
          setNotice({ kind: "info", text: `${result.trashed} archived task${result.trashed === 1 ? "" : "s"} moved to the trash.` });
        } catch (error) {
          setNotice({ kind: "error", text: `Could not move archived tasks to the trash: ${error instanceof Error ? error.message : String(error)}` });
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
    return maxTurns() !== c.max_turns || evidenceAgeHours() * 3600 !== (c.intent_evidence_max_age_secs ?? 86400);
  };

  const applyAgent = async () => {
    setSaving(true);
    try {
      if (scope() === "user") await api.patchGlobalConfig({ max_turns: maxTurns() });
      else await api.patchConfig({ max_turns: maxTurns() }, activeAgentId());
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

  const removeKey = async () => {
    const name = displayedProvider();
    setKeyBusy(true);
    try {
      const res = await api.removeProviderKey(name);
      await loadHealth();
      const refreshed = await api.listProviders();
      setProviders(refreshed);
      setSetupEpoch((n) => n + 1);
      setNotice(
        (res.shadowed_by_env || refreshed.providers.find((p) => p.name === name)?.configured)
          ? { kind: "error", text: `Removed the shared key, but another key is still available, so ${api.providerLabel(providers()?.providers, name)} can still use it.` }
          : { kind: "info", text: "Key removed." },
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
      else await api.patchConfig({ permission_mode: wire }, activeAgentId());
      if (scope() === "user") setPrivacyLayer((current) => current ? { ...current, permission_mode: wire } : current);
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
      else await api.patchConfig({ approval_mode: mode }, activeAgentId());
      if (scope() === "user") setPrivacyLayer((current) => current ? { ...current, approval_mode: mode } : current);
      setConfig((current) => (current ? { ...current, approval_mode: mode } : current));
    } catch (error) {
      setNotice({
        kind: "error",
        text: `Could not update approvals: ${error instanceof Error ? error.message : String(error)}`,
      });
    }
  };

  const changeMemoryPolicy = async (
    key: "memory_search_enabled" | "memory_write_enabled" | "memory_reflection" | "memory_skill_proposals",
    value: boolean,
  ) => {
    try {
      const patch = { [key]: value };
      if (scope() === "user") await api.patchGlobalConfig(patch);
      else await api.patchConfig(patch, activeAgentId());
      if (scope() === "user") setPrivacyLayer((current) => current ? {
        ...current,
        memory: {
          ...current.memory,
          search_enabled: key === "memory_search_enabled" ? value : current.memory?.search_enabled,
          write_enabled: key === "memory_write_enabled" ? value : current.memory?.write_enabled,
          reflection: key === "memory_reflection" ? value : current.memory?.reflection,
          skill_proposals: key === "memory_skill_proposals" ? value : current.memory?.skill_proposals,
        },
      } : current);
      setConfig((current) => current ? {
        ...current,
        memory: {
          ...current.memory,
          search_enabled: key === "memory_search_enabled" ? value : current.memory.search_enabled,
          write_enabled: key === "memory_write_enabled" ? value : current.memory.write_enabled,
          reflection: key === "memory_reflection" ? value : current.memory.reflection,
          skill_proposals: key === "memory_skill_proposals" ? value : current.memory.skill_proposals,
        },
      } : current);
    } catch (error) {
      setNotice({ kind: "error", text: `Could not update memory settings: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  const openWorkspaceConfig = async () => {
    const relative = ".vak/config.toml";
    const sid = activeId();
    if (!sid) return;
    try {
      await api.readFile(relative, sid);
    } catch {
      await api.writeFile(relative, `permission_mode = "workspace-write"\n`, sid);
    }
    openInEditor(relative);
    setSettingsOpen(false);
  };

  return (
    <div ref={settingsRoot} class="settings-shell" data-phone-view={phoneView()} role="dialog" aria-modal="true" aria-label="Settings" use:trapFocus>
      <aside class="settings-nav">
        <div class="window-drag-strip" data-titlebar aria-hidden="true" />
        <button type="button" class="settings-back" onClick={() => setSettingsOpen(false)}><Icon name="chevron" /><span>Back to Vakyartha</span></button>
        <div class="settings-search"><Icon name="search" /><input aria-label="Search settings" placeholder="Search settings…" value={query()} onInput={(event) => setQuery(event.currentTarget.value)} /></div>
        <Show when={pageGroups().length > 0 || agentEntries().length > 0} fallback={<div class="settings-no-results">No matching settings</div>}>
          <For each={pageGroups().filter(([group]) => group === "Everyday")}>
            {([group, items]) => <div class="settings-nav-group"><div class="settings-nav-label">{group}</div><nav><For each={items}>{(item) => <button type="button" classList={{ active: page() === item.id || (item.id === "privacy" && page() === "archived") }} onClick={() => selectPage(item.id)}><Icon name={item.icon} /><span>{item.label}</span></button>}</For></nav></div>}
          </For>
          <Show when={agentEntries().length > 0}>
            <div class="settings-nav-group"><div class="settings-nav-label">Agents</div><nav><For each={agentEntries()}>{(agent) => <><button type="button" classList={{ active: page() === "agent" && scope() === "workspace" && activeAgentId() === agent.id }} onClick={() => void openAgentPage(agent.id)}><AgentMark character={agent.character} motion={agent.animation} size={24} /><span>{agent.name}</span></button><Show when={scope() === "workspace" && activeAgentId() === agent.id}><div class="settings-agent-subnav" aria-label={`${agent.name} settings`}><button type="button" classList={{ active: page() === "agent" }} onClick={() => selectPage("agent")}>Overview</button><button type="button" classList={{ active: page() === "connections" }} onClick={() => { setCapabilityView("mine"); selectPage("connections"); }}>Capabilities</button><button type="button" classList={{ active: page() === "mail-calendar" }} onClick={() => selectPage("mail-calendar")}>Email and calendar</button><button type="button" classList={{ active: page() === "social" }} onClick={() => selectPage("social")}>Social accounts</button><button type="button" classList={{ active: page() === "privacy" }} onClick={() => selectPage("privacy")}>Privacy and safety</button><Show when={technicalDetails()}><button type="button" classList={{ active: page() === "prompts" }} onClick={() => selectPage("prompts")}>Prompts</button></Show></div></Show></>}</For></nav></div>
          </Show>
          <For each={pageGroups().filter(([group]) => group === "Advanced")}>
            {([group, items]) => <div class="settings-nav-group"><div class="settings-nav-label">{group}</div><nav><For each={items}>{(item) => <button type="button" classList={{ active: page() === item.id }} onClick={() => selectPage(item.id)}><Icon name={item.icon} /><span>{item.label}</span></button>}</For></nav></div>}
          </For>
        </Show>
        <div class="settings-nav-foot"><div class="settings-app-mark"><img src={`${import.meta.env.BASE_URL}vak-icon.png`} alt="" /></div><div><strong>Vakyartha</strong><span>{backend().version ? `Version ${backend().version}` : "Version unavailable"}</span></div></div>
      </aside>

      <main ref={settingsMain} class="settings-main">
        <div class="window-drag-strip settings-main-strip" data-titlebar aria-hidden="true" />
        <div class="settings-content">
          <button type="button" class="settings-page-back" onClick={() => setPhoneView("list")}><Icon name="chevron" /><span>Settings</span></button>
          <Show when={voiceProviders.error || presentationLibrary.error || promptLayer.error || promptEffective.error}>
            <div class="settings-load-error" role="alert">
              Some settings could not be loaded. Existing values were kept; use the relevant section's refresh or reopen Settings to retry.
              <Show when={voiceProviders.error}><span> Voice discovery: {String(voiceProviders.error)}</span></Show>
              <Show when={presentationLibrary.error}><span> Presentations: {String(presentationLibrary.error)}</span></Show>
              <Show when={promptLayer.error || promptEffective.error}><span> Prompt configuration is unavailable.</span></Show>
            </div>
          </Show>
          <Show when={!loading()} fallback={<Skeleton kind="blocks" class="settings-loading" label="Loading settings" />}>
            <Show when={SCOPED_PAGES.has(page())}>
              {/* Editing one agent is the default; the shared defaults are the
                  deliberate, secondary choice, so they sit behind a quiet link. */}
              <Show
                when={scope() === "user"}
                fallback={<button type="button" class="settings-scope-link" onClick={() => setSettingsScope("user")}>Change the shared defaults for every agent instead</button>}
              >
                <button type="button" class="settings-scope-link" onClick={() => setSettingsScope("workspace")}>Back to {agentName()}</button>
              </Show>
            </Show>
            <Show when={page() === "general"}>
              <header><h1>General</h1><p>How Vakyartha looks after you day to day.</p></header>
              <Group>
                <Row title="Suggested prompts" description="Show useful starting points when a task has no conversation yet."><Switch label="Suggested prompts" checked={uiPreferences.suggestions} onChange={(value) => updateUiPreference("suggestions", value)} /></Row>
                <Row title="Show technical details" description="Folders, file paths, IDs, tool output and the technical settings pages. Changes what you see, never what an agent may do."><Switch label="Show technical details" checked={technicalDetails()} onChange={(value) => setTechnicalDetails(value)} /></Row>
                <Row title="Presentation styles" description="Choose how Vakyartha presents different kinds of results."><span class="settings-value">{presentationLibrary.loading ? "Loading…" : `${activePresentationCount()} active · ${presentationCatalog().reduce((count, group) => count + group.types.length, 0)} available`}</span></Row>
                <Row title="Share presentation styles" description="Export your styles or import a collection."><span class="settings-actions"><button class="settings-button" onClick={() => void exportPresentationPack()}>Export</button><label class="settings-button">Import<input type="file" accept="application/json,.json" hidden onChange={importPresentationPack} /></label></span></Row>
                <Show when={!presentationLibrary.loading && (presentationLibrary()?.definitions.length ?? 0) > 0}>
                  <details class="presentation-library-disclosure">
                    <summary>Manage presentation styles</summary>
                  <section class="presentation-library" aria-label="Presentation styles">
                    <div class="presentation-toolbar">
                      <label class="presentation-search"><Icon name="search" size={14} /><input aria-label="Search presentations" placeholder="Search by name or capability…" value={presentationQuery()} onInput={(event) => setPresentationQuery(event.currentTarget.value)} /></label>
                      <div class="presentation-filters" role="group" aria-label="Presentation filter">
                        <For each={[{ id: "all", label: "All" }, { id: "active", label: "Active" }, { id: "inactive", label: "Inactive" }] as const}>{(filter) => <button type="button" classList={{ active: presentationFilter() === filter.id }} aria-pressed={presentationFilter() === filter.id} onClick={() => setPresentationFilter(filter.id)}>{filter.label}</button>}</For>
                      </div>
                      <div class="presentation-bulk-actions">
                        <button type="button" class="settings-button" disabled={presentationBusy() !== null} onClick={() => void activateAllPresentations()}>{presentationBusy() === "all" ? "Working…" : "Activate all"}</button>
                        <button type="button" class="settings-button subtle" disabled={presentationBusy() !== null || activePresentationCount() === 0} onClick={() => void deactivateAllPresentations()}>Deactivate all</button>
                      </div>
                    </div>
                    <div class="presentation-scope-note"><Icon name={presentationScope() === "user" ? "layers" : "folder"} /><span>Managing <strong>{presentationScope() === "user" ? "Shared" : "This agent"}</strong>. These activations are selected before broader fallbacks.</span></div>
                    <Show when={presentationGroups().length > 0} fallback={<div class="presentation-empty">No presentation packs match this filter.</div>}>
                      <div class="presentation-groups">
                        <For each={presentationGroups()}>{(group, index) => {
                          const open = () => presentationExpanded().has(group.key) || (!presentationExpanded().size && !presentationQuery() && presentationFilter() === "all" && index() === 0);
                          const allLatestActive = () => group.types.every((type) => presentationActivation(type.id)?.revision === type.latest.spec.revision);
                          const groupBusy = () => presentationBusyFor(`group:${group.key}`);
                          return <section class="presentation-group" classList={{ open: open() }}>
                            <div class="presentation-group-header">
                              <button type="button" class="presentation-disclosure" aria-expanded={open()} onClick={() => togglePresentationGroup(group.key)}><Icon name="chevron" /><span><strong>{group.label}</strong><small>{group.types.length} pack{group.types.length === 1 ? "" : "s"}{group.definitionCount > group.types.length ? ` · ${group.definitionCount} versions` : ""}</small></span></button>
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
                                      <div class="presentation-type-copy"><strong>{type.label}</strong><span>{type.semanticType} · {type.latest.origin.owner === "builtin" ? "Built-in" : type.latest.origin.plugin_id ?? "Added by you"}</span></div>
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
                  </details>
                </Show>
                <Row title="Keyboard shortcuts" description="See every shortcut for navigation, tasks, and workspace tools."><button class="settings-button" onClick={() => { setSettingsOpen(false); setShowShortcuts(true); }}>View shortcuts</button></Row>
              </Group>
            </Show>

            <Show when={page() === "notifications"}>
              <header><h1>Notifications</h1><p>When Vakyartha lets you know something happened.</p></header>
              <Group>
                <Row title="Desktop notifications" description="Let you know when work finishes while Vakyartha is in the background."><Switch label="Desktop notifications" checked={uiPreferences.notifications} onChange={(value) => updateUiPreference("notifications", value)} /></Row>
                <Row title="Quiet hours" description="No finished-work or update alerts overnight. Requests for your approval still come through, because the work waits for you."><select aria-label="Quiet hours" value={uiPreferences.quietHours} onChange={(event) => updateUiPreference("quietHours", event.currentTarget.value as "off" | "22-07")}><option value="off">Off</option><option value="22-07">22:00–07:00</option></select></Row>
              </Group>
            </Show>

            <Show when={page() === "voice"}>
              <header><h1>Voice and sound</h1><p>Talk with {agentName()} and hear it answer.</p></header>
              <Group title="Voice">
                <Row title="Enable voice conversations" description="Talk with Vakyartha using your microphone."><Switch label="Enable voice conversations" checked={config()?.voice?.enabled ?? false} onChange={(value) => void updateVoice({ voice_enabled: value })} /></Row>
                <Row title="Speak updates aloud" description="Read updates and approval requests aloud."><Switch label="Speak updates aloud" checked={uiPreferences.voiceEnabled} onChange={(value) => updateUiPreference("voiceEnabled", value)} /></Row>
                <Row title="Available voice services" description="Shows which services are ready to use."><span class="settings-value">{voiceProviders.loading ? "Checking…" : (voiceProviders()?.providers.map((provider) => `${provider.name} · ${provider.readiness ? (provider.readiness.ready ? "ready" : provider.readiness.detail) : provider.configured ? "ready" : "setup needed"}`).join(", ") || "None available")}</span></Row>
                <Row title="Voice service" description="Used for listening and speaking."><select value={config()?.voice?.provider ?? ""} onChange={(e) => void updateVoice({ voice_provider: e.currentTarget.value || null })}><option value="">Inherit configured provider</option><For each={voiceProviders()?.providers ?? []}>{(provider) => <option value={provider.name}>{provider.name}</option>}</For></select></Row>
                <Show when={!voiceProviderName()}><Row title="Voice setup" description="Choose a provider to configure its key and discovered listening and speaking models."><span class="settings-value">No voice provider is selected for this Agent.</span></Row></Show>
                <Show when={voiceProviderName() && voiceProviderName() !== "local"}><Row title="Test speaking" description="Make one short request with this Agent's selected provider and speaking model."><button class="settings-button" disabled={voiceTestBusy()} onClick={() => void testVoice()}>{voiceTestBusy() ? "Testing…" : "Test voice"}</button></Row></Show>
                <Show when={voiceProviderName() && voiceProviderName() !== "local"}>
                  <Row title="Service key" description="Saved in shared credentials and available to your Agents. The key is never shown again.">
                    <span class="key-edit">
                      <input type="password" value={voiceKeyDraft()} autocomplete="new-password" placeholder={`Enter ${voiceProviderName() === "gemini" ? "Google Gemini" : "OpenAI"} key`} onInput={(event) => setVoiceKeyDraft(event.currentTarget.value)} />
                      <button class="settings-button" disabled={!voiceKeyDraft().trim() || voiceKeyBusy()} onClick={() => void saveVoiceKey()}>{voiceKeyBusy() ? "Saving…" : "Save key"}</button>
                      <Show when={voiceProviders()?.providers.find((provider) => provider.name === voiceProviderName())?.configured}>
                        <button class="settings-button danger" disabled={voiceKeyBusy()} onClick={() => setConfirmConfig({ title: `Remove the shared ${voiceProviderName() === "gemini" ? "Google Gemini" : "OpenAI"} key?`, description: "Voice for every Agent using this shared key will stop working unless another credential is available.", confirmLabel: "Remove key", isDanger: true, onConfirm: removeVoiceKey })}>Remove key</button>
                      </Show>
                    </span>
                  </Row>
                </Show>
                <Show when={voiceModels.error}>
                  <Row title="Model discovery" description="The service did not return its model list. You can enter a model ID from your provider account.">
                    <span class="settings-value">{String(voiceModels.error)}</span>
                    <button class="settings-button" onClick={() => void refreshVoiceModels()}>Retry discovery</button>
                  </Row>
                </Show>
                <Show when={uiPreferences.voiceEnabled}>
                  <Row title="Voice" description="The voice's name from your voice service; leave blank for its default."><input value={uiPreferences.voiceName} placeholder="Service default" onChange={(event) => updateUiPreference("voiceName", event.currentTarget.value.trim())} /></Row>
                  <Row title="Speaking style" description="Describe how Vakyartha should sound."><input value={uiPreferences.voicePersona} placeholder="e.g. calm and concise" onInput={(event) => updateUiPreference("voicePersona", event.currentTarget.value)} /></Row>
                </Show>
                <Show when={voiceProviderName() && voiceProviderName() !== "local"}>
                  <Row title="Listening model" description="Choose from models returned by this account. Access depends on the provider key.">
                    <Show when={voiceModelChoices("transcription").length > 0} fallback={<input type="text" value={config()?.voice?.transcription_model ?? ""} placeholder="Model ID from provider account" onChange={(e) => void updateVoice({ voice_transcription_model: e.currentTarget.value.trim() || null })} />}>
                      <select value={config()?.voice?.transcription_model ?? ""} onChange={(e) => void updateVoice({ voice_transcription_model: e.currentTarget.value || null })}><option value="">Choose discovered model</option><Show when={config()?.voice?.transcription_model && !voiceModelChoices("transcription").includes(config()!.voice!.transcription_model!)}><option value={config()!.voice!.transcription_model!}>{config()!.voice!.transcription_model} (saved; unavailable)</option></Show><For each={voiceModelChoices("transcription")}>{(model) => <option value={model}>{model}</option>}</For></select>
                    </Show>
                  </Row>
                  <Row title="Speaking model" description="Choose from text-to-speech models returned by this account.">
                    <Show when={voiceModelChoices("synthesis").length > 0} fallback={<input type="text" value={config()?.voice?.synthesis_model ?? ""} placeholder="Model ID from provider account" onChange={(e) => void updateVoice({ voice_synthesis_model: e.currentTarget.value.trim() || null })} />}>
                      <select value={config()?.voice?.synthesis_model ?? ""} onChange={(e) => void updateVoice({ voice_synthesis_model: e.currentTarget.value || null })}><option value="">Choose discovered model</option><Show when={config()?.voice?.synthesis_model && !voiceModelChoices("synthesis").includes(config()!.voice!.synthesis_model!)}><option value={config()!.voice!.synthesis_model!}>{config()!.voice!.synthesis_model} (saved; unavailable)</option></Show><For each={voiceModelChoices("synthesis")}>{(model) => <option value={model}>{model}</option>}</For></select>
                    </Show>
                  </Row>
                </Show>
                <Show when={voiceProviderName() === "local"}><Row title="Local voice engines" description="Listening and speaking use the local engines shown in Available voice services."><span class="settings-value">{voiceProviders()?.providers.find((provider) => provider.name === "local")?.readiness?.detail ?? "Checking local engines…"}</span></Row></Show>
                <TechnicalRow>
                  <Row title="Session limit" description="Longest voice conversation, in seconds."><input type="number" min="1" max="86400" value={config()?.voice?.max_session_secs ?? 900} onChange={(e) => void updateVoice({ voice_max_session_secs: Number(e.currentTarget.value) })} /></Row>
                  <Row title="Simultaneous conversations" description="How many voice conversations can run at once."><input type="number" min="1" max="64" value={config()?.voice?.max_concurrent ?? 2} onChange={(e) => void updateVoice({ voice_max_concurrent: Number(e.currentTarget.value) })} /></Row>
                  <Row title="Audio size limit" description="Largest recording accepted in one conversation, in bytes."><input type="number" min="1" max={256 * 1024 * 1024} value={config()?.voice?.max_audio_bytes ?? 16 * 1024 * 1024} onChange={(e) => void updateVoice({ voice_max_audio_bytes: Number(e.currentTarget.value) })} /></Row>
                </TechnicalRow>
              </Group>
              <Group title="Sound">
                <Row title="Sound cues" description="A short chime when work starts and when it finishes."><Switch label="Sound cues" checked={uiPreferences.soundCues} onChange={(value) => updateUiPreference("soundCues", value)} /></Row>
              </Group>
            </Show>

            <Show when={page() === "archived"}>
              <button type="button" class="settings-scope-link" onClick={() => selectPage("privacy")}>Back to Privacy and safety</button>
              <header class="archived-header"><div><h1>Archived tasks</h1><p>Hidden from the sidebar until you restore them.</p></div><button class="settings-button danger" disabled={!archivedSessions().length} onClick={() => void trashAllArchived()}><Icon name="trash" size={14} /> Move all to trash</button></header>
              <div class="settings-callout"><Icon name="archive" /><div><strong>Archive and trash are both reversible</strong><span>Archived tasks stay searchable. Moving one to the trash hides it everywhere, search included, until you restore it. Nothing is erased.</span></div></div>
              <Show when={archivedSessions().length} fallback={<div class="archived-empty"><Icon name="archive" size={24} /><strong>No archived tasks</strong><span>Tasks you archive from the sidebar will appear here.</span></div>}>
                <section class="archived-list" aria-label="Archived tasks">
                  <For each={archivedSessions()}>{(session) => <div class="archived-item"><span class="archived-item-icon"><Icon name="chat" size={15} /></span><span class="archived-item-copy"><strong>{session.title || "Untitled task"}</strong><span>{session.updated_at ? new Date(session.updated_at).toLocaleString() : ""} · {session.entries ?? 0} events</span></span><button class="settings-button" onClick={() => void restoreTask(session.session_id)}><Icon name="restore" size={13} /> Restore</button><button class="icon-button subtle danger has-tooltip" data-tooltip="Move to trash" aria-label={`Move ${session.title || "untitled task"} to the trash`} onClick={() => void trashTask(session.session_id)}><Icon name="trash" size={14} /></button></div>}</For>
                </section>
              </Show>
              <Show when={trashedSessions().length}>
                <header class="archived-header"><div><h2>Trash</h2><p>Hidden everywhere, search included, until you restore them.</p></div></header>
                <section class="archived-list" aria-label="Trash">
                  <For each={trashedSessions()}>{(session) => <div class="archived-item"><span class="archived-item-icon"><Icon name="trash" size={15} /></span><span class="archived-item-copy"><strong>{session.title || "Untitled task"}</strong><span>{session.updated_at ? new Date(session.updated_at).toLocaleString() : ""}</span></span><button class="settings-button" onClick={() => void restoreFromTrash(session.session_id)}><Icon name="restore" size={13} /> Restore</button></div>}</For>
                </section>
              </Show>
            </Show>

            <Show when={page() === "appearance"}>
              <header><h1>Appearance</h1><p>Make the workspace comfortable for long sessions.</p></header>
              <Group title="Theme">
                <div class="theme-grid"><For each={[{ id: "system", label: "Match system" }, { id: "light", label: "Light" }, { id: "dark", label: "Dark" }, { id: "contrast", label: "High contrast" }] as const}>{(theme) => <button class="theme-choice" classList={{ active: uiPreferences.theme === theme.id }} onClick={() => updateUiPreference("theme", theme.id)}><span class={`theme-preview ${theme.id}`}><i /><i /><i /></span><strong>{theme.label}</strong><Show when={uiPreferences.theme === theme.id}><Icon name="check" /></Show></button>}</For></div>
              </Group>
              <Group title="Artwork">
                <Row title="Visual pack" description="Switch the in-app Songbird, agent artwork, compact character marks, and browser icon. Pages at the same address remember this choice."><select aria-label="Visual pack" value={uiPreferences.visualPack} onChange={(event) => updateUiPreference("visualPack", event.currentTarget.value as "classic" | "dimensional")}><option value="classic">Classic</option><option value="dimensional">Dimensional 3D</option></select></Row>
              </Group>
              <Group title="Layout and text">
                <Row title="Interface font" description="Menus, settings, and panels. Fonts installed on your device are used when available."><select aria-label="Interface font" value={uiPreferences.interfaceFont} onChange={(event) => updateUiPreference("interfaceFont", event.currentTarget.value as keyof typeof interfaceFonts)}><For each={Object.entries(interfaceFonts)}>{([value, font]) => <option value={value}>{font.label}</option>}</For></select></Row>
                <Row title="Content font" description="Conversation text and result cards."><select aria-label="Content font" value={uiPreferences.contentFont} onChange={(event) => updateUiPreference("contentFont", event.currentTarget.value as keyof typeof contentFonts)}><For each={Object.entries(contentFonts)}>{([value, font]) => <option value={value}>{font.label}</option>}</For></select></Row>
                <Row title="Code font" description="Code blocks, diffs, and the editor. Uses an installed font when available."><select aria-label="Code font" value={uiPreferences.codeFont} onChange={(event) => updateUiPreference("codeFont", event.currentTarget.value as keyof typeof codeFonts)}><For each={Object.entries(codeFonts)}>{([value, font]) => <option value={value}>{font.label}</option>}</For></select></Row>
                <Row title="Text size" description="Conversation, cards, and interface text."><div class="range-control"><input type="range" aria-label="Text size" min="75" max="125" step="5" value={uiPreferences.textScale} onInput={(event) => updateUiPreference("textScale", Number(event.currentTarget.value))} /><span>{uiPreferences.textScale}%</span></div></Row>
                <Row title="Code size" description="Code blocks, diffs, editor, and terminal labels."><div class="range-control"><input type="range" aria-label="Code size" min="75" max="125" step="5" value={uiPreferences.codeScale} onInput={(event) => updateUiPreference("codeScale", Number(event.currentTarget.value))} /><span>{uiPreferences.codeScale}%</span></div></Row>
                <Row title="Compact task list" description="Fit more tasks in the sidebar with tighter rows."><Switch label="Compact task list" checked={uiPreferences.compactSidebar} onChange={(value) => updateUiPreference("compactSidebar", value)} /></Row>
                <Row title="Reduce motion" description="Disable pulsing, smooth scrolling, and animated transitions."><Switch label="Reduce motion" checked={uiPreferences.reduceMotion} onChange={(value) => updateUiPreference("reduceMotion", value)} /></Row>
                <Row title="Cards and previews" description="Show results as cards, charts and link previews."><Switch label="Cards and previews" checked={uiPreferences.richPreviews} onChange={(value) => updateUiPreference("richPreviews", value)} /></Row>
                <Row title="Images from the web" description="Show images and media from safe web addresses."><Switch label="Images from the web" checked={uiPreferences.externalMedia} onChange={(value) => updateUiPreference("externalMedia", value)} /></Row>
                <Row title="Autoplay media" description="Never enabled by default; turn on only for trusted media sources."><Switch label="Autoplay media" checked={uiPreferences.autoplayMedia} onChange={(value) => updateUiPreference("autoplayMedia", value)} /></Row>
                <Row title="Experimental skills" description="Allow sandboxed, not-yet-promoted presentation skills to render with fallback diagnostics."><Switch label="Experimental skills" checked={uiPreferences.experimentalSkills} onChange={(value) => updateUiPreference("experimentalSkills", value)} /></Row>
              </Group>
            </Show>

            <Show when={page() === "agent"}>
              <header class="agent-settings-header">
                <div class="agent-settings-title">
                  <Show when={scope() !== "user"}><AgentMark character={agentLook().character} motion={agentLook().animation} size={60} /></Show>
                  <div><h1>{scope() === "user" ? "Shared defaults" : agentName()}</h1><p>{scope() === "user" ? "Every agent starts from these unless it sets its own." : "Service and model changes apply from the next message. Work already running keeps its current choice."}</p></div>
                </div>
                <button type="button" class="settings-button" onClick={() => { setSettingsOpen(false); setAgentPickerTab("fleet"); setAgentPickerOpen(true); }}>Manage agents</button>
              </header>
              <Group title="AI service and model">
                <Show when={scope() === "user" && sharedRoute.error}><p class="connect-error" role="alert">Could not load shared settings. <button class="settings-button" onClick={() => void refreshSharedRoute()}>Try again</button></p></Show>
                <Row title="AI service" description={currentProviderInfo()?.requires_key ? "Uses your own account with this service." : "Uses the configured model server."}>
                  <span class="settings-value">{keyProviderLabel() || "Not chosen"}</span>
                </Row>
                <Row title="Model" description="Your chosen model. If it stops responding, Vakyartha can switch to a backup you allow in the admin portal: the same model at another service, or a different model.">
                  <span class="settings-value">{displayedModel() || "Not chosen"}</span>
                </Row>
                <Row title="Change service or model" description="Check an account, choose an available model, then save. Nothing changes just by opening the list.">
                  <button class="btn primary" onClick={changeService}>Change</button>
                </Row>
                <Show when={currentProviderInfo()?.requires_key}>
                  <Row title="Account key" description="Keys are stored securely where Vakyartha runs. A shared key can be used by several agents; the AI service bills your account.">
                    <span class="key-edit">
                      <span class="settings-status">{currentProviderInfo()?.configured ? "Key available · connection not tested" : "Key needed"}</span>
                      <button class="settings-button" onClick={changeService}>{currentProviderInfo()?.configured ? "Check or replace key" : "Add key"}</button>
                      <Show when={currentProviderInfo()?.key_in_user}>
                        <button class="settings-button danger" disabled={keyBusy()} onClick={() => setConfirmConfig({ title: `Remove the shared ${keyProviderLabel()} key?`, description: "Other agents using this account may stop responding. Keys supplied by the server or an individual workspace remain available.", confirmLabel: "Remove shared key", isDanger: true, onConfirm: removeKey })}>Remove shared key</button>
                      </Show>
                    </span>
                  </Row>
                </Show>
                <Row title="Advanced AI settings" description="Manage exact model IDs, account-key scopes, connection checks and routing in the admin portal.">
                  <button class="settings-button" onClick={() => host.openAdmin("#/settings/models")}>Open admin portal</button>
                </Row>
                <TechnicalRow>
                  <Row title="Maximum turns" description="Hard limit for one task before the agent stops."><input class="settings-number" type="number" min="1" max="1000" value={maxTurns()} onInput={(event) => setMaxTurns(Number(event.currentTarget.value))} /></Row>
                  <Row title="Check freshness" description="How long a successful check still counts as current."><input class="settings-number" type="number" min="0" max="8760" value={evidenceAgeHours()} onInput={(event) => setEvidenceAgeHours(Number(event.currentTarget.value) || 0)} /><span class="settings-status">hours</span></Row>
                  <Row title="Helpers" description="Vakyartha can split big jobs across helpers that work in parallel."><span class="settings-status good">{config()?.workers ? "On" : "Off in configuration"}</span></Row>
                  <Row title="Context size" description="Most the model reads at once before older turns are summarised."><span class="metric">{limitText(config()?.context_window, config()?.context_window_source)}</span></Row>
                  <Row title="Maximum output" description="Most the model writes in one reply."><span class="metric">{limitText(config()?.max_tokens, config()?.max_tokens_source)}</span></Row>
                  <Row title="Where these come from" description={`AI service: ${config()?.provider_source ?? "unknown"} · model: ${config()?.model_source ?? "unknown"}`}><span /></Row>
                  <Show when={currentProviderInfo()?.requires_key}>
                    <Row title="Key storage" description={`${currentProviderInfo()?.env_var ?? "API key"} · ${currentProviderInfo()?.pool_size ?? 0} in the routing pool · kept in the Vakyartha host's secure credential store (the OS keychain, or an encrypted file when there is none). A real environment variable takes precedence.`}><span /></Row>
                  </Show>
                </TechnicalRow>
              </Group>
              <Show when={technicalDetails()}><div class="settings-actions">
                <button class="btn primary" disabled={saving() || !agentDirty()} onClick={() => void applyAgent()}>{saving() ? "Applying…" : "Apply changes"}</button>
                <Show when={technicalDetails()}>
                  <button class="settings-button" onClick={() => void openWorkspaceConfig()}>Edit configuration file</button>
                </Show>
                <Show when={agentDirty()} fallback={<span class="settings-status good">Saved</span>}>
                  <span class="settings-status warn">Unsaved changes — press Apply</span>
                </Show>
              </div></Show>
            </Show>

            <Show when={page() === "privacy"}>
              <header><h1>{scope() === "user" ? "Shared privacy and safety" : `${agentName()} · Privacy and safety`}</h1><p>{scope() === "user" ? "Defaults for every agent. An agent can set its own choices from its settings." : `These controls are for ${agentName()} only. Unset choices inherit the shared defaults.`}</p></header>
              <Show when={host.authenticate}>
                <Group title="Your sign-in">
                  <Row title="Recovery codes" description="Create a new set after confirming with your passkey. The previous codes will stop working.">
                    <button class="settings-button" disabled={recoveryBusy()} onClick={() => void rotateRecovery()}>{recoveryBusy() ? "Checking passkey…" : "Generate new codes"}</button>
                  </Row>
                  <Row title="This browser" description="End this browser session. You can sign in again with your passkey or a recovery code.">
                    <button class="settings-button" onClick={() => void host.logout?.().then(() => window.location.reload()).catch((error) => setNotice({ kind: "error", text: `Could not sign out: ${error instanceof Error ? error.message : String(error)}` }))}>Sign out</button>
                  </Row>
                </Group>
              </Show>
              <section class="settings-group"><h3>What it may do</h3><p class="settings-group-copy">{scope() === "user" ? (privacyLayer()?.permission_mode ? "Editing the shared permission default." : "No shared permission default is set; built-in defaults apply.") : privacyLayer()?.permission_mode ? `This choice is set for ${agentName()}.` : `${agentName()} inherits the shared permission default.`}</p>
              <div class="permission-options"><For each={[{ id: "ReadOnly", title: "Look only", text: "Read and search this folder. Makes no changes.", icon: "preview" as IconName }, { id: "WorkspaceWrite", title: "Edit files in this folder", text: "Asks before anything sensitive.", icon: "pencil" as IconName }, { id: "FullAccess", title: "Full access to this computer", text: "Runs any command and opens files outside this folder.", icon: "warning" as IconName }] as const}>{(mode) => <button classList={{ active: privacyPermissionMode() === mode.id, danger: mode.id === "FullAccess" }} onClick={() => void changePermission(mode.id)}><span class="permission-icon"><Icon name={mode.icon} /></span><span><strong>{mode.title}</strong><small>{mode.text}</small></span><span class="permission-check"><Show when={privacyPermissionMode() === mode.id}><Icon name="check" /></Show></span></button>}</For></div>
              </section>
              <section class="settings-group">
                <h3>Memory for {scope() === "user" ? "every agent" : agentName()}</h3>
                <p class="settings-group-copy">{scope() === "user" ? "Choose what every agent can remember and learn by default. An agent can set its own choices." : "Choose what this agent can remember and learn. Unset choices inherit Shared."}</p>
                <Row title="Recall saved notes" description="Let the agent search its saved memory while responding."><Switch label="Recall saved notes" checked={privacyMemory().search_enabled} onChange={(value) => void changeMemoryPolicy("memory_search_enabled", value)} /></Row>
                <Row title="Save useful details" description="Allow the agent to add useful facts to its own memory."><Switch label="Save useful details" checked={privacyMemory().write_enabled} onChange={(value) => void changeMemoryPolicy("memory_write_enabled", value)} /></Row>
                <Row title="Learn from conversations" description="Reflect on completed conversations to find useful details to remember."><Switch label="Learn from conversations" checked={privacyMemory().reflection} onChange={(value) => void changeMemoryPolicy("memory_reflection", value)} /></Row>
                <Row title="Suggest new skills" description="Allow memory to propose reusable skills for review."><Switch label="Suggest new skills" checked={privacyMemory().skill_proposals} onChange={(value) => void changeMemoryPolicy("memory_skill_proposals", value)} /></Row>
              </section>
              <section class="settings-group"><h3>When to ask</h3>
                <p class="settings-group-copy">{scope() === "user" ? (privacyLayer()?.approval_mode ? "Editing the shared approval default." : "No shared approval default is set; built-in defaults apply.") : privacyLayer()?.approval_mode ? `This choice is set for ${agentName()}.` : `${agentName()} inherits the shared approval default.`}</p>
                <div class="permission-options">
                  <For
                    each={
                      [
                        { id: "ask", title: "Every time", text: "Pause for anything that needs approval.", icon: "shield" as IconName },
                        { id: "approve-safe", title: "Only outside this folder", text: "Reading and editing here continue. Web and outside access still ask.", icon: "check" as IconName },
                        { id: "auto-approve", title: "Don't ask", text: "Vakyartha keeps going unless a rule requires your approval.", icon: "warning" as IconName },
                      ] as const
                    }
                  >
                    {(mode) => (
                      <button
                        classList={{ active: privacyApprovalMode() === mode.id }}
                        onClick={() => void changeApproval(mode.id)}
                      >
                        <span class="permission-icon"><Icon name={mode.icon} /></span>
                        <span><strong>{mode.title}</strong><small>{mode.text}</small></span>
                        <span class="permission-check">
                          <Show when={privacyApprovalMode() === mode.id}><Icon name="check" /></Show>
                        </span>
                      </button>
                    )}
                  </For>
                </div>
              </section>
              <Show
                when={technicalDetails()}
                fallback={
                  <Group title="Your rules">
                    <Row title="Custom rules" description="Rules you set win over the choice above.">
                      <span class="settings-value">{(() => { const n = (["deny", "ask", "allow"] as const).reduce((count, key) => count + (privacyRules()[key]?.length ?? 0), 0); return n ? `${n} rule${n === 1 ? "" : "s"}` : "None"; })()}</span>
                    </Row>
                  </Group>
                }
              >
                <Show when={scope() === "workspace"}>
                  <Group title="Isolation">
                    <Row title="Workspace files" description="File access stays inside this agent's folder."><span class="settings-status good">Protected</span></Row>
                    <Row title="Command isolation" description="Keep commands separated from the rest of this device."><span class="settings-status good">{config()?.sandbox ?? "…"}</span></Row>
                  </Group>
                </Show>
                <Group title="Rules">
                  <p class="settings-group-copy">Specific rules take priority over the approval choice above.</p>
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
                        (privacyRules()[section.key]?.length ?? 0) > 0
                          ? privacyRules()[section.key].join(", ")
                          : section.empty
                      }>
                        <span />
                      </Row>
                    )}
                  </For>
                  <Show when={scope() === "workspace"}><Row title="Permission rules" description="Choose which actions are allowed, blocked, or require approval."><button class="settings-button" onClick={() => void openWorkspaceConfig()}>Edit rules</button></Row></Show>
                </Group>
              </Show>
              <h2 class="settings-section-title">What it remembers</h2>
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
                            <span class="memory-recent-scope">every agent</span>
                          </Show>
                          <span class="memory-recent-time">{relTime(n.ts)}</span>
                        </button>
                      )}
                    </For>
                  </div>
                </section>
              </Show>
              <nav class="capability-tabs memory-tabs" aria-label="Memory tiers">
                <button classList={{ active: tier() === "workspace" }} onClick={() => setTier("workspace")}><Icon name="folder" /><span>This agent</span><em>{notes().filter((n) => (n.scope ?? "workspace") === "workspace").length}</em></button>
                <button classList={{ active: tier() === "profile" }} onClick={() => setTier("profile")}><Icon name="spark" /><span>Every agent</span><em>{notes().filter((n) => n.scope === "profile").length}</em></button>
              </nav>
              <Group title={tier() === "profile" ? "Remembered for every agent" : `Remembered by ${agentName()}`}>
                <Show when={tier() === "profile"}>
                  <p class="settings-hint memory-hint">Every agent can recall these notes.</p>
                </Show>
                <Show
                  when={!addingNote()}
                  fallback={
                    <div class="memory-add">
                      <div class="memory-add-fields">
                        <label>Kind<input value={noteKind()} aria-label="Note kind" onInput={(e) => setNoteKind(e.currentTarget.value)} /></label>
                        <label>Tag <span class="label-hint">optional</span><input value={noteTag()} aria-label="Note tag" onInput={(e) => setNoteTag(e.currentTarget.value)} /></label>
                      </div>
                      <textarea rows={2} placeholder={tier() === "profile" ? "Something that should hold across every agent…" : "Something that should hold for this agent…"} aria-label="Note text" value={noteText()} onInput={(e) => setNoteText(e.currentTarget.value)} />
                      <div class="task-add-row">
                        <button class="btn primary" disabled={!noteText().trim() || !noteKind().trim()} onClick={() => void addMemoryNote()}>Save note</button>
                        <button class="btn" onClick={() => setAddingNote(false)}>Cancel</button>
                      </div>
                    </div>
                  }
                >
                  <div class="task-add-row"><button class="btn" onClick={() => setAddingNote(true)}><Icon name="add" /> Add a note</button></div>
                </Show>
                <Show
                  when={tierNotes().length > 0}
                  fallback={<Row title="Nothing remembered yet" description={tier() === "profile" ? "Add a preference once and every agent can use it." : "Important decisions and preferences will appear here."}><span class="settings-status good">Ready</span></Row>}
                >
                  <div class="archived-list" aria-label="Memory notes">
                    <For each={tierNotes().slice().reverse()}>
                      {(n) => (
                        <div class="task-row memory-note" data-note-id={n.id} classList={{ picked: recentPickedId() === n.id }}>
                          <div class="task-main">
                            <div class="task-name">
                              <Show when={technicalDetails()}><span class="badge memory-id" title={`Note id ${n.id}`}>{n.id.slice(0, 8)}</span></Show>
                              {n.tag || n.kind}
                              <span class="badge">{n.kind}</span>
                              <Show when={n.tag && n.tag !== n.kind}><span class="badge">tag: {n.tag}</span></Show>
                            </div>
                            <small class="memory-meta">{new Date(n.ts).toLocaleString()}<Show when={technicalDetails()}> · from {n.session_id.slice(0, 8)}</Show></small>
                            <Show
                              when={editingId() === n.id}
                              fallback={<p class="memory-text">{n.text}</p>}
                            >
                              <textarea
                                class="memory-amend"
                                rows={3}
                                aria-label="Edit note"
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
                              <button class="chip sm" onClick={() => { setEditingId(n.id); setEditText(n.text); }}>Edit</button>
                            </Show>
                            <button class="chip sm danger-chip" onClick={() => void forgetNote(n.id)}>Forget</button>
                          </div>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </Group>
              <Group title="History">
                <Row title="Archived tasks" description="Hidden from the sidebar until you restore them."><button type="button" class="settings-button" onClick={() => selectPage("archived")}>{archivedSessions().length ? `Open (${archivedSessions().length})` : "Open"}</button></Row>
              </Group>
            </Show>

            <Show when={page() === "reliability"}>
              <header><h1>Reliability</h1><p>Understand how Vakyartha recovers from provider and task failures.</p></header>
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
              <button class="settings-button" onClick={() => void openWorkspaceConfig()}>Tune in workspace config</button>
            </Show>

            <Show when={page() === "models"}>
              <header><h1>Models and routing</h1><p>Which models take over when the chosen one fails.</p></header>
              <Group title="Route ladder">
                <Row title="Objective" description={`How fallback legs are ordered: ${config()?.route.objective === "auto" ? "derived from request demand (utility / balanced / quality-critical)." : `fixed to ${config()?.route.objective}.`}`}><span class="metric">{config()?.route.objective}</span></Row>
                <Row
                  title="Same model at other services"
                  description={config()?.route.same_model.length
                    ? `Confirmed, tried first: ${config()!.route.same_model.map((group) => group.join(" = ")).join("; ")}.`
                    : "None confirmed. An equal name at another service is never taken to be the same model; confirm matches in the admin portal."}
                >
                  <span class="metric">{config()?.route.same_model.length ?? 0}</span>
                </Row>
                <Row
                  title="Cross-model fallbacks"
                  description={config()?.route.fallback_models.length
                    ? `Allowed models, admitted only when discovery reaches them: ${config()!.route.fallback_models.join(", ")}.`
                    : "None allowed. Allow backup models in the admin portal."}
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

            <Show when={page() === "mail-calendar"}>
              <header class="mail-calendar-settings-header"><div class="mail-calendar-settings-intro"><h1>Connected accounts and access</h1><p>Manage the accounts and permissions available to {agentName()}.</p><p class="settings-hint">Mail, calendar, drafts, Review and automation setup are in Canvas. This page manages account access and lifecycle.</p></div><button type="button" class="settings-button" onClick={() => { setSettingsOpen(false); openArtifactCanvas({ kind: "daily_mail_calendar", title: "Today", agentId: activeAgentId() }); }}>Open Today in Canvas</button><p class="settings-hint">Connect an account for {agentName()}. Each connection belongs to this Agent and only grants the access you select.</p>
                <p class="settings-hint" role="status">
                  <Show when={connection() === "offline"} fallback={connection() === "live" && health()
                    ? health()?.automation_scheduler?.status === "stale"
                      ? "The service API answers, but its background scheduler has not checked in recently. The service may be starting, paused, stalled, or the computer may have slept; scheduled work has not been confirmed during this period."
                      : health()?.automation_scheduler?.status === "starting"
                        ? "The service API answers; its background scheduler is starting. Automation status and provider-check times will update when the scheduler begins reporting."
                        : "Scheduled and continuous automations run on this Vakyartha service. The computer or server running it must stay awake and connected; each automation's last successful provider check shows source freshness."
                    : "Connecting to the Vakyartha service. Automation status and source freshness will appear when it is reachable."}>
                    The Vakyartha service is offline, so its scheduled and continuous routines cannot run. Missed work follows the task schedule and configured catch-up behavior.
                  </Show>
                </p>
              </header>
              <Show when={canOfferSyntheticMailCalendar()}>
                <Group title="Safe practice mode">
                  <SyntheticMailCalendarDemoControl checked={syntheticMailDemo()} onChange={(enabled) => { setSyntheticMailCalendarEnabled(enabled); setSyntheticMailDemo(enabled); }} />
                </Group>
              </Show>
              <div class="settings-callout"><Icon name="shield" /><div><strong>Google and Microsoft support email and calendar reads; verified Apple accounts support bounded Mail and Calendar previews.</strong><span>Read results sent to the Agent become part of append-only conversation history and may remain after disconnect or account deletion. Current storage cannot erase those copies. Channel conversations are blocked unless separately shared. Owner previews open in Canvas and do not add provider content to conversation history. Disconnect removes saved sign-in details and blocks future reads; it does not delete provider messages or events. Scheduled automations are available; dependable continuous service recovery is still in progress.</span></div></div>
              <Show when={!syntheticMailDemo()}>
              <Group title="Choose access">
                <p class="settings-hint">Read access is selected by default. Email sending and calendar changes are optional and request separate provider permissions. Every effect requires exact review and your confirmation. Event creation supports one timed event without attendees, invitations, recurrence, or reminders. Google event updates and cancellations are limited to one unchanged, public, standalone timed event with no attendees when you are its organizer; cancellation applies to that event only. Provider calendar-write consent is broader than these actions; Vakyartha exposes only the reviewed operations.</p>
                <div class="capability-list">
                  {([["mail_read", "Read email"], ["mail_send", "Send email after review"], ["calendar_free_busy", "Check availability"], ["calendar_read", "Read calendar events"], ["calendar_write", "Create or change calendar events after review"]] as const).map(([capability, label]) => <label class="capability-item"><input type="checkbox" checked={mailCalendarCapabilities().includes(capability)} onChange={(event) => setMailCalendarCapabilities((current) => event.currentTarget.checked ? [...new Set([...current, capability])] : current.filter((item) => item !== capability))} /><span>{label}</span></label>)}
                </div>
              </Group>
              <Group title="Connect an account">
                <Row title="Google · OAuth (recommended)" description="Sign in with Google using a local PKCE flow. Only the access you select is requested. If setup is missing, configure VAK_GOOGLE_OAUTH_CLIENT_ID on this host with a Desktop OAuth client."><button class="settings-button" disabled={mailCalendarBusy()} onClick={() => void connectMailCalendar("google")}>Connect with Google</button></Row>
                <Show when={canAddLocalAppPassword()} fallback={<p class="settings-hint">Google App Password setup is available only when Vakyartha and this browser run on the same device. OAuth remains the recommended method.</p>}>
                  <Row title="Google Gmail · App Password" description="Optional older sign-in for MailRead only. It cannot access Calendar or send email." className="mail-calendar-credential-row">
                    <p class="settings-hint">Warning: an App Password is a long-lived account credential and is less secure than OAuth. Use a separate password for Vakyartha, then revoke it in your Google Account security settings when you disconnect. Never enter your regular Google password.</p>
                    <div class="settings-actions mail-calendar-credential-fields"><input type="email" aria-label="Google account email" autocomplete="username" value={googleAppEmail()} onInput={(event) => setGoogleAppEmail(event.currentTarget.value)} placeholder="name@gmail.com" /><input type="password" aria-label="Google App Password" autocomplete="new-password" value={googleAppPassword()} onInput={(event) => setGoogleAppPassword(event.currentTarget.value)} placeholder="Google App Password" /><button class="settings-button" disabled={mailCalendarBusy() || !googleAppEmail() || !googleAppPassword()} onClick={() => void connectGoogleAppPassword()}>Connect Gmail</button></div>
                  </Row>
                </Show>
                <Row title="Microsoft · OAuth (recommended)" description="Outlook email and calendar through local delegated OAuth with PKCE. Configure VAK_MICROSOFT_OAUTH_CLIENT_ID with an Entra public client. No client secret is used."><button class="settings-button" disabled={mailCalendarBusy()} onClick={() => void connectMailCalendar("microsoft")}>Connect with Microsoft</button></Row>
                <p class="settings-hint">Exchange Online and Microsoft 365 accounts require OAuth. Never enter your regular Microsoft password here.</p>
                <Show when={canAddLocalAppPassword()} fallback={<p class="settings-hint">Outlook.com app-password setup is available only when Vakyartha and this browser run on the same device.</p>}>
                  <Row title="Outlook.com · App password (email only)" description="For personal Outlook.com, Live, Hotmail, or MSN accounts only. Vakyartha verifies the fixed-host IMAP sign-in before saving it." className="mail-calendar-credential-row">
                    <p class="settings-hint">Security warning: this is a long-lived credential using Microsoft's legacy IMAP sign-in and is less secure than OAuth. Microsoft may reject it or disable this path. It grants email reading only; no calendar, send, or provider changes. Create a unique app password in Microsoft Account security settings, then revoke it there when disconnected. Do not use your regular Microsoft password or a work/school Exchange password.</p>
                    <div class="settings-actions mail-calendar-credential-fields"><input type="email" aria-label="Microsoft account email" autocomplete="username" value={microsoftAppEmail()} onInput={(event) => setMicrosoftAppEmail(event.currentTarget.value)} placeholder="name@outlook.com" /><input type="password" aria-label="Microsoft app password" autocomplete="new-password" value={microsoftAppPassword()} onInput={(event) => setMicrosoftAppPassword(event.currentTarget.value)} placeholder="Microsoft app password" /><button class="settings-button" disabled={mailCalendarBusy() || !microsoftAppEmail() || !microsoftAppPassword()} onClick={() => void connectMicrosoftAppPassword()}>Connect Outlook.com</button></div>
                  </Row>
                </Show>
                <Show when={canAddLocalAppPassword()} fallback={<p class="settings-hint">For security, add iCloud app-specific passwords only from Vakyartha running on this device. The credential form is unavailable on hosted servers.</p>}>
                  <Row title="Apple iCloud · App-specific password" description="This build uses a password generated at account.apple.com. Apple also documents account authorization for supported third-party apps, but Vakyartha has no verified integration for it yet." className="mail-calendar-credential-row">
                    <p class="settings-hint">Security warning: this provider password can grant broader iCloud access than the read capabilities selected here. It is stored in this Agent's local credential vault, but Apple controls its scope. Use a unique app-specific password, select only the access you need, and revoke it at account.apple.com when you disconnect. Vakyartha verifies each selected protocol before saving the link; if any check fails, no active account is created. Never enter your Apple Account password.</p>
                    <div class="settings-actions mail-calendar-credential-fields"><input type="email" aria-label="iCloud account email" autocomplete="username" value={icloudEmail()} onInput={(event) => setIcloudEmail(event.currentTarget.value)} placeholder="name@icloud.com" /><input type="password" aria-label="iCloud app-specific password" autocomplete="new-password" value={icloudAppPassword()} onInput={(event) => setIcloudAppPassword(event.currentTarget.value)} placeholder="App-specific password" /><button class="settings-button" disabled={mailCalendarBusy() || !icloudEmail() || !icloudAppPassword() || mailCalendarCapabilities().some((capability) => !["mail_read", "calendar_free_busy", "calendar_read"].includes(capability))} onClick={() => void connectIcloud()}>Connect iCloud</button></div>
                  </Row>
                  <p class="settings-hint">Apple's app-specific password can authorize more than the selected access. “Read email” provides bounded inbox metadata and separately selected plain-text message reads; “Check availability” returns busy intervals only; “Read calendar events” provides a bounded calendar preview. You may select more than one read capability; Vakyartha verifies IMAP and CalDAV separately and enables only the checked capabilities. Provider changes remain unavailable. Remove the password at Apple to revoke it.</p>
                </Show>
                <p class="settings-hint">Google and Microsoft sign-in currently requires Vakyartha and your browser on the same device. The callback uses a loopback address; hosted or public-server callbacks are not enabled.</p>
              </Group>
              </Show>
              <Group title={`Accounts for ${agentName()}`}>
                <Show when={!mailCalendarAccounts.loading} fallback={<div class="settings-hint">Loading connected accounts…</div>}>
                  <Show when={(mailCalendarAccounts()?.accounts.length ?? 0) > 0} fallback={<p class="settings-hint">No accounts are connected to this Agent.</p>}>
                    <For each={mailCalendarAccounts()?.accounts ?? []}>{(account) => {
                      const label = account.provider === "google" ? "Google account" : account.provider === "microsoft" ? "Microsoft account" : "Apple iCloud account";
                      const needsNewOAuthLink = account.status === "reauthentication_required"
                        && account.credential_available
                        && !account.superseded_by_active_link;
                      const connectionState = account.revoked_at
                        ? "Disconnected"
                          : account.status === "pending"
                            ? "Connection incomplete · cleanup needed"
                            : !account.credential_available
                              ? "Saved sign-in details are unavailable · disconnect this entry, then connect again"
                              : account.status === "connected_unverified"
                                ? "Credential saved · not verified or available to Agents"
                              : account.status === "reauthentication_required"
                            ? account.superseded_by_active_link
                                ? "Reconnected · remove this old entry"
                                : "New sign-in required · connect again, then remove this entry"
                              : account.status === "connected" && !account.refresh_token_available
                                ? "Sign-in cannot be renewed · disconnect this entry, then connect again"
                              : "Connected";
                      const canPreview = account.status === "connected" && account.credential_available && !account.revoked_at;
                      const enabledAccountRoutines = () => (mailCalendarTasks() ?? []).filter(
                        (task) => task.enabled && task.scope?.account_id === account.id,
                      );
                      const pauseAccountKey = `account:${account.id}`;
                      return <Row title={`${label}${account.identity_masked ? ` · ${account.identity_masked}` : ""}`} description={`${syntheticMailDemo() ? "Synthetic sample · no provider connected" : connectionState} · Access: ${describeMailCalendarCapabilities(account.capabilities)} · ${account.auth_method === "app_password" ? "App Password · revoke at provider" : account.refresh_token_available ? "Sign-in can be renewed" : "Sign-in may need renewal"}`}>
                        <div class="settings-actions">
                          <button class="settings-button" onClick={() => { setSettingsOpen(false); openArtifactCanvas({ kind: "daily_mail_calendar", title: "Today", agentId: activeAgentId() }); }}>Open in Canvas</button>
                          <Show when={!syntheticMailDemo()}>
                            <Show when={enabledAccountRoutines().length > 0}><button class="settings-button" disabled={mailCalendarPausing() !== null} onClick={() => void pauseMailCalendarRoutines(account.id)}>{mailCalendarPausing() === pauseAccountKey ? "Pausing…" : "Pause automations for this account"}</button></Show>
                            <Show when={!account.revoked_at && account.status === "connected" && account.credential_available && account.refresh_token_available}><button class="settings-button" disabled={mailCalendarBusy()} onClick={() => void refreshMailCalendarAccount(account)}>Refresh sign-in</button></Show>
                            <Show when={!account.revoked_at && account.provider !== "apple_icloud" && needsNewOAuthLink}><button class="settings-button" disabled={mailCalendarBusy()} onClick={() => void connectMailCalendar(account.provider)}>Connect again</button></Show>
                            <button class="settings-button danger" disabled={mailCalendarBusy()} onClick={() => disconnectMailCalendar(account)}>{account.status === "pending" ? "Clean up connection" : account.revoked_at ? "Finish cleanup" : "Disconnect"}</button>
                          </Show>
                        </div>
                      </Row>;
                    }}</For>
                  </Show>
                </Show>
              </Group>
            </Show>

            <Show when={page() === "social"}>
              <header><h1>Social accounts</h1><p>{scope() === "user" ? "Set up social platforms once for every agent. An agent can still use its own credential instead." : `Connect the social platforms ${agentName()} may use. A credential saved under Shared applies here unless this agent has its own.`}</p></header>
              <For each={[{ title: "Platforms", note: "Owner-only previews. Results stay on this screen and are never sent to the agent or saved.", ids: ["social-youtube", "social-reddit", "social-x", "social-linkedin"] }]}>{(groupDef) => <section class="social-group">
                <div class="social-group-head"><h2>{groupDef.title}</h2><small>{groupDef.note}</small></div>
                <div class="social-grid"><For each={groupDef.ids.map((id) => socialConnectors().find((connector) => connector.id === id)).filter((connector): connector is api.SocialConnector => !!connector)}>{(connector) => {
                  const plugin = () => [...plugins(), ...inheritedPlugins()].find((item) => item.name === connector.id);
                  const isYoutube = connector.id === "social-youtube";
                  const isLinkedin = connector.id === "social-linkedin";
                  const isReddit = connector.id === "social-reddit";
                  const isX = connector.id === "social-x";
                  const credentialSet = () => isYoutube ? youtubeKeyConfigured() : isLinkedin ? linkedinClientIdConfigured() : isReddit ? redditConfigured() : xConfigured();
                  const inherited = () => isYoutube ? youtubeInherited() : isLinkedin ? linkedinInherited() : isReddit ? redditInherited() : xInherited();
                  const status = () => !plugin() ? { text: "Not installed", cls: "muted" }
                    : !plugin()!.enabled ? { text: "Disabled", cls: "muted" }
                    : isLinkedin && linkedinAccount().connected ? { text: "Connected", cls: "ready" }
                    : credentialSet() || inherited() ? { text: "Ready", cls: "ready" }
                    : { text: "Needs credential", cls: "warn" };
                  return <article class="social-card">
                    <div class="social-card-head"><SocialIcon platform={connector.id} /><div><strong>{connector.platform}</strong><small>{connector.summary}</small></div><span class="capability-state" classList={{ ready: status().cls === "ready", muted: status().cls === "muted" }}>{status().text}</span></div>
                    <Show when={connector.readiness === "blocked"}><p class="settings-hint">{connector.reason}</p></Show>
                    <ol class="social-steps">
                      <li><div class="social-step-title"><span class="social-step-num">1</span><strong>Add-on</strong></div>
                        <div class="settings-actions"><Show when={plugin()} fallback={<button type="button" class="settings-button" disabled={pluginBusy()} onClick={() => void installSocialGuidance(connector.id, connector.platform)}>Install</button>}><span class="settings-hint">{plugin()!.enabled ? "Enabled" : "Installed, disabled"}</span><button type="button" class="settings-button" disabled={pluginBusy()} onClick={() => void toggleSocialGuidance(plugin()!)}>{plugin()!.enabled ? "Disable" : "Enable"}</button><button type="button" class="settings-button subtle" disabled={pluginBusy()} onClick={() => void installSocialPresentationPack(connector.id)}>Add layouts</button></Show></div>
                        <Show when={plugin() && connector.guidance_digest && plugin()!.digest !== connector.guidance_digest}><p class="settings-hint">A newer version of this add-on is built in. Updating turns it off until you turn it on again, and you can roll back under Capabilities.</p><button type="button" class="settings-button" disabled={pluginBusy()} onClick={() => void installSocialGuidance(connector.id, connector.platform, plugin()!.scope, true)}>Update add-on</button></Show></li>
                      <Show when={isYoutube && plugin()?.enabled}><li><div class="social-step-title"><span class="social-step-num">2</span><strong>API key</strong><Show when={inherited()}><span class="social-badge">Using the Shared key</span></Show></div>
                        <p class="settings-hint">A Google Cloud YouTube Data API key, separate from your Google account. Each search uses your project's quota.</p>
                        <a href="https://console.cloud.google.com/apis/credentials" target="_blank" rel="noreferrer noopener">Create or manage a key</a>
                        <Show when={!youtubeKeyConfigured()}>
                          <label>API key<input type="password" autocomplete="new-password" value={youtubeKeyInput()} onInput={(event) => setYoutubeKeyInput(event.currentTarget.value)} placeholder={inherited() ? "Enter a key to override the Shared one" : "Paste key; it will not be shown again"} /></label>
                          <div class="settings-actions"><button type="button" class="settings-button" disabled={youtubeBusy() || !youtubeKeyInput().trim()} onClick={async () => { setYoutubeBusy(true); try { await api.saveYoutubeKey(youtubeKeyInput(), activeAgentId(), credScope()); setYoutubeKeyInput(""); setYoutubeKeyConfigured(true); setYoutubeInherited(false); setNotice({ kind: "info", text: scope() === "user" ? "YouTube API key saved for every agent." : "YouTube API key saved for this agent." }); } catch (error) { setNotice({ kind: "error", text: `Could not save YouTube API key: ${(error as Error).message}` }); } finally { setYoutubeBusy(false); } }}>Save key</button></div>
                        </Show>
                        <Show when={youtubeKeyConfigured()}><div class="settings-actions"><span class="capability-state ready">{scope() === "user" ? "Shared key saved" : `Key saved for ${agentName()}`}</span><button type="button" class="settings-button danger" disabled={youtubeBusy()} onClick={async () => { try { await api.removeYoutubeKey(activeAgentId(), credScope()); setYoutubeKeyConfigured(false); setYoutubeResults([]); await refreshCapabilities(); setNotice({ kind: "info", text: "YouTube API key removed." }); } catch (error) { setNotice({ kind: "error", text: `Could not remove YouTube API key: ${(error as Error).message}` }); } }}>Remove key</button></div></Show>
                      </li>
                      <Show when={scope() !== "user" && (youtubeKeyConfigured() || inherited())}><li><div class="social-step-title"><span class="social-step-num">3</span><strong>Try a search</strong></div>
                        <div class="settings-actions"><input aria-label="YouTube search query" value={youtubeQuery()} onInput={(event) => setYoutubeQuery(event.currentTarget.value)} maxlength={200} placeholder="Search public YouTube videos" /><button type="button" class="settings-button" disabled={youtubeBusy() || !youtubeQuery().trim()} onClick={async () => { setYoutubeBusy(true); try { const result = await api.searchYoutubePreview(youtubeQuery(), 5, activeAgentId()); setYoutubeResults(result.items); } catch (error) { setNotice({ kind: "error", text: `YouTube search failed: ${(error as Error).message}` }); } finally { setYoutubeBusy(false); } }}>{youtubeBusy() ? "Searching…" : "Search"}</button></div>
                        <div class="capability-list"><For each={youtubeResults()}>{(item) => <article class="capability-item"><a href={item.url} target="_blank" rel="noreferrer noopener">{item.title}</a><small>{item.channel_title} · {item.published_at}</small><p>{item.description}</p></article>}</For></div>
                      </li></Show></Show>
                      <Show when={isReddit && plugin()?.enabled}><li><div class="social-step-title"><span class="social-step-num">2</span><strong>App Client ID</strong><Show when={inherited()}><span class="social-badge">Using the Shared Client ID</span></Show></div>
                        <p class="settings-hint">Create an app at Reddit’s app page and choose <strong>installed app</strong>. Any redirect address works, for example <code>http://localhost</code>. Copy the ID shown under the app name. No client secret is needed or accepted.</p>
                        <a href="https://www.reddit.com/prefs/apps" target="_blank" rel="noreferrer noopener">Open Reddit app preferences</a>
                        <label>Client ID<input aria-label="Reddit app Client ID" autocomplete="off" value={redditInput()} onInput={(event) => setRedditInput(event.currentTarget.value)} maxlength={128} placeholder={redditConfigured() ? "Enter a new Client ID to replace the saved one" : inherited() ? "Enter an ID to override the Shared one" : "Paste your app's Client ID"} /></label>
                        <div class="settings-actions">
                          <button type="button" class="settings-button" disabled={redditBusy() || !redditInput().trim()} onClick={async () => { setRedditBusy(true); try { await api.saveRedditClientId(redditInput(), activeAgentId(), credScope()); setRedditInput(""); await refreshCapabilities(); setNotice({ kind: "info", text: scope() === "user" ? "Reddit Client ID saved for every agent." : "Reddit Client ID saved for this agent." }); } catch (error) { setNotice({ kind: "error", text: `Could not save Reddit Client ID: ${(error as Error).message}` }); } finally { setRedditBusy(false); } }}>{redditConfigured() ? "Update Client ID" : "Save Client ID"}</button>
                          <Show when={redditConfigured()}><button type="button" class="settings-button subtle" disabled={redditBusy()} onClick={async () => { try { await api.removeRedditClientId(activeAgentId(), credScope()); setRedditResults([]); await refreshCapabilities(); setNotice({ kind: "info", text: "Reddit Client ID removed." }); } catch (error) { setNotice({ kind: "error", text: `Could not remove Client ID: ${(error as Error).message}` }); } }}>Remove</button></Show>
                        </div>
                        <p class="settings-hint">Reddit’s terms apply, including its rule to delete content that was deleted on Reddit. Results appear here only: they are not saved or sent to the agent. Commercial use may need a separate Reddit agreement.</p>
                      </li>
                      <Show when={scope() !== "user" && (redditConfigured() || inherited())}><li><div class="social-step-title"><span class="social-step-num">3</span><strong>Try a search</strong></div>
                        <div class="settings-actions"><input aria-label="Reddit search query" value={redditQuery()} onInput={(event) => setRedditQuery(event.currentTarget.value)} maxlength={200} placeholder="Search public Reddit posts" /><button type="button" class="settings-button" disabled={redditBusy() || !redditQuery().trim()} onClick={async () => { setRedditBusy(true); try { const result = await api.searchRedditPreview(redditQuery(), 5, activeAgentId()); setRedditResults(result.items); if (result.items.length === 0) setNotice({ kind: "info", text: "No Reddit posts matched." }); } catch (error) { setNotice({ kind: "error", text: `Reddit search failed: ${(error as Error).message}` }); } finally { setRedditBusy(false); } }}>{redditBusy() ? "Searching…" : "Search"}</button></div>
                        <div class="capability-list"><For each={redditResults()}>{(item) => <article class="capability-item"><a href={item.url} target="_blank" rel="noreferrer noopener">{item.title}</a><small>{item.subreddit} · {item.score} points · {item.comments} comments · {item.created_at ? new Date(item.created_at).toLocaleDateString() : ""}</small><Show when={item.snippet}><p>{item.snippet}</p></Show></article>}</For></div>
                      </li></Show></Show>
                      <Show when={isX && plugin()?.enabled}><li><div class="social-step-title"><span class="social-step-num">2</span><strong>Bearer token</strong><Show when={inherited()}><span class="social-badge">Using the Shared token</span></Show></div>
                        <p class="settings-hint">An app bearer token from the X developer console. X bills API use, and an X subscription does not include API access. Set a spending limit in the console too.</p>
                        <a href="https://developer.x.com/en/portal/dashboard" target="_blank" rel="noreferrer noopener">Open the X developer console</a>
                        <Show when={!xConfigured()}>
                          <label>Bearer token<input type="password" autocomplete="new-password" value={xInput()} onInput={(event) => setXInput(event.currentTarget.value)} placeholder={inherited() ? "Enter a token to override the Shared one" : "Paste token; it will not be shown again"} /></label>
                          <div class="settings-actions"><button type="button" class="settings-button" disabled={xBusy() || !xInput().trim()} onClick={async () => { setXBusy(true); try { await api.saveXToken(xInput(), activeAgentId(), credScope()); setXInput(""); await refreshCapabilities(); setNotice({ kind: "info", text: scope() === "user" ? "X token saved for every agent." : "X token saved for this agent." }); } catch (error) { setNotice({ kind: "error", text: `Could not save X token: ${(error as Error).message}` }); } finally { setXBusy(false); } }}>Save token</button></div>
                        </Show>
                        <Show when={xConfigured()}><div class="settings-actions"><span class="capability-state ready">{scope() === "user" ? "Shared token saved" : `Token saved for ${agentName()}`}</span><button type="button" class="settings-button danger" disabled={xBusy()} onClick={async () => { try { await api.removeXToken(activeAgentId(), credScope()); setXResults([]); await refreshCapabilities(); setNotice({ kind: "info", text: "X token removed." }); } catch (error) { setNotice({ kind: "error", text: `Could not remove X token: ${(error as Error).message}` }); } }}>Remove token</button></div></Show>
                      </li>
                      <li><div class="social-step-title"><span class="social-step-num">3</span><strong>Monthly search limit</strong><span class="social-badge">Shared by every agent</span></div>
                        <p class="settings-hint">Every search counts before it is sent, including one that fails, and searching stops at the limit. This bounds how many searches Vakyartha makes. It is not a price: check your X billing for cost.</p>
                        <Show when={xUsage()}><p class="capability-state" classList={{ ready: xUsage()!.used < xUsage()!.limit, muted: xUsage()!.used >= xUsage()!.limit }}>{xUsage()!.used} of {xUsage()!.limit} searches used in {xUsage()!.month}{xUsage()!.used >= xUsage()!.limit ? " — limit reached" : ""}</p></Show>
                        <div class="settings-actions"><input type="number" min="1" max="1000" aria-label="Monthly X search limit" value={xLimitInput()} onInput={(event) => setXLimitInput(event.currentTarget.value)} placeholder={xUsage() ? String(xUsage()!.limit) : "20"} /><button type="button" class="settings-button" disabled={xBusy() || !Number.isInteger(Number(xLimitInput())) || Number(xLimitInput()) < 1} onClick={async () => { setXBusy(true); try { setXUsage(await api.setXLimit(Number(xLimitInput()), activeAgentId())); setXLimitInput(""); } catch (error) { setNotice({ kind: "error", text: `Could not set the limit: ${(error as Error).message}` }); } finally { setXBusy(false); } }}>Set limit</button></div>
                      </li>
                      <Show when={scope() !== "user" && (xConfigured() || inherited())}><li><div class="social-step-title"><span class="social-step-num">4</span><strong>Try a search</strong></div>
                        <div class="settings-actions"><input aria-label="X search query" value={xQuery()} onInput={(event) => setXQuery(event.currentTarget.value)} maxlength={200} placeholder="Search recent public posts" /><button type="button" class="settings-button" disabled={xBusy() || !xQuery().trim()} onClick={async () => { setXBusy(true); try { const result = await api.searchXPreview(xQuery(), 5, activeAgentId()); setXResults(result.items); setXUsage(result.usage); } catch (error) { setNotice({ kind: "error", text: `X search failed: ${(error as Error).message}` }); await refreshCapabilities(); } finally { setXBusy(false); } }}>{xBusy() ? "Searching…" : "Search"}</button></div>
                        <div class="capability-list"><For each={xResults()}>{(item) => <article class="capability-item"><a href={item.url} target="_blank" rel="noreferrer noopener">@{item.handle}</a><small>{item.likes} likes · {item.reposts} reposts · {item.created_at ? new Date(item.created_at).toLocaleDateString() : ""}</small><p>{item.text}</p></article>}</For></div>
                      </li></Show></Show>
                      <Show when={isLinkedin && plugin()?.enabled}><li><div class="social-step-title"><span class="social-step-num">2</span><strong>App Client ID</strong><Show when={inherited()}><span class="social-badge">Using the Shared Client ID</span></Show></div>
                        <p class="settings-hint">From a LinkedIn app with “Sign In with LinkedIn using OpenID Connect” and native PKCE enabled. Never enter a password or client secret here.</p>
                        <a href="https://www.linkedin.com/developers/apps" target="_blank" rel="noreferrer noopener">Open LinkedIn Developer Portal</a>
                        <label>Client ID<input aria-label="LinkedIn app Client ID" autocomplete="off" value={linkedinClientIdInput()} onInput={(event) => setLinkedinClientIdInput(event.currentTarget.value)} maxlength={256} placeholder={linkedinClientIdConfigured() ? "Enter a new Client ID to replace the saved one" : inherited() ? "Enter an ID to override the Shared one" : "Paste your app's Client ID"} /></label>
                        <div class="settings-actions">
                          <button type="button" class="settings-button" disabled={linkedinBusy() || !linkedinClientIdInput().trim()} onClick={() => void saveLinkedinClientId()}>{linkedinClientIdConfigured() ? "Update Client ID" : "Save Client ID"}</button>
                          <Show when={linkedinClientIdConfigured()}><button type="button" class="settings-button subtle" disabled={linkedinBusy()} onClick={async () => { setLinkedinBusy(true); try { await api.removeLinkedinClientId(activeAgentId(), credScope()); await refreshCapabilities(); setNotice({ kind: "info", text: "LinkedIn Client ID removed. Disconnect the profile separately to remove its saved sign-in." }); } catch (error) { setNotice({ kind: "error", text: `Could not remove Client ID: ${error instanceof Error ? error.message : String(error)}` }); } finally { setLinkedinBusy(false); } }}>Remove</button></Show>
                        </div>
                      </li>
                      <Show when={scope() !== "user"}><li><div class="social-step-title"><span class="social-step-num">3</span><strong>Your profile</strong></div>
                        <p class="settings-hint">Only your profile name, through OpenID Connect (<code>openid profile</code>). It does not verify identity, search posts, or grant content access, and it stays on this agent.</p>
                        <p class="capability-state" classList={{ ready: linkedinAccount().connected && !linkedinAccount().expired, muted: !linkedinAccount().connected || linkedinAccount().expired }}>{linkedinAccount().connected ? linkedinAccount().expired ? `Connected as ${linkedinAccount().display_name ?? "LinkedIn member"}; sign-in expired` : `Connected as ${linkedinAccount().display_name ?? "LinkedIn member"}` : "Not connected"}</p>
                        <Show when={linkedinAccount().connected}><p class="settings-hint">Requested: {(linkedinAccount().scopes_requested ?? []).join(", ") || "not reported"}. Expires {linkedinAccount().expires_at ? new Date(linkedinAccount().expires_at!).toLocaleString() : "at an unknown time"}.</p></Show>
                        <div class="settings-actions">
                          <button type="button" class="settings-button" disabled={linkedinBusy() || !(linkedinClientIdConfigured() || linkedinInherited())} title={!(linkedinClientIdConfigured() || linkedinInherited()) ? "Save a Client ID first" : "Open LinkedIn sign-in"} onClick={() => void connectLinkedinAccount()}>{linkedinBusy() ? "Opening…" : linkedinAccount().connected ? "Reconnect" : "Connect LinkedIn"}</button>
                          <Show when={linkedinAccount().connected}><button type="button" class="settings-button danger" disabled={linkedinBusy()} onClick={() => void disconnectLinkedinAccount()}>Disconnect</button></Show>
                        </div>
                      </li></Show></Show>
                    </ol>
                    <div class="social-card-foot"><a href={connector.official_api} target="_blank" rel="noreferrer noopener">Official API information</a></div>
                  </article>;
                }}</For></div>
              </section>}</For>
            </Show>

            <Show when={page() === "connections"}>
              <header><h1>Capabilities</h1><p>{scope() === "user" ? "Find and manage what every agent can use." : `Find and manage what ${agentName()} can use, including shared capabilities.`}</p></header>
              <nav class="capability-tabs" aria-label="Capability views">
                <button classList={{ active: capabilityView() === "discover" }} onClick={() => setCapabilityView("discover")}>Discover</button>
                <button classList={{ active: capabilityView() === "mine" }} onClick={() => setCapabilityView("mine")}>{scope() === "user" ? "Shared capabilities" : `${agentName()}'s capabilities`}</button>
                <button classList={{ active: capabilityView() === "manage" }} onClick={() => setCapabilityView("manage")}>Manage</button>
              </nav>
              <Show when={capabilityView() === "discover"}>
                <div class="capability-discover-intro"><strong>Explore what is available</strong><span>Skills already on this device and packages from your registered catalogs appear here. Catalog listings are not installed automatically.</span></div>
                <div class="capability-discover-search"><Icon name="search" /><input aria-label="Search capabilities" placeholder="Search skills and catalog packages" value={discoveryQuery()} onInput={(event) => setDiscoveryQuery(event.currentTarget.value)} /></div>
                <div class="capability-discover-filters" role="group" aria-label="Capability type">
                  <For each={(["all", "skills", "plugins"] as const)}>{(kind) => <button type="button" classList={{ active: discoveryKind() === kind }} onClick={() => setDiscoveryKind(kind)}>{kind === "all" ? "All" : kind === "skills" ? "Skills" : "Packages"}</button>}</For>
                </div>
                <Show when={marketplaceErrors().length > 0}><div class="settings-callout" role="status"><Icon name="shield" /><div><strong>Some catalog entries could not be loaded</strong><span>{marketplaceErrors().join(" · ")}</span></div></div></Show>
                <Show when={discoverableSkills().length + discoverablePlugins().length > 0} fallback={<div class="capability-empty"><span class="capability-empty-icon skills"><Icon name="spark" /></span><strong>{discoveryQuery().trim() ? "No matching capabilities" : "Nothing to discover yet"}</strong><span>{discoveryQuery().trim() ? "Try another search." : "Available skills appear here automatically. You can register a package catalog under Manage."}</span><Show when={!discoveryQuery().trim()}><button class="settings-button" onClick={() => { setCapabilityView("manage"); setCapabilityTab("plugins"); }}>Manage catalogs</button></Show></div>}>
                  <div class="marketplace-grid">
                    <For each={discoverableSkills()}>{(skill) => <div class="marketplace-card"><div class="marketplace-card-head"><CapabilityIcon name={skill.name} /><div><strong>{skill.name}</strong><small>Skill · {skill.scope === "user" ? "Shared with every agent" : "This agent"}</small></div></div><p>{skill.description || "No description provided."}</p><div class="settings-actions"><button class="settings-button" onClick={() => { setCapabilityView("mine"); }}>View in my capabilities</button></div></div>}</For>
                    <For each={discoverablePlugins()}>{(entry) => <div class="marketplace-card"><div class="marketplace-card-head"><CapabilityIcon name={entry.name} /><div><strong>{entry.name}</strong><small>Package · {entry.source_label}{entry.version ? ` · v${entry.version}` : ""}</small></div></div><p>{entry.description || "No description provided."}</p><small>{entry.source_scope === "user" ? "Shared catalog" : "Agent catalog"} · {entry.source_enabled ? "Source enabled" : "Source disabled"}{entry.license ? ` · ${entry.license}` : " · License not declared"}</small><div class="settings-actions"><button class="settings-button" onClick={() => { setCapabilityView("manage"); setCapabilityTab("plugins"); setMarketplaceQuery(entry.name); }}>Details</button><button class="settings-button" disabled={!entry.source_enabled || !entry.license || pluginBusy()} title={!entry.source_enabled ? "Enable this catalog under Manage first" : !entry.license ? "A license must be declared before installation" : "Package changes are staged disabled for review"} onClick={() => void installCatalogPlugin(entry, plugins().some((plugin) => plugin.name === entry.name))}>{plugins().some((plugin) => plugin.name === entry.name) ? "Stage update" : "Install disabled"}</button></div></div>}</For>
                  </div>
                </Show>
              </Show>
              <Show when={capabilityView() === "mine"}>
                <Show when={scope() === "workspace"}>
                  <Group title={`Shared capabilities for ${agentName()}`}>
                    <p class="settings-hint">Choose which Shared capabilities this agent can use. Turning one off keeps the agent’s own settings and hides Shared entries from its turns.</p>
                    <For each={[
                      ["mcp", "Connected apps", "Shared MCP connections"] as const,
                      ["hooks", "Hooks", "Shared lifecycle hooks"] as const,
                      ["skills", "Skills", "Shared skills"] as const,
                      ["commands", "Commands", "Shared commands"] as const,
                      ["plugins", "Add-ons", "Shared plugins"] as const,
                    ]}>{([kind, label, detail]) => <Row title={label} description={detail}><span class="settings-inheritance-control"><span class="capability-state" classList={{ inherited: inherits(kind), muted: !inherits(kind) }}>{inherits(kind) ? "Inherited from Shared" : "Agent only"}</span><Switch label={`${label} inherit from Shared for ${agentName()}`} checked={inherits(kind)} onChange={(enabled) => void setCapabilityInheritance(kind, enabled)} /><Show when={inheritanceBusy() === kind}><span class="settings-status">Saving…</span></Show></span></Row>}</For>
                  </Group>
                </Show>
                <p class="settings-hint">{scope() === "user" ? "These shared capabilities are available to every agent unless its own settings narrow access." : `These are ${agentName()}'s capabilities. Agent settings take precedence over shared defaults where the same name is configured.`}</p>
                <Group title={`Skills (${visibleSkills().length})`}><Show when={visibleSkills().length > 0} fallback={<div class="capability-empty"><strong>No skills available</strong><span>Skills discovered for this scope will appear here.</span></div>}><div class="capability-list"><For each={visibleSkills()}>{(skill) => <details class="capability-item"><summary><span><CapabilityIcon name={skill.name} /><span class="capability-title"><strong>{skill.name}</strong><small>{skill.scope === "user" ? "Shared" : "This agent"}</small></span></span><span class="capability-state" classList={{ ready: !skill.shadowed, muted: !!skill.shadowed }}>{skill.shadowed ? "Overridden for this agent" : "Available"}</span></summary><div class="capability-detail"><p>{skill.description || "No description provided."}</p><Show when={skill.provenance}><small>{skill.provenance}</small></Show><Show when={(skill.path || skill.source) && technicalDetails()}><code>{skill.path || skill.source}</code></Show><button class="settings-button" onClick={async () => { try { await navigator.clipboard.writeText(`/skill ${skill.name} `); setNotice({ kind: "info", text: `Copied /skill ${skill.name} to your clipboard.` }); } catch { setNotice({ kind: "error", text: "Could not copy the skill command." }); } }}>Copy skill command</button></div></details>}</For></div></Show></Group>
                <Group title={`Connections (${totalMcpCount()})`}><Show when={totalMcpCount() > 0} fallback={<div class="capability-empty"><strong>No connections configured</strong><span>{!inherits("mcp") ? "Shared connections are turned off for this agent. Add one under Manage or change this agent’s inheritance settings." : "Add one under Manage to give this agent more tools."}</span></div>}><div class="capability-list"><For each={Object.keys(mcpServers() ?? {})}>{(name) => <div class="capability-item capability-overview-row"><CapabilityIcon name={name} /><strong>{name}</strong><span class="capability-state ready">{scope() === "user" ? "Shared" : "This agent"}</span></div>}</For><Show when={scope() === "workspace" && inherits("mcp")}><For each={Object.keys(inheritedMcpServers()).filter((name) => !(name in (mcpServers() ?? {})))}>{(name) => <div class="capability-item capability-overview-row"><CapabilityIcon name={name} /><strong>{name}</strong><span class="capability-state inherited">Inherited from Shared</span></div>}</For></Show></div></Show></Group>
                <Group title={`Add-ons (${totalPluginsCount()})`}><Show when={totalPluginsCount() > 0} fallback={<div class="capability-empty"><strong>No add-ons installed</strong><span>{!inherits("plugins") ? "Shared add-ons are turned off for this agent." : "Installed packages will appear here, including shared ones."}</span></div>}><div class="capability-list"><For each={[...plugins().map((plugin) => ({ plugin, inherited: false })), ...(scope() === "workspace" && inherits("plugins") ? inheritedPlugins().map((plugin) => ({ plugin, inherited: true })) : [])]}>{({ plugin, inherited }) => <div class="capability-item capability-overview-row"><CapabilityIcon name={plugin.name} /><strong>{plugin.name}</strong><span class="capability-state" classList={{ ready: plugin.enabled, muted: !plugin.enabled }}>{plugin.enabled ? (inherited ? "Inherited from Shared" : "Enabled") : "Disabled"}</span></div>}</For></div></Show></Group>
                <Group title={`Hooks (${totalHooksCount()})`}><Show when={totalHooksCount() > 0} fallback={<div class="capability-empty"><strong>No hooks configured</strong><span>{!inherits("hooks") ? "Shared hooks are turned off for this agent." : "Hooks added under Manage will appear here."}</span></div>}><div class="capability-list"><For each={[...hooks().map((hook) => ({ hook, inherited: false })), ...(scope() === "workspace" && inherits("hooks") ? inheritedHooks().map((hook) => ({ hook, inherited: true })) : [])]}>{({ hook, inherited }) => <div class="capability-item capability-overview-row"><Icon name="tune" /><strong>{hook.event}{hook.matcher ? ` · ${hook.matcher}` : ""}</strong><span class="capability-state" classList={{ ready: hook.enabled !== false, muted: hook.enabled === false }}>{hook.enabled === false ? "Disabled" : inherited ? "Inherited from Shared" : "Enabled"}</span></div>}</For></div></Show></Group>
                <button class="settings-button" onClick={() => setCapabilityView("manage")}>Manage these capabilities</button>
              </Show>
              <Show when={capabilityView() === "manage"}>
              <nav class="capability-tabs" aria-label="Capability types">
                <button classList={{ active: shownCapabilityTab() === "mcp" }} onClick={() => setCapabilityTab("mcp")}><Icon name="plug" /><span>Connections</span><em>{totalMcpCount()}</em></button>
                <button classList={{ active: shownCapabilityTab() === "skills" }} onClick={() => setCapabilityTab("skills")}><Icon name="spark" /><span>Skills</span><em>{visibleSkills().length}</em></button>
                  <button classList={{ active: capabilityTab() === "hooks" }} onClick={() => setCapabilityTab("hooks")}><Icon name="tune" /><span>Hooks</span><em>{totalHooksCount()}</em></button>
                  <button classList={{ active: capabilityTab() === "plugins" }} onClick={() => setCapabilityTab("plugins")}><Icon name="grid" /><span>Add-ons</span><em>{totalPluginsCount()}</em></button>
              </nav>
              <Show when={shownCapabilityTab() === "mcp"}>
                <Group title="Connections"><Show when={mcpServers()} fallback={<Skeleton kind="rows" label="Loading connections" />}>
                  <div class="mcp-editor">
                    <Show when={Object.keys(mcpServers() ?? {}).length > 0} fallback={<div class="capability-empty"><span class="capability-empty-icon mcp"><Icon name="plug" /></span><strong>{scope() === "user" ? "No shared connections yet" : (Object.keys(inheritedMcpServers()).length > 0 ? "Nothing added for this agent" : "No connections yet")}</strong><span>{scope() === "user" ? "Add one to give every agent a tool such as search, a browser or your data." : (Object.keys(inheritedMcpServers()).length > 0 ? "This agent uses the shared connections below. Add one here to give only this agent a tool." : "Add a connection to give the agent tools such as search, a browser or your data.")}</span></div>}>
                      <For each={Object.entries(mcpServers() ?? {})}>{([name, def]) => <div class="mcp-row">
                        <div class="mcp-row-head"><div><CapabilityIcon name={name} /><strong>{name}</strong><span class="capability-state ready">Configured</span></div><button class="settings-button danger" onClick={() => removeServer(name)}><Icon name="trash" /> Remove</button></div>
                        <div class="mcp-fields"><label>Server name<input value={name} aria-label="Server name" onChange={(e) => renameServer(name, e.currentTarget.value.trim())} /></label><label>Command<input placeholder="/path/to/command" value={def.command} aria-label="Command" onInput={(e) => updateServer(name, { command: e.currentTarget.value })} /></label><label>Arguments<input placeholder="Space-separated arguments" value={def.args.join(" ")} aria-label="Arguments" onInput={(e) => updateServer(name, { args: e.currentTarget.value.split(" ").filter(Boolean) })} /></label></div>
                        <div class="mcp-controls"><label class="mcp-network"><Switch checked={def.network} label={`Allow network for ${name}`} onChange={(v) => updateServer(name, { network: v })} /><span>Allow internet access</span></label></div>
                      </div>}</For>
                    </Show>
                    <div class="settings-actions"><button class="btn" onClick={addServer}><Icon name="add" /> Add connection</button><button class="btn primary" disabled={!mcpDirty() || mcpSaving()} onClick={() => void saveMcp()}>{mcpSaving() ? "Saving…" : "Save & apply"}</button><Show when={mcpDirty()}><span class="mcp-dirty">Unsaved changes</span></Show></div>
                    <p class="settings-hint">{scope() === "user" ? "Every agent can use these. Saved keys are never shown here." : "These apply only to this agent. Internet access stays off until you turn it on."}</p>

                    <Show when={scope() === "workspace" && Object.keys(inheritedMcpServers()).length > 0}>
                      <div class="inherited-capabilities-group">
                        <div class="inherited-capabilities-head">
                          <Icon name="layers" />
                          <div>
                            <strong>Shared connections ({Object.keys(inheritedMcpServers()).length})</strong>
                            <span>{inherits("mcp") ? "Available from your shared settings." : "Shared connections are turned off for this agent."}</span>
                          </div>
                        </div>
                        <div class="capability-list">
                          <For each={Object.entries(inheritedMcpServers())}>{([name, def]) => {
                            const isOverridden = () => !!mcpServers()?.[name];
                            return (
                              <div class="capability-item" classList={{ "capability-item-overridden": isOverridden() }}>
                                <div class="inherited-row">
                                  <div class="inherited-info" style="display: flex; align-items: flex-start; gap: 10px;">
                                    <CapabilityIcon name={name} />
                                    <div>
                                      <strong>{name}</strong>
                                      <Show when={technicalDetails()}><code>{def.command} {def.args.join(" ")}</code></Show>
                                      <small>{def.network ? "Can use the internet" : "This computer only"}</small>
                                    </div>
                                  </div>
                                  <div class="inherited-actions">
                                    <Show when={isOverridden()} fallback={
                                      <>
                                        <span class="capability-state inherited">{inherits("mcp") ? "Inherited" : "Not inherited"}</span>
                                        <button class="settings-button" onClick={() => {
                                          updateServer(name, { command: def.command, args: [...def.args], env: { ...def.env }, network: def.network });
                                          setNotice({ kind: "info", text: `Copied ${name} to this agent's connections. You can now change it.` });
                                        }}>Change for this agent</button>
                                      </>
                                    }>
                                      <span class="capability-state muted">Changed for this agent</span>
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
              <Show when={shownCapabilityTab() === "skills"}>
                <Group title={`Skills (${visibleSkills().length})`}><Show when={visibleSkills().length > 0} fallback={<div class="capability-empty"><span class="capability-empty-icon skills"><Icon name="spark" /></span><strong>No skills discovered</strong><span>{scope() === "user" ? "Skills you add to the shared defaults appear here for every agent." : "Skills added for this agent or for every agent appear here."}</span></div>}><p class="settings-hint">{scope() === "user" ? "Every agent can use these skills." : "Skills for this agent and for every agent. Each shows where it came from."}</p><div class="capability-list"><For each={visibleSkills()}>{(skill) => <details class="capability-item"><summary><span><CapabilityIcon name={skill.name} /><span class="capability-title"><strong>{skill.name}</strong><small>{skill.scope === "user" ? "Every agent" : "This agent"}</small></span></span><span class="capability-state ready">Available</span></summary><div class="capability-detail"><p>{skill.description || "No description provided."}</p><Show when={skill.source && technicalDetails()}><code>{skill.source}</code></Show><button type="button" class="settings-button" onClick={async () => { try { if (!navigator.clipboard) throw new Error("Clipboard access is unavailable"); await navigator.clipboard.writeText(`/skill ${skill.name} `); setNotice({ kind: "info", text: `Copied /skill ${skill.name} to your clipboard. Open a task and paste it into the composer.` }); } catch { setNotice({ kind: "error", text: "Could not copy the skill command. Clipboard access was denied." }); } }}>Copy to composer</button></div></details>}</For></div></Show></Group>
                <Group title={`Suggested skills (${proposals().length})`}><Show when={proposals().length > 0} fallback={<Row title="No suggestions waiting" description="Agents can suggest skills to reuse; nothing is used until you approve it."><span class="settings-status good">Clear</span></Row>}><For each={proposals()}>{(proposal) => <div class="setting-row"><div class="setting-copy"><strong>{proposal.name}</strong><span>{proposal.description}</span></div><div class="setting-control"><button class="settings-button" onClick={() => void promote(proposal.id)}>Approve</button><button class="settings-button danger" onClick={() => void reject(proposal.id)}>Reject</button></div></div>}</For></Show></Group>
              </Show>
              <Show when={shownCapabilityTab() === "hooks"}>
                <Group title="Hooks"><div class="settings-callout"><Icon name="shield" /><div><strong>Hooks run commands at controlled lifecycle points.</strong><span>Use closed failure handling for guards where a timeout or unavailable script must stop the operation.</span></div></div><Show when={hooks().length > 0} fallback={<div class="capability-empty"><span class="capability-empty-icon hooks"><Icon name="tune" /></span><strong>{scope() === "user" ? "No shared hooks configured" : (inheritedHooks().length > 0 ? "No project-specific hooks" : "No hooks configured")}</strong><span>{scope() === "user" ? "Add a hook to run a safe, repeatable action at session or tool lifecycle events across all projects." : (inheritedHooks().length > 0 ? "This project is using the shared hooks listed below. Add a hook here to run project-specific actions." : "Add a hook to run a safe, repeatable action at session or tool lifecycle events.")}</span></div>}><div class="hook-editor"><For each={hooks()}>{(hook, index) => <div class="hook-row"><div class="hook-row-head"><span class="capability-state" classList={{ ready: hook.enabled !== false, muted: hook.enabled === false }}>{hook.enabled === false ? "Disabled" : "Enabled"}</span><Switch checked={hook.enabled !== false} label={`Enable hook ${index() + 1}`} onChange={(v) => updateHook(index(), { enabled: v })} /><button class="settings-button danger" aria-label={`Remove hook ${index() + 1}`} onClick={() => removeHook(index())}><Icon name="trash" /></button></div><label>Lifecycle event<select value={hook.event} onChange={(e) => updateHook(index(), { event: e.currentTarget.value })}><option value="session_start">Session start</option><option value="pre_tool_use">Before a tool runs</option><option value="post_tool_use">After a tool runs</option><option value="stop">Task stop</option></select></label><label>Tool matcher <span class="label-hint">optional</span><input value={hook.matcher ?? ""} placeholder="bash, write, or leave blank" onInput={(e) => updateHook(index(), { matcher: e.currentTarget.value })} /></label><label>Command<input class="hook-command" value={hook.command} placeholder="e.g. cargo fmt --all --check" onInput={(e) => updateHook(index(), { command: e.currentTarget.value })} /></label><label>Timeout (ms)<input type="number" min="100" max="120000" value={hook.timeout_ms ?? 10000} onInput={(e) => updateHook(index(), { timeout_ms: Number(e.currentTarget.value) || 10000 })} /></label><label>On hook failure<select value={hook.failure_mode ?? "open"} onChange={(e) => updateHook(index(), { failure_mode: e.currentTarget.value as "open" | "closed" })}><option value="open">Continue and report</option><option value="closed">Block the operation</option></select></label></div>}</For></div></Show><div class="settings-actions"><button class="btn" onClick={addHook}><Icon name="add" /> Add hook</button><button class="btn primary" disabled={!hooksDirty() || hooksSaving()} onClick={() => void saveHooks()}>{hooksSaving() ? "Saving…" : "Save hooks"}</button><Show when={hooksDirty()}><span class="mcp-dirty">Unsaved changes</span></Show></div>
                <Show when={scope() === "workspace" && inheritedHooks().length > 0}>
                  <div class="inherited-capabilities-group">
                    <div class="inherited-capabilities-head">
                      <Icon name="layers" />
                      <div>
                        <strong>Shared lifecycle hooks ({inheritedHooks().length})</strong>
                        <span>{inherits("hooks") ? "Inherited from Shared. These run alongside this agent’s hooks." : "Shared hooks are turned off for this agent."}</span>
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
                              <span class="capability-state inherited">{hook.enabled === false ? "Disabled" : inherits("hooks") ? "Inherited (Active)" : "Not inherited"}</span>
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
              <Show when={shownCapabilityTab() === "plugins"}>
                <Group title="Installed plugins">
                  <div class="settings-callout"><Icon name="shield" /><div><strong>Plugins are installed disabled.</strong><span>Each generation is content-addressed and remains inactive until you explicitly enable it. Review the digest, publisher, and capabilities first.</span></div></div>
                  <div class="mcp-fields"><label>Package directory<input class="mono" placeholder="/path/to/plugin" value={pluginPath()} onInput={(e) => setPluginPath(e.currentTarget.value)} /></label><div class="settings-actions"><button type="button" class="btn" disabled={!pluginPath().trim() || pluginBusy()} onClick={() => void installPlugin(false)}>Install disabled</button><button type="button" class="btn primary" disabled={!pluginPath().trim() || pluginBusy()} onClick={() => void installPlugin(true)}>Stage update</button><button type="button" class="settings-button" disabled={pluginBusy()} onClick={async () => { const picked = await host.pickWorkspace(); if (typeof picked === "string") setPluginPath(picked); }}>Choose…</button></div></div>
                  <div class="mcp-fields"><label>Catalog directory<input class="mono" placeholder="/path/to/catalog" value={sourcePath()} onInput={(e) => setSourcePath(e.currentTarget.value)} /></label><div class="settings-actions"><button class="settings-button" disabled={!sourcePath().trim() || pluginBusy()} onClick={() => void registerPluginSource()}>Register catalog source</button></div></div>
                  <details class="advanced"><summary>Detached Ed25519 evidence (optional)</summary><div class="mcp-fields"><label>Key ID<input class="mono" value={sourceKeyId()} onInput={(e) => setSourceKeyId(e.currentTarget.value)} /></label><label>Public key (base64)<input class="mono" value={sourcePublicKey()} onInput={(e) => setSourcePublicKey(e.currentTarget.value)} /></label><label>Signature (base64)<input class="mono" value={sourceSignature()} onInput={(e) => setSourceSignature(e.currentTarget.value)} /></label></div></details>
                  <Show when={pluginSources().length > 0}><div class="capability-list"><For each={pluginSources()}>{(source) => <div class="capability-item"><strong>{source.label}</strong><small>{source.format} · {source.trust} · {source.enabled ? "Enabled" : "Disabled"} · {source.signature ? (source.signature.verified ? "Signed" : "Signature invalid") : "Unsigned"}</small><code title={source.catalog_digest}>sha256:{source.catalog_digest.slice(0, 16)}</code><div class="settings-actions"><button type="button" class="settings-button" disabled={pluginBusy()} onClick={async () => { setPluginBusy(true); try { await api.pluginSourceAction(source.id, source.enabled ? "disable" : "enable", capabilityScope(), activeAgentId()); await refreshCapabilities(); } catch (e) { setNotice({ kind: "error", text: `Source action failed: ${e instanceof Error ? e.message : String(e)}` }); } finally { setPluginBusy(false); } }}>{source.enabled ? "Disable source" : "Enable source"}</button><Show when={source.signature}><button type="button" class="settings-button danger" disabled={pluginBusy()} onClick={async () => { setPluginBusy(true); try { await api.pluginKeyAction(source.signature!.key_id, source.signature!.revoked ? "restore" : "revoke", capabilityScope(), activeAgentId()); await refreshCapabilities(); } catch (e) { setNotice({ kind: "error", text: `Key action failed: ${e instanceof Error ? e.message : String(e)}` }); } finally { setPluginBusy(false); } }}>{source.signature!.revoked ? "Restore key" : "Revoke key"}</button></Show></div></div>}</For></div></Show>
                  <Show when={scope() === "workspace" && inheritedPluginSources().length > 0}><div class="inherited-capabilities-group"><div class="inherited-capabilities-head"><Icon name="layers" /><div><strong>Shared catalogs ({inheritedPluginSources().length})</strong><span>Registered for every agent. Change them from Shared defaults.</span></div></div><div class="capability-list"><For each={inheritedPluginSources()}>{(source) => <div class="capability-item capability-overview-row"><CapabilityIcon name={source.label} /><strong>{source.label}</strong><span class="capability-state inherited">{source.enabled ? "Shared · enabled" : "Shared · disabled"}</span></div>}</For></div></div></Show>
                  <div class="mcp-fields"><label>Search marketplace entries<input value={marketplaceQuery()} placeholder="frontend, testing, release…" onInput={(e) => setMarketplaceQuery(e.currentTarget.value)} /></label></div>
                  <Show when={marketplaceEntries().filter((entry) => `${entry.name} ${entry.description ?? ""} ${entry.source_label}`.toLowerCase().includes(marketplaceQuery().trim().toLowerCase())).length > 0} fallback={<Show when={marketplaceQuery().trim()}><div class="capability-empty"><span class="capability-empty-icon plugins"><Icon name="grid" /></span><strong>No matching entries</strong><span>Try a different search term, or register a catalog source above.</span></div></Show>}>
                    <div class="marketplace-grid"><For each={marketplaceEntries().filter((entry) => `${entry.name} ${entry.description ?? ""} ${entry.source_label}`.toLowerCase().includes(marketplaceQuery().trim().toLowerCase()))}>{(entry) => (
                      <div class="marketplace-card">
                        <div class="marketplace-card-head"><CapabilityIcon name={entry.name} /><div><strong>{entry.name}</strong><small>{entry.source_label} · {entry.source_enabled ? "Source enabled" : "Source disabled"}{entry.version ? ` · v${entry.version}` : ""}</small></div></div>
                        <p>{entry.description || "No description"}</p>
                        <code title={entry.catalog_digest}>sha256:{entry.catalog_digest.slice(0, 16)}</code>
                        <small>{entry.license ? `License: ${entry.license}` : "License: not declared"}</small>
                      </div>
                    )}</For></div>
                  </Show>
                  <Show when={plugins().length > 0} fallback={<div class="capability-empty"><span class="capability-empty-icon plugins"><Icon name="grid" /></span><strong>{scope() === "user" ? "No shared plugins installed" : (inheritedPlugins().length > 0 ? "No project-specific plugins installed" : "No plugins installed")}</strong><span>{scope() === "user" ? "Install a reviewed package to make its skills, commands, and integrations available everywhere." : (inheritedPlugins().length > 0 ? "This project inherits the shared plugins listed below. Install a project-specific package below if needed." : "Install a reviewed local package to make its skills, commands, and integrations available.")}</span></div>}>
                    <div class="capability-list"><For each={plugins()}>{(plugin) => <details class="capability-item"><summary><span><CapabilityIcon name={plugin.name} /><span class="capability-title"><strong>{plugin.name}</strong><small>v{plugin.version} · {plugin.scope} · {plugin.format}</small></span></span><span class="capability-state" classList={{ ready: plugin.enabled, muted: !plugin.enabled }}>{plugin.enabled ? "Enabled" : "Disabled"}</span></summary><div class="capability-detail"><p>{plugin.description || "No description provided."}</p><code title={plugin.digest}>sha256:{plugin.digest.slice(0, 16)}</code><code>{plugin.trace_id}</code><div class="mcp-controls"><Show when={!plugin.network_denied} fallback={<span class="capability-state muted">Blocked by plugins.network_deny</span>}><label class="mcp-network"><Switch checked={plugin.network_allowed} label={`Allow network for ${plugin.name}`} onChange={(v) => void togglePluginNetwork(plugin.name, v)} /><span title="Deny entries (plugins.network_deny) always win; the allowlist grants egress otherwise. Privileged: set at the trusted (user) level for an untrusted workspace.">{plugin.network_allowed ? "Network allowed from sandbox" : "Local only"}</span></label></Show></div><div class="settings-actions"><button class="settings-button" disabled={pluginBusy()} onClick={() => void mutatePlugin(plugin.name, plugin.enabled ? "disable" : "enable")}>{plugin.enabled ? "Disable" : "Enable"}</button><button class="settings-button" disabled={pluginBusy()} onClick={() => void mutatePlugin(plugin.name, "rollback")}>Rollback</button><button class="settings-button danger" disabled={pluginBusy()} onClick={() => void mutatePlugin(plugin.name, "remove")}>Remove</button></div></div></details>}</For></div>
                  </Show>
                  <Show when={scope() === "workspace" && inheritedPlugins().length > 0}>
                    <div class="inherited-capabilities-group">
                      <div class="inherited-capabilities-head">
                        <Icon name="layers" />
                        <div>
                          <strong>Shared user plugins ({inheritedPlugins().length})</strong>
                          <span>{inherits("plugins") ? "Installed globally and inherited by this agent." : "Shared add-ons are turned off for this agent."}</span>
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
                              <span class="capability-state inherited">{!inherits("plugins") ? "Not inherited" : plugin.enabled ? "Inherited (Active)" : "Disabled (Shared)"}</span>
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
              <Group title="Chat bots">
                {/* A chat credential belongs to a bot, not to a surface
                    (AGENTS.md invariant 23), and several bots can share a
                    transport, so bots are created, credentialed and approved
                    in the admin console. */}
                <Row title="Telegram, Discord and Slack" description="Reach your agents from a chat app. Each bot has its own sign-in and rules, set up in the admin console."><button type="button" class="settings-button" onClick={() => host.openAdmin("#/gateway")}>Open admin console</button></Row>
              </Group>
              </Show>
            </Show>

            <Show when={page() === "prompts"}>
              <div class="prompt-page">
              <header><h1>Prompts</h1><p>Shape the agent’s voice and working rules without changing what it is allowed to do.</p></header>
              <div class="prompt-scope-note"><Icon name="layers" /><div><strong>{scope() === "user" ? "Shared prompt layer" : "Agent prompt layer"}</strong><span>{scope() === "user" ? "Used as the default across your agents. An agent can override individual blocks." : "Applies only to this agent. Unchanged blocks continue to inherit from Shared."}</span><Show when={promptLayerPath()}><code>{promptLayerPath()}</code></Show></div></div>
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
                          <div class="settings-callout"><Icon name="shield" /><div><strong>Guardrails instruct the model; they do not enforce anything.</strong><span>A model can misread a guardrail or be talked out of it. Permissions and isolation are what enforce limits.</span></div></div>
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
                <p class="settings-hint prompt-effective-intro">What the model actually receives, and where each part came from. Changes apply from the next turn of every session; a running turn keeps the prompt it started with.</p>
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
                <div class="settings-callout"><Icon name="shield" /><div><strong>The lists of tools, skills and connections come from Vakyartha itself and cannot be edited here.</strong><span>They describe the callable interface as it actually is. Editing them could only make the model wrong about its own tools.</span></div></div>
              </Group>
              </div>
            </Show>

            <Show when={page() === "storage"}>
              <header><h1>Storage and backup</h1><p>Where Vakyartha keeps its files, and how to back them up.</p></header>
              <Group title="Paths">
                <Row title="Working directory" description={config()?.paths.cwd ?? backend().cwd ?? ""}><button class="settings-button" onClick={() => void openWorkspaceConfig()}>Open config</button></Row>
                <Row title="Project config" description={config()?.paths.project_config ?? ""}><button class="settings-button" onClick={() => void openWorkspaceConfig()}>Open</button></Row>
                <Row title="Global config" description={config()?.paths.global_config ?? "Not configured"}><button type="button" class="settings-button" onClick={() => void copySettingText(config()?.paths.global_config ?? "", "Global config path")}>Copy path</button></Row>
                <Row title="Session store" description={config()?.paths.sessions_home ?? ""}><button type="button" class="settings-button" onClick={() => void copySettingText(config()?.paths.sessions_home ?? "", "Session store path")}>Copy path</button></Row>
              </Group>
              <Group title="Data & backup">
                <div class="settings-callout"><Icon name="shield" /><div><strong>Your backup includes Vakyartha data and settings.</strong><span>Passwords and API keys stay excluded unless you include them below.</span></div></div>
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
                  description="Include saved API keys when available. Keep this backup private."
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
                  description={`Restore a previous backup. Existing items are ${conflict() === "skip" ? "kept" : "renamed"}.`}
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
