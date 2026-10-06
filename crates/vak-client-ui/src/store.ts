import { createSignal } from "solid-js";
import { isOfficePath } from "./officeFiles";
import { fileSubject, matchExecutionArtifact, type ArtifactOrigin, type CanvasSubject } from "./canvasSubject";
import { activeEntry, applyOp, entriesOf, isShared, type CanvasMode, type CanvasOp, type CanvasStacks, type EntryPatch } from "./canvasStack";
import { freshEntry } from "./canvasViewers";
import { displayFileName } from "./attachFiles";
import { createStore, reconcile, unwrap } from "solid-js/store";
import * as api from "./api";
export { stripControlScaffolding } from "./structured";
import { stripControlScaffolding } from "./structured";
import { mergePresentationSnapshot, applyPresentationDelta } from "./presentationHydration";
import { interfaceFonts, contentFonts, codeFonts } from "./typography";
import type { InterfaceFont, ContentFont, CodeFont } from "./typography";
import type {
  ClientEvent,
  AssistantMessage,
  BackendInfo,
  ContentBlock,
  Health,
  Message,
  SessionSummary,
  TranscriptEntryMeta,
  Usage,
  OutputTimeline,
  PresentationStreamEvent,
} from "./types";

export type Item =
  | { kind: "user"; text: string; entryId?: string; authorId?: string; authorName?: string; files?: api.InboxFile[] }
  | { kind: "assistant"; key: string; text: string; streaming: boolean }
  | { kind: "thinking"; key: string; text: string; done: boolean }
  | {
      kind: "tool";
      id: string;
      name: string;
      argsJson: string;
      done: boolean;
      isError: boolean;
      preview: string | null;
    }
  | {
      kind: "approval";
      id: string;
      tool: string;
      argsJson: string;
      reason: string;
      resolved: null | "allowed" | "denied" | "gone";
    }
  | {
      kind: "question";
      id: string;
      label: string;
      question: string;
      options: string[];
      resolved: null | "answered" | "gone";
    }
  | { kind: "worker"; label: string; lines: string[]; open: boolean; isError: boolean }
  | { kind: "system"; text: string; needs?: "ai-service" };

export const [backend, setBackend] = createSignal<BackendInfo>({ ready: false, recent_workspaces: [] });
export const [workspaceSwitching, setWorkspaceSwitching] = createSignal(false);

/**
 * Whether the client is actually in touch with the server right now
 * (docs/design/48-web-client.md §7.4).
 *
 * In-process on the desktop, "Working" is instantaneous truth. Over a WAN
 * it is a claim about a round trip that may not have happened — so the
 * connection itself becomes a thing the UI has to state rather than
 * assume. `resyncing` is its own state and not a flavour of `reconnecting`
 * because it means something different to the reader: events were lost and
 * the transcript is being rebuilt, so what is on screen is briefly behind.
 */
export type Connection = "connecting" | "live" | "reconnecting" | "resyncing" | "offline";
export const [connection, setConnection] = createSignal<Connection>("live");
export const [sessions, setSessions] = createSignal<SessionSummary[]>([]);
export type CoworkingParticipant = { principal_id: string; display_name: string; office_room_id?: string | null; office_anchor?: string | null };
const [coworkingPresenceBySession, setCoworkingPresenceBySession] = createStore<Record<string, CoworkingParticipant[]>>({});
export const coworkingPresence = (sessionId: string | null) => sessionId ? (coworkingPresenceBySession[sessionId] ?? []) : [];
export function setCoworkingPresence(sessionId: string, participants: CoworkingParticipant[]) {
  setCoworkingPresenceBySession(sessionId, participants);
}
export const [activeId, setActiveId] = createSignal<string | null>(null);
const [armedStopSession, setArmedStopSession] = createSignal<string | null>(null);
export const stopRunArmed = (id: string | null) => !!id && armedStopSession() === id;
export function armStopRun(id: string | null) { setArmedStopSession(id); }
/** A local send is an explicit request to follow the new turn, even when the
 * reader was previously browsing older messages. */
export const [lastSubmittedPrompt, setLastSubmittedPrompt] = createSignal<{ sessionId: string; sequence: number } | null>(null);
let submittedPromptSequence = 0;
export type AgentSummary = { id: string; name: string; revision?: number; character?: string; animation?: "subtle" | "expressive" | "off"; voice?: string };
export const [activeAgent, setActiveAgent] = createSignal<AgentSummary | null>(null);
export function agentForSession(id: string | null): AgentSummary {
  const found = sessions().find((session) => session.session_id === id)?.agent;
  if (found) return found;
  if (id && id === activeId() && activeAgent()) return activeAgent()!;
  return {id: "vak", name: "Vakyartha", revision: 1, character: "vak", animation: "subtle", voice: "default"};
}
export const activeAgentId = () => activeAgent()?.id ?? agentForSession(activeId()).id;
export const [agentOpening, setAgentOpening] = createSignal(false);
export const [openingAgentId, setOpeningAgentId] = createSignal<string | null>(null);
export interface ReplyTarget {
  sessionId: string;
  resultId?: string;
  label: string;
}
export const [replyTarget, setReplyTarget] = createSignal<ReplyTarget | null>(null);
export const [health, setHealth] = createSignal<Health | null>(null);

const PROMPT_HISTORY_KEY = "vak.promptHistory";
function loadPromptHistory(): string[] {
  try {
    return JSON.parse(sessionStorage.getItem(PROMPT_HISTORY_KEY) ?? "[]");
  } catch {
    return [];
  }
}
export const [promptHistory, setPromptHistory] = createSignal<string[]>(loadPromptHistory());

export function recordPrompt(prompt: string) {
  const trimmed = prompt.trim();
  if (!trimmed) return;
  setPromptHistory((prev) => {
    const filtered = prev.filter((p) => p !== trimmed);
    const updated = [trimmed, ...filtered].slice(0, 50);
    try {
      sessionStorage.setItem(PROMPT_HISTORY_KEY, JSON.stringify(updated));
    } catch {
      // ignore
    }
    return updated;
  });
}

// Provider/model picker state. Whether *setup* is complete is not tracked
// here: it is derived from `GET /onboarding` on every read, so there is no
// local flag that can disagree with the server about what is configured.
export const [providers, setProviders] = createSignal<import("./types").ProvidersResponse | null>(null);
/** Show technical details (DESIGN.md, docs/design/75 §8): one switch for how
 * much machinery everyday screens show. Off for new installs. It changes what
 * is shown, never what an agent may do, and never hides a safety state. */
const storedTechnical = (() => { try { return localStorage.getItem("vak.technicalDetails"); } catch { return null; } })();
export const [technicalDetails, setTechnicalDetailsSignal] = createSignal(storedTechnical === "1");
export function setTechnicalDetails(value: boolean) {
  setTechnicalDetailsSignal(value);
  try { localStorage.setItem("vak.technicalDetails", value ? "1" : "0"); } catch { /* the choice still applies to this window */ }
}
export type DockTab = "workbench" | "preview" | "diff" | "terminal" | "editor" | "pr" | "agents" | "sources" | "commitments";
const storedDockTab = localStorage.getItem("vak.dockTab") as DockTab | null;
const validDockTab = storedDockTab && ["workbench", "preview", "diff", "terminal", "editor", "pr", "agents", "sources", "commitments"].includes(storedDockTab)
  ? storedDockTab
  : null;
const [dockTab, setDockTabSignal] = createSignal<DockTab | null>(validDockTab);
export { dockTab };
/** Keep the selected surface across reloads so a useful artifact view returns
 * with the task instead of dropping the operator back on an empty canvas. */
export function setDockTab(value: DockTab | null | ((current: DockTab | null) => DockTab | null)) {
  const next = typeof value === "function" ? value(dockTab()) : value;
  setDockTabSignal(next);
  if (next) localStorage.setItem("vak.dockTab", next);
  else localStorage.removeItem("vak.dockTab");
}

export interface WorkbenchExecution {
  id: string;
  ownerSessionId?: string;
  tool: string;
  command: string;
  language: string;
  scratchDir: string;
  stdout: string;
  stderr: string;
  outputTruncated?: boolean;
  status: "running" | "completed" | "failed";
  exitCode?: number;
  durationMs?: number;
  memoryBytes?: number;
  cpuPercent?: number;
  packages: string[];
  artifacts: Array<{ path: string; mimeType: string; sizeBytes: number; revision?: number }>;
  timestamp: string;
}

const [sessionWorkbenchMap, setSessionWorkbenchMap] = createSignal<Record<string, WorkbenchExecution[]>>({});

export function workbenchExecutions(): WorkbenchExecution[] {
  const sid = activeId();
  if (!sid) return [];
  return sessionWorkbenchMap()[sid] ?? [];
}

export function setWorkbenchExecutions(
  updater: WorkbenchExecution[] | ((prev: WorkbenchExecution[]) => WorkbenchExecution[])
) {
  const sid = activeId();
  if (!sid) return;
  setWorkbenchExecutionsFor(sid, updater);
}

export function setWorkbenchExecutionsFor(
  sessionId: string,
  updater: WorkbenchExecution[] | ((prev: WorkbenchExecution[]) => WorkbenchExecution[])
) {
  setSessionWorkbenchMap((prev) => {
    const current = prev[sessionId] ?? [];
    const next = typeof updater === "function" ? updater(current) : updater;
    return { ...prev, [sessionId]: next };
  });
}

export const [workbenchLoadError, setWorkbenchLoadError] = createSignal<string | null>(null);
const [activeExecutionId, setActiveExecutionSignal] = createSignal<string | null>(localStorage.getItem("vak.activeExecutionId"));
export { activeExecutionId };
export function setActiveExecutionId(value: string | null) {
  setActiveExecutionSignal(value);
  if (value) localStorage.setItem("vak.activeExecutionId", value);
  else localStorage.removeItem("vak.activeExecutionId");
}

export const [workbenchTab, setWorkbenchTab] = createSignal<"execution" | "artifacts">("execution");
export interface CandidateReviewRequest {
  executionId: string;
  sessionId?: string;
  candidateId?: string;
}
export const [candidateReviewRequest, setCandidateReviewRequest] = createSignal<CandidateReviewRequest | null>(null);

export function openCandidateReview(execId: string, sessionId?: string, candidateId?: string) {
  setActiveExecutionId(execId);
  setWorkbenchTab("execution");
  setCandidateReviewRequest({ executionId: execId, sessionId, candidateId });
  setDockTab("workbench");
}

export function openWorkbenchExecution(execId?: string) {
  setWorkbenchTab("execution");
  if (execId) {
    setActiveExecutionId(execId);
  }
  setDockTab("workbench");
}

export const [requestedArtifact, setRequestedArtifact] = createSignal<string | null>(null);
export function openWorkbenchArtifact(path: string) {
  setWorkbenchTab("artifacts");
  if (path) {
    setRequestedArtifact(path);
  }
  setDockTab("workbench");
}

export function openWorkbenchFolder(_path?: string) {
  setWorkbenchTab("artifacts");
  setDockTab("workbench");
}

export function hydrateWorkbenchExecutions(sessionId: string, events: Array<Record<string, unknown>>, preferLive = false) {
  const rebuilt = new Map<string, WorkbenchExecution>();
  for (const raw of events) {
    const kind = raw.kind as string | undefined;
    const id = raw.execution_id as string | undefined;
    if (!id) continue;
    const current = rebuilt.get(id);
    if (kind === "ExecutionStarted") {
      const started: WorkbenchExecution = {
        id,
        ownerSessionId: typeof raw.owner_session_id === "string" ? raw.owner_session_id : undefined,
        tool: String(raw.tool ?? "bash"),
        command: String(raw.code_preview ?? ""),
        language: String(raw.language ?? "text"),
        scratchDir: String(raw.scratch_dir ?? ""),
        stdout: current?.stdout ?? "",
        stderr: current?.stderr ?? "",
        status: current?.status ?? "running",
        exitCode: current?.exitCode,
        durationMs: current?.durationMs,
        memoryBytes: current?.memoryBytes,
        cpuPercent: current?.cpuPercent,
        outputTruncated: current?.outputTruncated,
        packages: current?.packages ?? [],
        artifacts: current?.artifacts ?? [],
        timestamp: current?.timestamp ?? new Date().toLocaleTimeString(),
      };
      rebuilt.set(id, started);
      continue;
    }
    if (!current) continue;
    if (kind === "Stdout") current.stdout += String(raw.chunk ?? "");
    else if (kind === "Stderr") current.stderr += String(raw.chunk ?? "");
    else if (kind === "OutputTruncated") current.outputTruncated = true;
    else if (kind === "PackageInstalled") {
      for (const packageName of Array.isArray(raw.packages) ? raw.packages.map(String) : []) {
        if (!current.packages.includes(packageName)) current.packages.push(packageName);
      }
    }
    else if (kind === "ArtifactGenerated") {
      const artifact = { path: String(raw.path ?? ""), revision: (current.artifacts.find((item) => item.path === raw.path)?.revision ?? 0) + 1, mimeType: String(raw.mime_type ?? "application/octet-stream"), sizeBytes: Number(raw.size_bytes ?? 0) };
      current.artifacts = [...current.artifacts.filter((item) => item.path !== artifact.path), artifact];
    }
    else if (kind === "ProcessTelemetry") Object.assign(current, { durationMs: Number(raw.elapsed_ms ?? 0), cpuPercent: Number(raw.cpu_percent ?? 0), memoryBytes: Number(raw.memory_bytes ?? 0) });
    else if (kind === "ExecutionFinished") Object.assign(current, { status: Number(raw.exit_code ?? 1) === 0 ? "completed" : "failed", exitCode: Number(raw.exit_code ?? 1), durationMs: Number(raw.duration_ms ?? 0) });
  }
  if (rebuilt.size) setWorkbenchExecutionsFor(sessionId, (prev) => { const merged = new Map(prev.map((item) => [item.id, item])); for (const [id, item] of rebuilt) if (!preferLive || !merged.has(id)) merged.set(id, item); return [...merged.values()]; });
}

/**
 * A Workbench is scoped to the currently selected task, never global app
 * history. The Canvas is scoped the same way, by holding one per conversation.
 */
export function resetWorkbenchExecutions() {
  setWorkbenchLoadError(null);
  setActiveExecutionId(null);
  setRequestedArtifact(null);
}

// ---------------------------------------------------------------------------
// Artifact Canvas — immersive overlay preview (docs/design/61 compliant:
// user-activated only, never auto-opens from tool/sandbox events).
// ---------------------------------------------------------------------------

export type { CanvasMode };
export type CanvasDevice = "desktop" | "tablet" | "mobile";

const [canvasStacks, setCanvasStacks] = createSignal<CanvasStacks>({});
const [canvasDevice, setCanvasDevice] = createSignal<CanvasDevice>("desktop");
export { canvasDevice, setCanvasDevice, canvasStacks, setCanvasStacks };

/** Each conversation has its own Canvas; the one in front is the active
 *  conversation's. "" is the Canvas of no conversation, kept on this device. */
export const canvasConversation = () => activeId() ?? "";

const narrowNow = () => window.matchMedia("(max-width: 1100px)").matches;
export const freshCanvasEntry = (subject: CanvasSubject) => freshEntry(subject, narrowNow());

/** Told about each change that the other surfaces should see (`canvasSync.ts`). */
let shareCanvasChange: (conversation: string, op: CanvasOp) => void = () => {};
export function onCanvasChange(share: (conversation: string, op: CanvasOp) => void) {
  shareCanvasChange = share;
}

/** The one way a Canvas changes: here at once, then for every other surface. */
export function changeCanvas(conversation: string, op: CanvasOp) {
  setCanvasStacks((stacks) => applyOp(stacks, conversation, op, freshCanvasEntry));
  if (conversation && isShared(op)) shareCanvasChange(conversation, op);
}

/** Escape asks the Canvas in front to close; the frame closes it, with its own exit. */
const [canvasCloseRequests, setCanvasCloseRequests] = createSignal(0);
export { canvasCloseRequests };
export function requestCanvasClose() {
  setCanvasCloseRequests((count) => count + 1);
}

/** Set when a reader opens a Canvas that was closed, so it slides in; a Canvas
 *  coming back with its conversation, or opened on another surface, just shows. */
let canvasArrival = false;
export function consumeCanvasArrival(): boolean {
  const arriving = canvasArrival;
  canvasArrival = false;
  return arriving;
}

/** What is open in the active conversation's Canvas, and which is in front. */
export const canvasEntries = () => entriesOf(canvasStacks(), canvasConversation());
export const canvasEntry = () => activeEntry(canvasStacks(), canvasConversation());
export const canvasSubject = (): CanvasSubject | null => canvasEntry()?.subject ?? null;
export const canvasMode = (): CanvasMode => canvasEntry()?.mode ?? "split";

/** Whether the active conversation has a Canvas showing. */
export const canvasOpen = () => canvasEntry() !== null;

/**
 * Open a subject in the active conversation's Canvas, in front of what is
 * already there. User-initiated only. The subject keeps its own identity for
 * reading, so it can belong to another conversation than the one showing it.
 */
export function openArtifactCanvas(subject: CanvasSubject) {
  // On a phone the sidebar is a drawer over the page; the Canvas replaces it.
  if (sidebarOpen() && narrowNow()) setSidebarOpen(false);
  const conversation = canvasConversation();
  if (!activeEntry(canvasStacks(), conversation)) canvasArrival = true;
  changeCanvas(conversation, { op: "open", subject });
}

/** Close the one in front; the last one closed closes the Canvas. */
export function closeArtifactCanvas() {
  const entry = canvasEntry();
  if (entry) closeCanvasEntry(entry.key);
}

/** Closes one tab, in the active conversation's Canvas or the one named. */
export function closeCanvasEntry(key: string, conversation = canvasConversation()) {
  changeCanvas(conversation, { op: "close", key });
}

export function activateCanvasEntry(key: string) {
  changeCanvas(canvasConversation(), { op: "activate", key });
}

/** Changes what the reader has done in a tab (view, selection, note): the one
 *  in front unless named, in the active conversation unless named. */
export function updateCanvasEntry(patch: EntryPatch, key = canvasEntry()?.key, conversation = canvasConversation()) {
  if (key) changeCanvas(conversation, { op: "update", key, patch });
}

/** Toggle between split and focused canvas modes. */
export function toggleCanvasMode() {
  const entry = canvasEntry();
  if (entry) updateCanvasEntry({ mode: entry.mode === "split" ? "focused" : "split" });
}

/** Check whether a path represents a directory or folder. */
export function isDirectoryPath(path: string | null | undefined): boolean {
  if (!path) return false;
  const p = path.trim().replace(/[.,;:!?)]'"`]+$/, "").trim().toLowerCase();
  return (
    p.endsWith("/") ||
    p === ".vak/scratch" ||
    p === ".vak" ||
    p.endsWith("/.vak/scratch") ||
    p === "." ||
    p === ".."
  );
}

/** Check whether a path specifically refers to the sandboxed scratch directory. */
export function isScratchDirectory(path: string | null | undefined): boolean {
  if (!path) return false;
  const p = path.trim().replace(/[.,;:!?)]'"`]+$/, "").trim().toLowerCase();
  return (
    p === ".vak/scratch" ||
    p === ".vak/scratch/" ||
    p.endsWith("/.vak/scratch") ||
    p.endsWith("/.vak/scratch/")
  );
}

/** Determine whether a file path points to an artifact previewable in the Artifact Canvas. */
export function isPreviewableArtifact(path: string | null | undefined): boolean {
  if (!path) return false;
  const p = path.trim().replace(/[.,;:!?)]'"`]+$/, "").trim().toLowerCase();
  if (isDirectoryPath(p)) return false;

  const hasExt =
    p.endsWith(".html") ||
    p.endsWith(".htm") ||
    p.endsWith(".xhtml") ||
    p.endsWith(".svg") ||
    p.endsWith(".pdf") ||
    p.endsWith(".png") ||
    p.endsWith(".jpg") ||
    p.endsWith(".jpeg") ||
    p.endsWith(".gif") ||
    p.endsWith(".webp") ||
    p.endsWith(".ico") ||
    p.endsWith(".bmp") ||
    p.endsWith(".csv") ||
    p.endsWith(".tsv") ||
    isOfficePath(p);

  if (hasExt) return true;

  if (p.includes(".vak/scratch/")) {
    const filename = p.split("/").pop();
    if (filename && filename.includes(".") && !filename.startsWith(".")) {
      return /\.(html?|xhtml|svg|pdf|png|jpe?g|gif|webp|ico|bmp|csv|tsv|json|md|txt)$/i.test(filename);
    }
  }

  return false;
}

/**
 * Open a file in the Canvas. A file named by its origin (a result, a run, a
 * saved version) is read from exactly that origin. A bare reference, such as
 * a path in the assistant's text, is matched to a run only when exactly one
 * run in this conversation left that exact path; if several did, there is no
 * telling which the reader means, so it says so instead of choosing.
 */
export function openArtifactFile(path: string, origin?: ArtifactOrigin) {
  const clean = path.trim().replace(/[.,;:!?)]'"`]+$/, "").trim();
  const title = displayFileName(clean) || "Preview";
  if (origin) {
    openArtifactCanvas(fileSubject(clean, origin, title));
    return;
  }
  const sessionId = activeId() ?? undefined;
  const runs = workbenchExecutions();
  const match = matchExecutionArtifact(clean, runs);
  if (match.kind === "many") {
    setNotice({ kind: "info", text: `${title} was made by more than one run. Open it from the run you mean.` });
    openWorkbenchFolder(clean);
    return;
  }
  const run = match.kind === "one" ? runs.find((candidate) => candidate.id === match.executionId) : undefined;
  const owner = run?.ownerSessionId ?? sessionId;
  openArtifactCanvas(
    fileSubject(clean, run && owner ? { sessionId: owner, executionId: run.id } : { sessionId }, title),
  );
}

/** Opens the view of a cited Office file or PDF at the place it names. */
export function openOfficeCitation(citation: { path: string; anchor: string }) {
  openArtifactFile(citation.path, { sessionId: activeId() ?? undefined, anchor: citation.anchor });
}

/** Opens an owner-only mail citation in the Agent's mail/calendar Canvas. */
export function openMailCalendarCitation(citation: MailCalendarCitationTarget) {
  setPendingMailCalendarCitation(citation);
  openArtifactCanvas({ kind: "daily_mail_calendar", title: "Today", agentId: activeAgentId() });
}

/**
 * Goal mode (docs/design/27 Phase H): an objective + criteria armed for the
 * *next* prompt, consumed once and cleared. `sessionId: null` means armed
 * before any specific run — the composer's goal button, or `/goal` typed
 * with no active task — and applies to whichever session the next prompt
 * resolves to; a non-null `sessionId` (set by `/goal`, which always has a
 * resolved session by the time it runs) scopes it to that one task.
 *
 * This is the ONE place goal state lives. It used to be two: a
 * module-level variable in App.tsx for the `/goal` slash command and a
 * separate signal local to Composer for the sparkle-button form — the same
 * concept, armed and consumed independently, which is exactly the "two
 * ways to do one thing" AGENTS.md invariant 30 rules out.
 */
export interface ArmedGoal {
  objective: string;
  criteria: string[];
  sessionId: string | null;
}
export const [armedGoal, setArmedGoal] = createSignal<ArmedGoal | null>(null);
/** Whether `armedGoal()` currently applies to session `id` (its own
 * scope, or unscoped-and-therefore-universal). */
export function goalAppliesTo(id: string | null): ArmedGoal | null {
  const current = armedGoal();
  if (!current) return null;
  return current.sessionId === null || current.sessionId === id ? current : null;
}
export const [showShortcuts, setShowShortcuts] = createSignal(false);
export const [agentPickerOpen, setAgentPickerOpen] = createSignal(false);
export const [agentPickerTab, setAgentPickerTab] = createSignal<"fleet" | "target">("fleet");
export const [agentCreateOpen, setAgentCreateOpen] = createSignal(false);
/** Bumped after a saved change to the agent list, so the sidebar shows a
 * new, renamed or paused agent at once rather than on its next refresh. */
export const [agentsEpoch, setAgentsEpoch] = createSignal(0);
export const [settingsOpen, setSettingsOpen] = createSignal(false);
/** The in-app "Connect an AI service" sheet (docs/design/75 §6.2). */
export const [connectOpen, setConnectOpen] = createSignal(false);
export const [connectScope, setConnectScope] = createSignal<"project" | "user">("project");
export function openConnect(scope: "project" | "user" = "project") {
  setConnectScope(scope);
  setConnectOpen(true);
}
/** Bumped by anything that can change setup; every `GET /onboarding`
 * reader refetches on it, so the banner and the header agree at once. */
export const [setupEpoch, setSetupEpoch] = createSignal(0);
/** How many empty-conversation greetings are on screen. While one is, it
 * carries the setup card and the app-wide banner stands down. */
export const [greetingsShown, setGreetingsShown] = createSignal(0);
/** Left navigation manages user-wide defaults; the workspace header manages
 * the active project's overlay. The server remains the single source of truth. */
// Defaults to "workspace" (edit the active agent) — that's what someone
// opening Settings almost always wants; editing the shared platform default
// is the deliberate, secondary action.
export const [settingsScope, setSettingsScope] = createSignal<"user" | "workspace">("workspace");
export const [hydratingId, setHydratingId] = createSignal<string | null>(null);
/** Viewport narrow enough that the sidebar and dock are overlays rather
 *  than columns (docs/design/48-web-client.md §7.2). Kept as a signal, not
 *  read ad hoc, so every component agrees about which layout is in force. */
export const [narrowViewport, setNarrowViewport] = createSignal(
  typeof window !== "undefined" && window.matchMedia("(max-width: 900px)").matches,
);
// Open on a desktop, closed on a phone: at that width the sidebar covers
// the transcript, so starting open would greet a phone with a file list
// and no conversation.
export const [sidebarOpen, setSidebarOpen] = createSignal(!narrowViewport());
export const [sidebarWidth, setSidebarWidth] = createSignal(252);
export const [dockWidth, setDockWidth] = createSignal(520);

/**
 * Split view: the session shown in the non-focused pane. `activeId` always
 * names the FOCUSED pane's session (composer, approvals, stop, dock all
 * follow it); `splitFocused` says whether that focus currently sits on the
 * right pane. Pane positions never move — focusing a pane swaps contents.
 */
export const [splitId, setSplitId] = createSignal<string | null>(null);
export const [splitFocused, setSplitFocused] = createSignal(false);
const storedSplitRatio = Number(localStorage.getItem("vak.splitRatio"));
export const [splitRatio, setSplitRatio] = createSignal(
  Number.isFinite(storedSplitRatio) && storedSplitRatio >= 0.25 && storedSplitRatio <= 0.75 ? storedSplitRatio : 0.5,
);
/** Session id rendered in a given pane; focus decides which side is active. */
export function paneSessions(): { left: string | null; right: string | null } {
  if (!splitId()) return { left: activeId(), right: null };
  return splitFocused()
    ? { left: splitId(), right: activeId() }
    : { left: activeId(), right: splitId() };
}
export type Notice = { kind: "error" | "info"; text: string };
export const [notices, setNotices] = createSignal<Notice[]>([]);

export function setNotice(value: Notice | null) {
  if (!value) {
    setNotices([]);
    return;
  }
  setNotices((prev) =>
    prev.some((notice) => notice.kind === value.kind && notice.text === value.text)
      ? prev
      : [...prev.slice(-4), value],
  );
}

export function dismissNotice(index: number) {
  setNotices((prev) => prev.filter((_, i) => i !== index));
}

/**
 * Live retry state per session.
 *
 * The server sends a neutral `"Retrying"` event once per attempt, before the
 * backoff sleep, with no attempt count, delay or reason (those are internal
 * dispatch detail — see `vak-server/src/client_events.rs`). Holding the flag
 * lets the header keep saying "Retrying" until real progress (a delta, a
 * tool call, an end) arrives.
 */
const [retryMap, setRetryMap] = createStore<Record<string, boolean>>({});
export function retryOf(id: string | null): boolean {
  return !!(id && retryMap[id]);
}
function noteRetry(id: string, retrying: boolean) {
  setRetryMap(id, retrying);
}

export interface UiPreferences {
  /** "system" follows the OS/browser, which is the only sane default for
   *  a surface that can be a browser tab on a phone in daylight
   *  (docs/design/48-web-client.md §7.1). */
  theme: "system" | "light" | "dark" | "contrast";
  visualPack: "classic" | "dimensional";
  interfaceFont: InterfaceFont;
  contentFont: ContentFont;
  codeFont: CodeFont;
  textScale: number;
  codeScale: number;
  compactSidebar: boolean;
  suggestions: boolean;
  notifications: boolean;
  quietHours: "off" | "22-07";
  reduceMotion: boolean;
  richPreviews: boolean;
  experimentalSkills: boolean;
  externalMedia: boolean;
  autoplayMedia: boolean;
  /** Narrate turn completions / permission prompts through /voice/speak. */
  voiceEnabled: boolean;
  voiceName: string;
  voicePersona: string;
  /** Short synthesized chime when a turn starts and when it finishes — the
   * audible counterpart to the visual "working" indicators, independent of
   * voice narration (which requires a round-trip to /voice/speak). */
  soundCues: boolean;
}

const defaultUiPreferences: UiPreferences = {
  theme: "system",
  visualPack: "classic",
  interfaceFont: "system",
  contentFont: "inherit",
  codeFont: "system",
  textScale: 100,
  codeScale: 100,
  compactSidebar: false,
  suggestions: true,
  notifications: true,
  quietHours: "off",
  reduceMotion: false,
  richPreviews: true,
  experimentalSkills: false,
  externalMedia: true,
  autoplayMedia: false,
  voiceEnabled: false,
  // Empty means provider default; concrete voices come from /voice/providers.
  voiceName: "",
  voicePersona: "",
  soundCues: true,
};

function loadUiPreferences(): UiPreferences {
  try {
    const stored: UiPreferences = { ...defaultUiPreferences, ...JSON.parse(localStorage.getItem("vak.uiPreferences") ?? "{}") };
    // A theme that no longer exists resolves to Match system (DESIGN.md).
    if (!["system", "light", "dark", "contrast"].includes(stored.theme)) stored.theme = "system";
    if (!["classic", "dimensional"].includes(stored.visualPack)) stored.visualPack = "classic";
    if (!Object.prototype.hasOwnProperty.call(interfaceFonts, stored.interfaceFont)) stored.interfaceFont = "system";
    if (!Object.prototype.hasOwnProperty.call(contentFonts, stored.contentFont)) stored.contentFont = "inherit";
    if (!Object.prototype.hasOwnProperty.call(codeFonts, stored.codeFont)) stored.codeFont = "system";
    stored.textScale = Number.isFinite(stored.textScale) ? Math.max(75, Math.min(125, stored.textScale)) : 100;
    stored.codeScale = Number.isFinite(stored.codeScale) ? Math.max(75, Math.min(125, stored.codeScale)) : 100;
    return stored;
  } catch {
    return defaultUiPreferences;
  }
}

export const [uiPreferences, setUiPreferences] = createStore<UiPreferences>(loadUiPreferences());

export function updateUiPreference<K extends keyof UiPreferences>(key: K, value: UiPreferences[K]) {
  setUiPreferences(key, value);
  localStorage.setItem("vak.uiPreferences", JSON.stringify({ ...uiPreferences, [key]: value }));
}
// File-editor pane target; set from anywhere (chat links, diff headers…).
export const [editorPath, setEditorPath] = createSignal<string | null>(null);
// `/btw` side chat panel.
export const [sideOpen, setSideOpen] = createSignal(false);
// Scheduled-tasks manager modal.
export const [tasksOpen, setTasksOpen] = createSignal(false);
export const [taskFocusId, setTaskFocusId] = createSignal<string | null>(null);
// Cross-project recall search (docs/design/29-personal-os.md P1).
export const [searchOpen, setSearchOpen] = createSignal(false);
// Inbox page (docs/design/29-personal-os.md P6) + live unread total shared by
// the header bell and the sidebar badge.
export const [inboxOpen, setInboxOpen] = createSignal(false);
/** The Library page (plan M8): every deliverable, across conversations. */
export const [libraryOpen, setLibraryOpen] = createSignal(false);
export const [inboxUnread, setInboxUnread] = createSignal(0);
// Feed pipeline modal.
// Settings page to land on when the next open happens (budget banner link).
export type SettingsPageId =
  | "general"
  | "appearance"
  | "voice"
  | "notifications"
  | "connections"
  | "mail-calendar"
  | "social"
  | "privacy"
  | "agent"
  | "models"
  | "reliability"
  | "prompts"
  | "services"
  | "storage"
  | "archived";
export const [pendingSettingsPage, setPendingSettingsPage] = createSignal<SettingsPageId | null>(null);
export type MailCalendarCitationTarget = { accountId: string; threadId: string; messageId: string };
export const [pendingMailCalendarCitation, setPendingMailCalendarCitation] = createSignal<MailCalendarCitationTarget | null>(null);
// Read-only historical transcript viewer (docs/design/29): any session by id,
// served from disk — no attach, no stream, never touches live view state.
export const [transcriptViewId, setTranscriptViewId] = createSignal<string | null>(null);
// Time-travel (checkpoints) modal.
export const [historyOpen, setHistoryOpen] = createSignal(false);
// Dispatch-forensics (receipts) modal.
export const [receiptsOpen, setReceiptsOpen] = createSignal(false);
export const [workOpen, setWorkOpen] = createSignal(false);
// Diff pane binding: which session's changes are shown (best-of-N override).
export const [diffTarget, setDiffTarget] = createSignal<string | null>(null);

export interface BestRun {
  session_id: string;
  branch: string;
  path: string;
}
// Dialog lifecycle: closed → config form → starting ([] pending) → runs.
export const [bestOfOpen, setBestOfOpen] = createSignal(false);
export const [bestOfRuns, setBestOfRuns] = createSignal<BestRun[] | null>(null);

// ---- sound cues --------------------------------------------------------
//
// Two short synthesized tones (WebAudio oscillators, no asset files, no
// network round-trip) so a turn starting/finishing is audible even when the
// user isn't looking at the window. Independent of voice narration below,
// which requires a live backend and a TTS call and is meant for spoken
// summaries, not a reflexive UI cue.

let cueCtx: AudioContext | null = null;

function audioCtx(): AudioContext | null {
  if (!uiPreferences.soundCues) return null;
  try {
    cueCtx ??= new (window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext)();
    if (cueCtx.state === "suspended") void cueCtx.resume();
    return cueCtx;
  } catch {
    return null; // WebAudio unavailable/blocked — cues are best-effort
  }
}

/** A single soft, short sine-wave blip at `freq` Hz, fading in/out to avoid a click. */
function tone(ctx: AudioContext, freq: number, startAt: number, duration = 0.09, gain = 0.05) {
  const osc = ctx.createOscillator();
  const g = ctx.createGain();
  osc.type = "sine";
  osc.frequency.value = freq;
  g.gain.setValueAtTime(0, startAt);
  g.gain.linearRampToValueAtTime(gain, startAt + 0.012);
  g.gain.linearRampToValueAtTime(0, startAt + duration);
  osc.connect(g).connect(ctx.destination);
  osc.start(startAt);
  osc.stop(startAt + duration + 0.02);
}

/** Rising two-note chime: a turn just started working. */
export function cueTurnStart() {
  const ctx = audioCtx();
  if (!ctx) return;
  const t = ctx.currentTime;
  tone(ctx, 523.25, t); // C5
  tone(ctx, 659.25, t + 0.08); // E5
}

/** Falling two-note chime: a turn finished (success). Errors stay silent
 * here — a failed run already surfaces via the error note/banner and
 * shouldn't sound identical to "done". */
export function cueTurnFinish(isError: boolean) {
  const ctx = audioCtx();
  if (!ctx || isError) return;
  const t = ctx.currentTime;
  tone(ctx, 659.25, t); // E5
  tone(ctx, 880, t + 0.08); // A5
}

// ---- voice narration (docs/design: Voice & Personality for vak) ------------

let voiceAudioEl: HTMLAudioElement | null = null;
let voiceObjectUrl: string | null = null;

/** Wired once by the <audio> element App mounts; see App.tsx. */
export function registerVoiceAudioElement(el: HTMLAudioElement | null) {
  voiceAudioEl = el;
}

/**
 * Narrate a short phrase through /voice/speak when voice is enabled. Silent
 * no-op when voice is off, the backend isn't ready, or the call fails --
 * narration is a nice-to-have, never a reason to break the UI.
 */
export async function speak(text: string): Promise<void> {
  if (!uiPreferences.voiceEnabled || !text.trim() || !api.isBackendReady()) return;
  try {
    const blob = await api.speak(text, {
      voiceName: uiPreferences.voiceName,
      persona: uiPreferences.voicePersona,
      sessionId: activeId() ?? undefined,
    });
    const url = URL.createObjectURL(blob);
    if (voiceObjectUrl) URL.revokeObjectURL(voiceObjectUrl);
    voiceObjectUrl = url;
    if (voiceAudioEl) {
      voiceAudioEl.src = url;
      await voiceAudioEl.play();
    }
  } catch (err) {
    console.error("vak: voice narration failed", err);
  }
}

export function openInEditor(path: string) {
  setEditorPath(path);
  setDockTab("editor");
}

const [itemsBySession, setItemsBySession] = createStore<Record<string, Item[]>>({});
const [expandedItems, setExpandedItems] = createStore<Record<string, boolean>>({});
const [runningMap, setRunningMap] = createStore<Record<string, boolean>>({});
/**
 * `POST /cancel` no longer synthesizes a `RunFinished` — the run's own
 * terminal event arrives once it actually stops (which can take a moment:
 * the current tool call or provider request has to unwind). Without this,
 * clicking Stop had nothing to show between the click and that event.
 */
const [stoppingMap, setStoppingMap] = createStore<Record<string, boolean>>({});
const [usageBySession, setUsageBySession] = createStore<Record<string, Usage>>({});
const [presentationBySession, setPresentationBySession] = createStore<Record<string, OutputTimeline | null>>({});
const [presentationErrors, setPresentationErrors] = createStore<Record<string, string | null>>({});

// ---- selectors -------------------------------------------------------------

export function itemsOf(id: string | null, bucket: Bucket = "main"): Item[] {
  if (!id) return [];
  return itemsBySession[K(bucket, id)] ?? [];
}

/** Restore live question cards after reconnecting to a session. */
export function restorePendingQuestions(id: string, questions: api.PendingQuestion[]) {
  const listed = new Set(questions.map((question) => question.id));
  setItemsBySession(id, (items) => {
    const existing = new Set(items.filter((item) => item.kind === "question").map((item) => item.id));
    // The server's list is authoritative: a card it no longer lists was
    // answered in another window or expired.
    const stale = items.some((item) => item.kind === "question" && !item.resolved && !listed.has(item.id));
    const missing = questions.filter((question) => !existing.has(question.id));
    if (!stale && !missing.length) return items;
    const cleared: Item[] = items.map((item) =>
      item.kind === "question" && !item.resolved && !listed.has(item.id) ? { ...item, resolved: "gone" } : item);
    const restored: Item[] = missing.map((question) => ({
      kind: "question",
      id: question.id,
      label: question.label,
      question: question.question,
      options: question.options,
      resolved: null,
    }));
    return [...cleared, ...restored];
  });
}

/** Restore live approval cards after reconnecting to a session. */
export function restorePendingApprovals(id: string, approvals: api.PendingApproval[]) {
  if (!approvals.length) return;
  setItemsBySession(id, (items) => {
    const existing = new Set(items.filter((item) => item.kind === "approval").map((item) => item.id));
    const restored: Item[] = approvals
      .filter((approval) => !existing.has(approval.id))
      .map((approval) => ({
        kind: "approval",
        id: approval.id,
        tool: approval.tool,
        argsJson: approval.args_json,
        reason: approval.reason,
        resolved: null,
      }));
    return [...items, ...restored];
  });
}

export function isRunning(id: string | null, bucket: Bucket = "main"): boolean {
  return !!(id && runningMap[K(bucket, id)]);
}

export function isStopping(id: string | null, bucket: Bucket = "main"): boolean {
  return !!(id && stoppingMap[K(bucket, id)]);
}

export function markStopping(id: string, on: boolean, bucket: Bucket = "main") {
  setStoppingMap(K(bucket, id), on);
}

export function usageOf(id: string | null): Usage {
  return (id && usageBySession[id]) || {};
}

export function presentationOf(id: string | null): OutputTimeline | null {
  return id ? (presentationBySession[id] ?? null) : null;
}

export function presentationErrorOf(id: string | null): string | null {
  return id ? (presentationErrors[id] ?? null) : null;
}

export function itemExpanded(id: string): boolean {
  return !!expandedItems[id];
}

export function toggleItemExpanded(id: string) {
  setExpandedItems(id, !expandedItems[id]);
}

export function hydrateFromPresentation(id: string, timeline: OutputTimeline) {
  if (timeline.schema_version !== 2 || timeline.session_id !== id) throw new Error("Unsupported presentation snapshot");
  setPresentationBySession(id, reconcile(mergePresentationSnapshot(presentationBySession[id] ?? null, timeline), { key: "id" }));
  setPresentationErrors(id, null);
}

export function clearPresentation(id: string) {
  setPresentationBySession(id, null);
}

export function setPresentationError(id: string, error: string | null) {
  setPresentationErrors(id, error);
}

export function applyPresentationEvent(id: string, event: PresentationStreamEvent) {
  // A snapshot frame (initial connect, run settlement, resync) is the sole
  // authority and replaces state wholesale; an ordinary live frame carries
  // only `delta` and is applied to the timeline already on screen (docs/
  // audits Finding 2 -- a snapshot on every live frame previously measured
  // up to 9.3 MB per answer).
  if (event.snapshot) {
    hydrateFromPresentation(id, event.snapshot);
    return;
  }
  if (event.delta) {
    setPresentationBySession(
      id,
      reconcile(applyPresentationDelta(presentationBySession[id] ?? null, event.delta), { key: "id" }),
    );
  }
}

// ---- buckets: "main" transcript vs "side" (/btw) branch --------------------

export type Bucket = "main" | "side";

const K = (bucket: Bucket, id: string) => (bucket === "main" ? id : `side:${id}`);

// ---- immutable list helpers ------------------------------------------------

function updateList(bucket: Bucket, id: string, fn: (list: Item[]) => Item[]) {
  setItemsBySession(K(bucket, id), (list) => fn(list ?? []));
}

function pushItem(bucket: Bucket, id: string, item: Item) {
  updateList(bucket, id, (list) => [...list, item]);
}

function note(bucket: Bucket, id: string, text: string) {
  pushItem(bucket, id, { kind: "system", text });
}

function patchById(bucket: Bucket, id: string, itemId: string, patch: (draft: Item) => Item) {
  updateList(bucket, id, (list) =>
    list.map((it) => {
      if ((it.kind === "tool" || it.kind === "approval") && it.id === itemId) {
        return patch(it);
      }
      return it;
    }),
  );
}

function patchLast(bucket: Bucket, id: string, pred: (it: Item) => boolean, patch: (draft: Item) => Item) {
  updateList(bucket, id, (list) => {
    for (let i = list.length - 1; i >= 0; i--) {
      if (pred(list[i])) {
        const next = [...list];
        next[i] = patch(list[i]);
        return next;
      }
    }
    return list;
  });
}

// ---- transcript hydration --------------------------------------------------

function blocksToItems(blocks: ContentBlock[], keyBase: string): Item[] {
  const out: Item[] = [];
  blocks.forEach((b, i) => {
    if (b.type === "text" && b.text.trim()) {
      out.push({ kind: "assistant", key: `${keyBase}-t${i}`, text: b.text, streaming: false });
    } else if (b.type === "thinking" && b.text.trim()) {
      out.push({ kind: "thinking", key: `${keyBase}-k${i}`, text: b.text, done: true });
    } else if (b.type === "tool_use") {
      out.push({
        kind: "tool",
        id: b.id,
        name: b.name,
        argsJson: JSON.stringify(b.input ?? null, null, 2),
        done: false,
        isError: false,
        preview: null,
      });
    }
    // tool_result blocks fold into their ToolUse card below.
  });
  return out;
}

/**
 * Rebuild chat items from a persisted ledger without writing any store
 * state. Shared by live hydration and the read-only transcript viewer.
 */
export function transcriptToItems(
  id: string,
  messages: Message[],
  entries?: TranscriptEntryMeta[],
  running = false,
): Item[] {
  const next: Item[] = [];
  let assistantSeq = 0;

  for (const [index, m] of messages.entries()) {
    const meta = entries?.[index];
    if (m.role === "User" || m.role === "user" || (typeof m.role === "string" && m.role.toLowerCase() === "user")) {
      // A block that names an attached file to the model is drawn as that
      // file, not as its text (TranscriptEntryMeta.attachments).
      const noteBlocks = new Set((meta?.attachments ?? []).map((file) => file.block));
      const files = (meta?.attachments ?? []).map(({ path, name, bytes }) => ({ path, name, bytes }));
      const texts = m.content
        .filter((b, index): b is Extract<ContentBlock, { type: "text" }> => b.type === "text" && !noteBlocks.has(index))
        .map((b) => b.text);
      const results = m.content.filter(
        (b): b is Extract<ContentBlock, { type: "tool_result" }> => b.type === "tool_result",
      );
      for (const tr of results) {
        const idx = next.findIndex(
          (it) => it.kind === "tool" && it.id === tr.tool_use_id,
        );
        if (idx >= 0) {
          const it = next[idx];
          if (it.kind === "tool") {
            next[idx] = { ...it, done: true, isError: !!tr.is_error, preview: tr.content };
          }
        }
      }
      const joined = stripControlScaffolding(texts.join("\n"));
      const authorPrefix = meta?.author_name ? `${meta.author_name}: ` : "";
      const displayText = authorPrefix && joined.startsWith(authorPrefix) ? joined.slice(authorPrefix.length) : joined;
      if (displayText || files.length) next.push({ kind: "user", text: displayText, entryId: meta?.entry_id, authorId: meta?.author_id ?? undefined, authorName: meta?.author_name ?? undefined, files: files.length ? files : undefined });
    } else {
      const baseKey = `${id}-h${assistantSeq++}`;
      const hasText = m.content.some(
        (b): b is Extract<ContentBlock, { type: "text" }> =>
          b.type === "text" && b.text.trim().length > 0,
      );
      next.push(...blocksToItems(m.content, baseKey));
      if (hasText) {
        for (let i = next.length - 1; i >= 0; i--) {
          const it = next[i];
          if (it.kind === "assistant") {
            next[i] = { ...it, streaming: false };
            break;
          }
        }
      }
    }
  }
  // During reconnect, the read-only transcript may be a prefix of an active
  // run. Keep unresolved calls visibly in flight; after settlement, an absent
  // result means its end event was lost and the card can be closed.
  for (let i = 0; i < next.length; i++) {
    const it = next[i];
    if (!running && it.kind === "tool" && !it.done) {
      next[i] = { ...it, done: true, preview: it.preview ?? null };
    }
    if (!running && it.kind === "approval" && it.resolved === null) {
      next[i] = { ...it, resolved: "gone" };
    }
  }
  return next;
}

/** Rebuild a session view from the persisted ledger. */
/** A transcript is append-only, so an item at the same position with the
 * same content is the same item: keep the object already on screen, and the
 * row showing it stays mounted instead of being rebuilt under the reader. */
function keepUnchanged(current: Item[], incoming: Item[]): Item[] {
  return incoming.map((item, index) => {
    const previous = current[index];
    return previous && JSON.stringify(unwrap(previous)) === JSON.stringify(item) ? previous : item;
  });
}

export function hydrateFromTranscript(id: string, messages: Message[], entries?: TranscriptEntryMeta[], running = false) {
  const current: Item[] = itemsBySession[id] ?? [];
  const incoming = keepUnchanged(current, transcriptToItems(id, messages, entries, running));

  if (current.length === 0) {
    setItemsBySession(id, incoming);
    return;
  }

  // Count user messages in incoming
  const incomingUserCount = incoming.filter((it) => it.kind === "user").length;

  // Find where unpersisted user messages or trailing notes begin in current
  let currentUserCount = 0;
  let unpersistedStartIndex = -1;
  for (let i = 0; i < current.length; i++) {
    if (current[i].kind === "user") {
      currentUserCount++;
      if (currentUserCount > incomingUserCount && unpersistedStartIndex === -1) {
        unpersistedStartIndex = i;
      }
    }
  }

  if (unpersistedStartIndex !== -1) {
    const tail = current.slice(unpersistedStartIndex).map((it: Item) => {
      if (it.kind === "assistant" && it.streaming) return { ...it, streaming: false };
      return it;
    });
    setItemsBySession(id, [...incoming, ...tail]);
    return;
  }

  // Preserve any trailing system note/error that happened after the last user turn
  const lastCurrent = current[current.length - 1];
  if (
    lastCurrent?.kind === "system" &&
    incoming.length > 0 &&
    incoming[incoming.length - 1]?.kind !== "system"
  ) {
    setItemsBySession(id, [...incoming, lastCurrent]);
    return;
  }

  setItemsBySession(id, incoming);
}

// ---- live event application ------------------------------------------------

let assistantSeq = 0;

function ensureStreamingAssistant(bucket: Bucket, id: string): void {
  updateList(bucket, id, (list) => {
    const last = list[list.length - 1];
    if (last?.kind === "assistant" && last.streaming) return list;
    assistantSeq += 1;
    return [
      ...list,
      { kind: "assistant", key: `live-${assistantSeq}`, text: "", streaming: true },
    ];
  });
}

function appendToLast(bucket: Bucket, id: string, kind: "assistant" | "thinking", delta: string) {
  updateList(bucket, id, (list) => {
    const last = list[list.length - 1];
    if (last?.kind === kind) {
      const next = [...list];
      next[next.length - 1] =
        kind === "assistant"
          ? { ...last, kind, key: last.key, text: last.text + delta, streaming: true }
          : { ...last, kind, key: last.key, text: last.text + delta, done: false };
      return next;
    }
    const fresh: Item =
      kind === "assistant"
        ? { kind: "assistant", key: `live-${++assistantSeq}`, text: delta, streaming: true }
        : { kind: "thinking", key: `think-${Date.now()}`, text: delta, done: false };
    return [...list, fresh];
  });
}

export function applyEvent(
  id: string,
  ev: ClientEvent,
  opts: {
    onFinish?: (message: string) => void;
    /** A gate is now waiting on a person. The only event in the stream
     *  that is *about* the reader rather than the work. */
    onApproval?: (requestId: string, tool: string) => void;
    bucket?: Bucket;
  },
) {
  const b: Bucket = opts.bucket ?? "main";
  // Rust's externally tagged unit variant serializes as the bare string
  // (`"StreamOpened"`, `"Retrying"`), while object variants serialize as
  // `{ Variant: ... }`.
  if (ev === "StreamOpened") return;
  if (ev === "Retrying") {
    // No attempt count, delay or reason: the server never sends them (see
    // `ClientEvent::Retrying` in vak-server/src/client_events.rs). The
    // header shows a neutral "Retrying" state until real progress arrives.
    noteRetry(id, true);
    return;
  }
  if ("TurnStart" in ev) {
    markRunning(id, true, b);
    cueTurnStart();
    noteRetry(id, false);
  } else if ("TextDelta" in ev) {
    noteRetry(id, false);
    ensureStreamingAssistant(b, id);
    appendToLast(b, id, "assistant", ev.TextDelta.delta);
  } else if ("ThinkingDelta" in ev) {
    noteRetry(id, false);
    appendToLast(b, id, "thinking", ev.ThinkingDelta.delta);
  } else if ("ToolCallStart" in ev) {
    noteRetry(id, false);
    pushItem(b, id, {
      kind: "tool",
      id: ev.ToolCallStart.id,
      name: ev.ToolCallStart.name,
      argsJson: ev.ToolCallStart.args_json,
      done: false,
      isError: false,
      preview: null,
    });
  } else if ("ToolCallEnd" in ev) {
    patchById(b, id, ev.ToolCallEnd.id, (it) =>
      it.kind === "tool"
        ? {
            ...it,
            done: true,
            isError: ev.ToolCallEnd.is_error,
            preview: ev.ToolCallEnd.result_preview,
          }
        : it,
    );
  } else if ("ApprovalRequested" in ev) {
    updateList("main", id, (items) => items.some((item) => item.kind === "approval" && item.id === ev.ApprovalRequested.id)
      ? items
      : [...items, {
          kind: "approval",
          id: ev.ApprovalRequested.id,
          tool: ev.ApprovalRequested.tool,
          argsJson: ev.ApprovalRequested.args_json,
          reason: ev.ApprovalRequested.reason,
          resolved: null,
        }]);
    opts.onApproval?.(ev.ApprovalRequested.id, ev.ApprovalRequested.tool);
  } else if ("WorkerQuestion" in ev) {
    updateList("main", id, (items) => items.some((item) => item.kind === "question" && item.id === ev.WorkerQuestion.id)
      ? items
      : [...items, {
          kind: "question",
          id: ev.WorkerQuestion.id,
          label: ev.WorkerQuestion.label,
          question: ev.WorkerQuestion.question,
          options: ev.WorkerQuestion.options,
          resolved: null,
        }]);
  } else if ("WorkerQuestionAnswered" in ev) {
    updateList("main", id, (items) => items.map((item) =>
      item.kind === "question" && item.id === ev.WorkerQuestionAnswered.id ? { ...item, resolved: "answered" } : item));
  } else if ("WorkerQuestionClosed" in ev) {
    updateList("main", id, (items) => items.map((item) =>
      item.kind === "question" && item.id === ev.WorkerQuestionClosed.id && !item.resolved ? { ...item, resolved: "gone" } : item));
  } else if ("WorkerStarted" in ev) {
    pushItem(b, id, {
      kind: "worker",
      label: ev.WorkerStarted.label,
      lines: [],
      open: false,
      isError: false,
    });
  } else if ("WorkerToolCall" in ev) {
    patchLast(
      b,
      id,
      (it) => it.kind === "worker",
      (it) => {
        if (it.kind !== "worker") return it;
        const lines = [...it.lines, `${ev.WorkerToolCall.name}${ev.WorkerToolCall.is_error ? " ✗" : ""}`];
        return { ...it, lines: lines.slice(-12) };
      },
    );
  } else if ("WorkerFinished" in ev) {
    patchLast(
      b,
      id,
      (it) => it.kind === "worker",
      (it) =>
        it.kind === "worker"
          ? {
              ...it,
              isError: ev.WorkerFinished.is_error,
              lines: [...it.lines, `done in ${(ev.WorkerFinished.elapsed_ms / 1000).toFixed(1)}s`],
            }
          : it,
    );
  } else if ("DraftDiscarded" in ev) {
    // The runtime sent this turn's text answer back for a redo. Drop the
    // discarded draft bubble silently -- no note, it was never a finished
    // answer the reader should see.
    updateList(b, id, (list) => {
      const idx = [...list].reverse().findIndex((it) => it.kind === "assistant");
      if (idx === -1) return list;
      const at = list.length - 1 - idx;
      return [...list.slice(0, at), ...list.slice(at + 1)];
    });
  } else if ("RunFinished" in ev) {
    markRunning(id, false, b);
    noteRetry(id, false);
    // Close every in-flight item. A finished run cannot still have a tool
    // executing or an approval pending, so anything left open means its
    // end event was missed (a dropped stream, a failed re-attach). Leaving
    // it open strands the card on "running…" for the rest of the session.
    updateList(b, id, (list) =>
      list.map((it) => {
        if (it.kind === "assistant" && it.streaming) return { ...it, streaming: false };
        if (it.kind === "thinking" && !it.done) return { ...it, done: true };
        if (it.kind === "tool" && !it.done) {
          return {
            ...it,
            done: true,
            preview: it.preview ?? null,
          };
        }
        if (it.kind === "approval" && it.resolved === null) {
          return { ...it, resolved: "gone" as const };
        }
        return it;
      }),
    );
    // A run that did not complete normally must say so in the transcript.
    // `message` is always one of the server's small set of human sentences
    // (never raw error text — see `ClientEvent.RunFinished`).
    const { outcome, message } = ev.RunFinished;
    const badOutcome = outcome === "Failed" || outcome === "MaxTurns";
    if (badOutcome) {
      note(b, id, message);
    }
    // Short narration only -- the full message can run long and reads
    // awkwardly aloud; a one-word cue is enough to signal completion.
    // Voice conversations own playback. Turn completion is surfaced in the
    // transcript and must not trigger an unscoped one-shot narration request.
    cueTurnFinish(badOutcome);
    opts.onFinish?.(message);
  } else if ("Sandbox" in ev) {
    const sb = ev.Sandbox;
    if (sb.kind === "ExecutionStarted") {
      const execId = sb.execution_id;
      const newExec: WorkbenchExecution = {
        id: execId,
        ownerSessionId: sb.owner_session_id ?? undefined,
        tool: sb.tool,
        command: sb.code_preview,
        language: sb.language,
        scratchDir: sb.scratch_dir,
        stdout: "",
        stderr: "",
        status: "running",
        packages: [],
        artifacts: [],
        timestamp: new Date().toLocaleTimeString(),
      };
      setWorkbenchExecutionsFor(id, (prev) => {
        const existing = prev.find((item) => item.id === execId);
        if (!existing) return [...prev, newExec];
        // Re-attachment can replay ExecutionStarted after hydration. Keep
        // accumulated output/artifacts while refreshing lifecycle metadata.
        return prev.map((item) => item.id === execId ? {
          ...item,
          ownerSessionId: newExec.ownerSessionId ?? item.ownerSessionId,
          tool: newExec.tool,
          command: newExec.command,
          language: newExec.language,
          scratchDir: newExec.scratchDir,
          status: item.status === "completed" || item.status === "failed" ? item.status : "running",
        } : item);
      });
      if (id === activeId()) {
        setActiveExecutionId(execId);
      }
    } else if (sb.kind === "Stdout") {
      const execId = sb.execution_id;
      if (execId) {
        setWorkbenchExecutionsFor(id, (prev) =>
          prev.map((e) => (e.id === execId ? { ...e, stdout: e.stdout + sb.chunk } : e))
        );
      }
    } else if (sb.kind === "Stderr") {
      const execId = sb.execution_id;
      if (execId) {
        setWorkbenchExecutionsFor(id, (prev) =>
          prev.map((e) => (e.id === execId ? { ...e, stderr: e.stderr + sb.chunk } : e))
        );
      }
    } else if (sb.kind === "OutputTruncated") {
      const execId = sb.execution_id;
      if (execId) setWorkbenchExecutionsFor(id, (prev) => prev.map((e) => e.id === execId ? { ...e, outputTruncated: true } : e));
    } else if (sb.kind === "PackageInstalled") {
      const execId = sb.execution_id;
      if (execId) {
        setWorkbenchExecutionsFor(id, (prev) =>
          prev.map((e) => (e.id === execId ? { ...e, packages: [...e.packages, ...sb.packages] } : e))
        );
      }
    } else if (sb.kind === "ArtifactGenerated") {
      const execId = sb.execution_id;
      if (execId) {
        // A deliverable is the primary outcome of execution. Surface the
        // result pane as soon as one exists; activity remains one click away.
        setWorkbenchExecutionsFor(id, (prev) =>
          prev.map((e) =>
            e.id === execId
              ? {
                  ...e,
                  artifacts: [...e.artifacts.filter((artifact) => artifact.path !== sb.path), {
                    path: sb.path, mimeType: sb.mime_type, sizeBytes: sb.size_bytes,
                    revision: (e.artifacts.find((artifact) => artifact.path === sb.path)?.revision ?? 0) + 1,
                  }],
                }
              : e
          )
        );
      }
    } else if (sb.kind === "ProcessTelemetry") {
      const execId = sb.execution_id;
      if (execId) {
        setWorkbenchExecutionsFor(id, (prev) =>
          prev.map((e) =>
            e.id === execId
              ? {
                  ...e,
                  durationMs: sb.elapsed_ms,
                  memoryBytes: sb.memory_bytes,
                  cpuPercent: sb.cpu_percent,
                }
              : e
          )
        );
      }
    } else if (sb.kind === "ExecutionFinished") {
      const execId = sb.execution_id;
      if (execId) {
        setWorkbenchExecutionsFor(id, (prev) =>
          prev.map((e) =>
            e.id === execId
              ? {
                  ...e,
                  status: sb.exit_code === 0 ? "completed" : "failed",
                  exitCode: sb.exit_code,
                  durationMs: sb.duration_ms,
                }
              : e
          )
        );
      }
    }
  }
}

// ---- imperative helpers used by App ---------------------------------------

export function appendUser(id: string, text: string, files?: api.InboxFile[], bucket: Bucket = "main") {
  pushItem(bucket, id, { kind: "user", text, files: files?.length ? files : undefined });
  if (bucket === "main") setLastSubmittedPrompt({ sessionId: id, sequence: ++submittedPromptSequence });
}

export function appendSystem(id: string, text: string, bucket: Bucket = "main") {
  note(bucket, id, text);
}

/** A message could not start because no AI service is connected
 * (the server's typed `no_ai_service` refusal). */
export function appendNeedsAiService(id: string, bucket: Bucket = "main") {
  pushItem(bucket, id, { kind: "system", text: "No AI service is connected yet, so this message wasn't sent.", needs: "ai-service" });
}

export function markRunning(id: string, on: boolean, bucket: Bucket = "main") {
  setRunningMap(K(bucket, id), on);
  // A session that just stopped running (cancelled, or reconciled against
  // the server's own truth in refreshSessions) must not keep showing a
  // stale "Retrying" state from whatever it was last doing.
  if (!on) {
    if (armedStopSession() === id) setArmedStopSession(null);
    noteRetry(id, false);
    markStopping(id, false, bucket);
  }
}

export function resolveApproval(id: string, requestId: string, verdict: "allowed" | "denied") {
  patchById(
    "main",
    id,
    requestId,
    (it): Item =>
      it.kind === "approval" ? { ...it, resolved: verdict } : it,
  );
}

export function setUsageFor(id: string, usage: Usage | Record<string, number>) {
  setUsageBySession(id, usage as Usage);
}

// ---- session registry ------------------------------------------------------

export function resetSessionView(id: string) {
  for (const key of [K("main", id), K("side", id)]) {
    setItemsBySession(key, []);
    setRunningMap(key, false);
  }
  setPresentationBySession(id, { schema_version: 2, session_id: id, items: [], diagnostics: [], goal: null });
  setPresentationErrors(id, null);
}
