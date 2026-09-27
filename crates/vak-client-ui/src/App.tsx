import { createEffect, createMemo, createSignal, lazy, onCleanup, onMount, Show, For, Suspense } from "solid-js";
import { host } from "./host";
import { interfaceFonts, contentFonts, codeFonts } from "./typography";
import { watchSession, watchStatus } from "./streamHub";
import {
  activeId,
  appendSystem,
  appendNeedsAiService,
  sessions,
  appendUser,
  applyEvent,
  applyPresentationEvent,
  backend,
  isRunning,
  markRunning,
  markStopping,
  resolveApproval,
  setActiveId,
  activeAgentId,
  setActiveAgent,
  setAgentOpening,
  setOpeningAgentId,
  setReplyTarget,
  setBackend,
  setDockTab,
  setEditorPath,
  setDiffTarget,
  setHealth,
  setShowShortcuts,
  setSessions,
  setCoworkingPresence,
  setUsageFor,
  hydrateFromTranscript,
  hydrateFromPresentation,
  itemsOf,
  presentationOf,
  hydrateWorkbenchExecutions,
  setWorkbenchLoadError,
  resetWorkbenchExecutions,
  clearPresentation,
  setPresentationError,
  dockTab,
  diffTarget,
  showShortcuts,
  setSideOpen,
  sideOpen,
  bestOfOpen,
  dockWidth,
  setDockWidth,
  setHydratingId,
  setSidebarOpen,
  setSidebarWidth,
  narrowViewport,
  setNarrowViewport,
  sidebarOpen,
  sidebarWidth,
  setNotice,
  setProviders,
  historyOpen,
  setHistoryOpen,
   receiptsOpen,
  setWorkOpen,
  setSearchOpen,
  settingsOpen,
  greetingsShown,
  setSettingsOpen,
  setAgentPickerOpen,
  setAgentPickerTab,
  inboxOpen,
  setInboxOpen,
  transcriptViewId,
  setTranscriptViewId,
  uiPreferences,
  workspaceSwitching,
  setWorkspaceSwitching,
  splitId,
  setSplitId,
  splitFocused,
  setSplitFocused,
  splitRatio,
  setSplitRatio,
  paneSessions,
  registerVoiceAudioElement,
  setConnection,
  setArmedGoal,
  goalAppliesTo,
  openWorkbenchFolder,
  isScratchDirectory,
  isDirectoryPath,
  canvasOpen,
  canvasMode,
  type ReplyTarget,
} from "./store";
import { recordAgentOpened } from "./agentRecents";
import type { SessionSummary } from "./types";
import * as api from "./api";
import Sidebar from "./components/Sidebar";
import ChatPane from "./components/ChatPane";
import Composer from "./components/Composer";
import StatusBar from "./components/StatusBar";
const DiffPane = lazy(() => import("./components/DiffPane"));
const TerminalPane = lazy(() => import("./components/TerminalPane"));
const EditorPane = lazy(() => import("./components/EditorPane"));
import ShortcutsModal from "./components/ShortcutsModal";
import SideChatPanel from "./components/SideChatPanel";
import BestOfNDialog from "./components/BestOfNDialog";
import PrPanel from "./components/PrPanel";
import TasksModal from "./components/TasksModal";
import CheckpointsModal from "./components/CheckpointsModal";
import ReceiptsModal from "./components/ReceiptsModal";
import WorkModal from "./components/WorkModal";
const PreviewPane = lazy(() => import("./components/PreviewPane"));
const ArtifactCanvas = lazy(() => import("./components/ArtifactCanvas"));
const WorkbenchPanel = lazy(() => import("./components/WorkbenchPanel"));
const WorkersPanel = lazy(() => import("./components/WorkersPanel"));
const CommitmentsPanel = lazy(() => import("./components/CommitmentsPanel"));
import WorkspaceGate from "./components/WorkspaceGate";
import WorkspaceHeader from "./components/WorkspaceHeader";
import Icon, { type IconName } from "./components/Icon";
import ResizeHandle from "./components/ResizeHandle";
import Toast from "./components/Toast";
const Settings = lazy(() => import("./components/Settings"));
import BudgetBanner from "./components/BudgetBanner";
import SearchModal from "./components/SearchModal";
const FeedsPanel = lazy(() => import("./components/FeedsPanel"));
import FeedsModal from "./components/FeedsModal";
import SetupBanner from "./components/SetupBanner";
import TranscriptModal from "./components/TranscriptModal";
import InboxPage from "./components/InboxPage";
import AgentPickerModal from "./components/AgentPickerModal";
import AgentCreateWizard from "./components/AgentCreateWizard";
import ConnectSheet from "./components/ConnectSheet";
import { closeOpenMenus, dismissMenusOnPressOutside } from "./menus";
import { titleBarGestures } from "./titleBar";
import Skeleton from "./components/Skeleton";

/** Unsubscribe handles for the sessions this tab follows (streamHub.ts). */
const streams = new Map<string, () => void>();
const sideStreams = new Map<string, () => void>();

export async function refreshSessions() {
  const source = api.backendUrl();
  // Web clients authenticate with the same-origin session cookie, so their
  // backend URL is intentionally empty. Only skip when the backend is not
  // ready at all (the desktop host has no live server yet).
  if (!api.isBackendReady()) return;
  try {
    const res = await api.listSessions();
    if (source === api.backendUrl()) {
      setSessions((prev) => {
        const active = prev.find((s) => s.session_id === activeId());
        const next = active && !res.sessions.some((s) => s.session_id === active.session_id) ? [active, ...res.sessions] : res.sessions;
        // An unchanged list keeps its objects, so lists drawn from it (the
        // greeting's recent results) are not rebuilt on every refresh.
        return JSON.stringify(next) === JSON.stringify(prev) ? prev : next;
      });
      // Startup opens the canonical Vakyartha conversation through agent admission.
      // Session refresh itself never chooses an arbitrary recent task.
      // The other pane's session was deleted elsewhere — collapse the split
      // rather than showing a ghost.
      const other = splitId();
      if (
        other &&
        res.sessions.length > 0 &&
        !res.sessions.some((s) => s.session_id === other)
      ) {
        closeSplit();
      }
      // Reconcile "running" against the server's own truth. The push
      // path (openStream -> RunFinished) can miss a completion for
      // reasons that are not bugs to chase individually: the
      // subscription is registered but the stream is still
      // establishing when the run finishes, the tab was backgrounded,
      // the stream dropped and hasn't reconnected yet -- in every case
      // the server-side broadcast channel does not replay history to a
      // late or reconnecting subscriber, so a missed RunFinished is
      // gone, not delayed. Without this, a session the client still
      // thinks is running never revisits that belief on its own: the
      // header stays on "Working" and hydrate()'s own guard (`if
      // (!isRunning(id))`) refuses to load the transcript that already
      // has the reply, indefinitely -- previously recoverable only by
      // relaunching the whole app. This runs every 10s regardless of
      // whether anything is being viewed, so the correction lands
      // without the user needing to do anything.
      const visible = new Set([activeId(), splitId()].filter((x): x is string => !!x));
      await Promise.all([...visible].map(async (sessionId) => {
        try {
          const presence = await api.coworkingPresence(sessionId);
          setCoworkingPresence(sessionId, presence.participants ?? []);
        } catch {
          /* presence is optional and never replaces the last observed state */
        }
      }));
      for (const s of res.sessions) {
        if (s.running) {
          if (!isRunning(s.session_id)) markRunning(s.session_id, true);
          continue;
        }
        if (!s.running && isRunning(s.session_id)) {
          markRunning(s.session_id, false);
          if (visible.has(s.session_id)) {
            await hydrate(s.session_id, true);
          }
        } else if (!s.running && visible.has(s.session_id)) {
          // Shared-human messages are append-only ledger writes rather than
          // Agent events. Reconcile visible settled conversations on the
          // existing session heartbeat so an owner's open transcript picks
          // them up without another subscription on the shared stream.
          await hydrate(s.session_id, true);
        }
      }
      pruneStreams();
    }
  } catch {
    /* backend restarting */
  }
}

export async function retryHydrate(id: string) {
  await hydrate(id);
}

async function settledTranscript(id: string): ReturnType<typeof api.transcript> {
  for (let attempt = 0; attempt < 8; attempt += 1) {
    try {
      const transcript = await api.transcript(id);
      if (!Array.isArray(transcript.messages)) throw new Error("Transcript response has no messages");
      return transcript;
    } catch (error) {
      if (!(error instanceof Error) || error.message !== "run in progress" || attempt === 7) throw error;
      await new Promise((resolve) => window.setTimeout(resolve, 200));
    }
  }
  throw new Error("Transcript is not ready");
}

/** What `hydrate` last applied per conversation. The session heartbeat
 * re-reads every visible settled conversation every 10 s; applying an
 * identical read rebuilt every turn (and reloaded every result preview),
 * which read as the page refreshing. */
const lastHydrated = new Map<string, string>();

/** Load a conversation's durable record. `background` re-reads (the session
 * heartbeat, a stream resync) never raise the loading state: an empty
 * conversation would swap its greeting for the loading skeleton and back
 * every 10 s. */
async function hydrate(id: string, background = false) {
  if (!background) setHydratingId(id);
  try {
    const [t, presentation, sandbox] = await Promise.all([
      isRunning(id) ? Promise.resolve(undefined) : settledTranscript(id),
      // A presentation snapshot is an optional projection. If it cannot be
      // read, preserve the last known projection rather than treating a
      // transient/permission error as an authoritative empty result.
      api.presentation(id).catch((error) => {
        setPresentationError(id, error instanceof Error ? error.message : String(error));
        return undefined;
      }),
      api.sandboxExecutions(id).catch((error) => ({ events: [], session_id: id, error: error instanceof Error ? error.message : String(error) })),
    ]);
    const read = JSON.stringify([t ?? null, presentation ?? null, sandbox]);
    const unchanged = lastHydrated.get(id) === read && itemsOf(id).length > 0 && (!presentation || presentationOf(id) !== null);
    if (unchanged) return;
    lastHydrated.set(id, read);
    if (!isRunning(id) && t) {
      hydrateFromTranscript(id, t.messages, t.entries);
      if (presentation) hydrateFromPresentation(id, presentation);
      setUsageFor(id, t.usage);
    }
    if (activeId() === id) {
      setWorkbenchLoadError("error" in sandbox ? sandbox.error : null);
    }
    hydrateWorkbenchExecutions(id, sandbox.events, isRunning(id));
  } catch (error) {
    // The server can admit a new turn between a settled-session snapshot and
    // transcript fetch. Its 409 is an active run, not a load failure.
    if (!isRunning(id) && !(error instanceof Error && error.message === "run in progress")) {
      appendSystem(id, `Could not load this task: ${error instanceof Error ? error.message : String(error)}`);
    }
  } finally {
    if (!background) setHydratingId((current) => (current === id ? null : current));
  }
}

const resyncingSessions = new Set<string>();

/**
 * Stop following sessions that are no longer visible.
 *
 * Every subscription rides the one shared stream (streamHub.ts), so this is
 * no longer what keeps the browser's six-connections-per-host pool free; it
 * keeps the server from projecting events nobody is looking at.
 */
function pruneStreams() {
  const visible = new Set([activeId(), splitId()].filter((x): x is string => !!x));
  for (const followed of [streams, sideStreams]) {
    for (const [id, stop] of followed) {
      if (visible.has(id)) continue;
      stop();
      followed.delete(id);
    }
  }
}

function openStream(id: string) {
  if (streams.has(id)) return;
  streams.set(id, watchSession(id, {
    agent: (ev) =>
      applyEvent(id, ev, {
        onFinish: (s) => onFinished(id, s),
        onApproval: (requestId, tool) => onApprovalRequested(id, requestId, tool),
      }),
    presentation: (frame) => applyPresentationEvent(id, frame),
    resync: () => {
      // The gap was wider than the server's replay ring, so what is on
      // screen may be missing events it cannot know about. Rebuild from the
      // durable transcript, which is the only complete record.
      resyncingSessions.add(id);
      setConnection("resyncing");
      // A running turn cannot be hydrated safely: its durable transcript is
      // intentionally incomplete until RunFinished. Keep the explicit
      // resync state until that terminal event, then hydrate the durable
      // record and only afterward report live again.
      if (!isRunning(id)) {
        void hydrate(id, true).finally(() => {
          resyncingSessions.delete(id);
          setConnection("live");
        });
      }
    },
  }));
}

const NOTIFY_DEDUPE_MS = 5 * 60 * 1000;
const lastNotifyAt = new Map<string, number>();
const pendingApprovals = new Set<string>();
export const isApprovalPending = (requestId: string) => pendingApprovals.has(requestId);

/** Native notification with per-source 5-minute dedupe (desktop round 2). */
export async function notifyOnce(source: string, title: string, body: string, route?: string) {
  const now = Date.now();
  if (now - (lastNotifyAt.get(source) ?? 0) < NOTIFY_DEDUPE_MS) return;
  lastNotifyAt.set(source, now);
  if (route) pendingRoute = route;
  await notify(title, body, source.startsWith("approval:"));
}

/** Where a notification click should land. Consumed by `applyRoute`. */
let pendingRoute: string | null = null;

/**
 * Point the client at a session, and optionally at one approval inside it.
 *
 * Used by the hash route on load (a bookmark, or a notification click that
 * reopened the tab) and by a notification arriving in a live tab.
 */
export async function applyRoute(hash: string) {
  const match = /^#\/s\/([^?]+)(?:\?(.*))?$/.exec(hash);
  if (!match) return;
  const [, sessionId, query] = match;
  if (sessionId === activeId()) {
    // Already shown: on load `openAgentChat` has just activated it and set
    // this route. Activating again would attach and hydrate it twice.
    setInboxOpen(false);
    if (narrowViewport()) setSidebarOpen(false);
  } else {
    if (!sessions().some((s) => s.session_id === sessionId)) await refreshSessions();
    await activate(sessionId);
  }
  const approval = new URLSearchParams(query ?? "").get("approval");
  if (!approval) return;
  // Scroll the card into view once it has actually rendered — the
  // transcript hydrates asynchronously, so the element does not exist yet
  // at the moment the route is applied.
  requestAnimationFrame(() => {
    document
      .querySelector(`[data-approval="${CSS.escape(approval)}"]`)
      ?.scrollIntoView({ behavior: "smooth", block: "center" });
  });
}

async function notify(title: string, body: string, critical = false) {
  if (!uiPreferences.notifications) return;
  const hour = new Date().getHours();
  const quiet = uiPreferences.quietHours === "22-07" && (hour >= 22 || hour < 7);
  if (quiet && !critical) return;
  const route = pendingRoute;
  pendingRoute = null;
  // OS notification centre on the desktop, the Web Notifications API in a
  // browser — the host knows which, and both are best-effort.
  await host.notify(title, body, route ?? undefined);
}

/**
 * A run has stopped and is waiting on a person.
 *
 * This is the one notification that earns an interruption: everything else
 * in the product can be read later, and an approval gate cannot — the work
 * is stopped until someone answers. So it notifies whenever the window is
 * not focused, not merely when the tab is hidden, and the notification
 * carries a deep link straight to the card rather than to the app.
 */
function onApprovalRequested(id: string, requestId: string, tool: string) {
  if (document.hasFocus() && id === activeId()) return;
  const session = sessions().find((s) => s.session_id === id);
  void notifyOnce(
    `approval:${requestId}`,
    "Vakyartha needs your approval",
    `${session?.title ?? "A task"} wants to run ${tool}.`,
    `#/s/${id}?approval=${encodeURIComponent(requestId)}`,
  );
}

function onFinished(id: string, summary: string) {
  // Keep the live transcript visible until the reconnectable snapshot has
  // arrived. Clearing the presentation here created a blank/legacy flash
  // between RunFinished and hydrate(), which was especially noticeable on
  // the first turn of a task.
  // Enter the hydration state before flipping the run flag so an older
  // settled snapshot cannot flash over the just-finished live transcript.
  setHydratingId(id);
  markRunning(id, false);
  pruneStreams();
  void Promise.all([refreshSessions(), hydrate(id)]).finally(() => {
    if (resyncingSessions.delete(id)) setConnection("live");
  });
  if (document.hidden && id === activeId()) {
    const s = sessions().find((x) => x.session_id === id);
    void notify("Vakyartha run finished", `${s?.title ?? "Session"} — ${summary}`);
  }
}

export async function activate(id: string) {
  // Choosing a task always lands back in the workspace view.
  setInboxOpen(false);
  // On a narrow viewport the sidebar is an overlay covering the very
  // transcript the tap asked for, so choosing dismisses it.
  if (narrowViewport()) setSidebarOpen(false);
  // Clicking the session already shown in the other pane focuses it there
  // instead of duplicating it across both panes.
  if (splitId() && id === splitId()) {
    swapPanes();
    return;
  }
  setActiveId(id);
  const matchedAgent = sessions().find((session) => session.session_id === id)?.agent;
  if (matchedAgent) {
    setActiveAgent(matchedAgent);
  }
  setReplyTarget(null);
  // Do not let execution/artifact state from the previously selected task
  // bleed into this task while its durable sidecar is loading.
  resetWorkbenchExecutions(id);
  if (sessions().find((session) => session.session_id === id)?.running) {
    markRunning(id, true);
  }
  pruneStreams();
  // Persisted sessions must be attached server-side before they can accept
  // runs; for live sessions the server keeps the existing handle (idempotent).
  try {
    await api.attachSession(id);
  } catch (e) {
    appendSystem(id, `could not resume this task: ${e instanceof Error ? e.message : String(e)}`);
    return false;
  }
  if (matchedAgent) lastSessionByAgent.set(matchedAgent.id, id);
  if (activeId() === id) {
    const route = `#/s/${encodeURIComponent(id)}`;
    if (window.location.hash !== route) window.history.replaceState({}, "", route);
  }
  await hydrate(id);
  // Load the durable conversation before opening two long-lived event
  // connections. Browsers cap same-origin HTTP/1.1 connections; opening
  // streams first could starve transcript/presentation requests on return
  // to an Agent, leaving its pane stuck on "Loading task".
  if (activeId() === id || splitId() === id) openStream(id);
  return true;
}

let openingAgent: Promise<string | null> | null = null;
const lastSessionByAgent = new Map<string, string>();
export async function openAgentChat(agentId = "vak", createNew = false): Promise<string | null> {
  while (openingAgent) {
    await openingAgent;
  }
  setAgentOpening(true);
  setOpeningAgentId(agentId);
  const source = api.backendUrl();
  const cwd = backend().cwd;
  openingAgent = (async () => {
  try {
    // Each Agent can own several conversations. Resume the one this client
    // last viewed; on a fresh client, choose its most recent nonempty task.
    const remembered = lastSessionByAgent.get(agentId);
    const recent = sessions().find((session) => session.agent?.id === agentId && session.title)?.session_id;
    const existing = remembered && sessions().some((session) => session.session_id === remembered)
      ? remembered
      : recent;
    if (existing && !createNew) {
      closeSplit();
      setReplyTarget(null);
      setArmedGoal(null);
      setDockTab(null);
      if (await activate(existing) === false) throw new Error("The conversation could not be resumed. Try again.");
      setAgentOpening(false);
      setOpeningAgentId(null);
      await refreshSessions();
      return existing;
    }
    const res = await api.openAgent(agentId, createNew);
    if (source !== api.backendUrl() || cwd !== backend().cwd) return null;
    recordAgentOpened(agentId);
    setActiveAgent(res.agent);
    setSessions((current) => [...current.filter((s) => s.session_id !== res.session_id), {session_id: res.session_id, cwd: res.cwd, agent: res.agent}]);
    closeSplit();
    setReplyTarget(null);
    setArmedGoal(null);
    setDockTab(null);
    if (await activate(res.session_id) === false) throw new Error("The conversation could not be resumed. Try again.");
    setAgentOpening(false);
    setOpeningAgentId(null);
    await refreshSessions();
    return res.session_id;
  } catch (e) {
    setNotice({ kind: "error", text: `Could not open this agent: ${e instanceof Error ? e.message : String(e)}` });
    return null;
  }
  })();
  try { return await openingAgent; }
  finally { openingAgent = null; setAgentOpening(false); setOpeningAgentId(null); }
}

export async function newSession() {
  await openAgentChat(activeAgentId(), true);
  window.dispatchEvent(new CustomEvent("vak:focus-composer"));
}

// ---- Split view ----------------------------------------------------------------

/**
 * Toggle two-session side-by-side view. The focused session stays put;
 * `candidate` (or the most recent other session) fills the other pane.
 * Positions never move afterwards — focusing a pane swaps contents.
 */
export async function toggleSplit(candidate?: string) {
  if (splitId()) {
    closeSplit();
    return;
  }
  const current = activeId();
  let other = candidate ?? null;
  if (!other) {
    const others = sessions().filter((s) => s.session_id !== current && !s.archived);
    other = others[0]?.session_id ?? null;
  }
  if (!other || other === current || !current) return;
  setSplitId(other);
  setSplitFocused(false); // focus stays on the left; candidate sits right
  openStream(other);
  await hydrate(other);
}

export function closeSplit() {
  setSplitId(null);
  setSplitFocused(false);
  pruneStreams();
}

/** Swap pane contents so the clicked side becomes the focused one. */
export function swapPanes() {
  if (!splitId()) return;
  const prevActive = activeId();
  setActiveId(splitId());
  setSplitId(prevActive);
  setSplitFocused(!splitFocused());
}

export async function sendPrompt(
  text: string,
  goal?: { objective: string; criteria: string[] },
  attachments?: api.Attachments,
  // Send to a specific session instead of the focused one — e.g. a diff
  // pane bound to a best-of-N child via `diffTarget`, which is not
  // necessarily `activeId()`. Without this, DiffPane's "review" always
  // reviewed the *focused* session's diff copy regardless of which
  // session's changes were actually on screen.
  targetId?: string | null,
  replyTarget?: ReplyTarget | null,
  relation?: api.RoutingEnvelope["relation"],
  propagateError = false,
) {
  if (!text.trim() && !attachments?.images.length && !attachments?.files.length) return;
  let id = targetId ?? activeId();
  if (!id) {
    id = await openAgentChat("vak");
    if (!id) return;
  }
  // Goal mode (docs/design/27 Phase H): /goal arms, bare /goal shows
  // status, /goal off disarms — mirroring the TUI. Shares one signal
  // (store.ts `armedGoal`) with the composer's own goal-mode form, so
  // arming it here and arming it from the sparkle button are the same
  // state, not two.
  if (text.trim().startsWith("/goal")) {
    const arg = text.trim().slice(5).trim();
    const current = goalAppliesTo(id);
    if (!arg) {
      appendSystem(id, current ? `🎯 armed: ${current.objective} (${current.criteria.length} criteria)` : "no goal armed · usage: /goal <objective> -- c1; c2");
      return;
    }
    if (arg === "off") {
      if (current) setArmedGoal(null);
      appendSystem(id, "goal disarmed");
      return;
    }
    const idx = arg.indexOf("--");
    const objective = (idx >= 0 ? arg.slice(0, idx) : arg).trim();
    const criteria = (idx >= 0 ? arg.slice(idx + 2) : "")
      .split(";")
      .map((c) => c.trim())
      .filter(Boolean);
    if (!objective || criteria.length === 0) {
      appendSystem(id, "goal needs criteria: /goal <objective> -- c1; c2");
      return;
    }
    setArmedGoal({ objective, criteria, sessionId: id });
    appendSystem(id, `🎯 goal armed (${criteria.length} criteria) — next prompt will be audited`);
    return;
  }
  if (text.trim() === "/status") {
    try {
      const projection = await api.work(id);
      if (!projection) {
        appendSystem(id, "Nothing is currently tracked for this conversation.");
      } else {
        const items = Object.values(projection.items ?? {});
        const active = items.filter((item) => ["running", "blocked", "waiting_approval", "ready_for_verification"].includes(item.status));
        const done = items.filter((item) => ["succeeded", "failed", "cancelled", "interrupted"].includes(item.status));
        appendSystem(id, active.length
          ? `${active.length} item${active.length === 1 ? " is" : "s are"} active${done.length ? `; ${done.length} finished` : ""}.`
          : `No active work${done.length ? `; ${done.length} item${done.length === 1 ? " is" : "s are"} finished` : ""}.`);
      }
    } catch (e) {
      appendSystem(id, `Status unavailable: ${e instanceof Error ? e.message : String(e)}`);
    }
    return;
  }

  const thisGoal = goalAppliesTo(id);
  setArmedGoal(null);
  appendUser(id, text, attachments?.files);
  try {
    const requestId = typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
      ? crypto.randomUUID()
      : `${Date.now()}-${Math.random().toString(36).slice(2)}`;
    if (isRunning(id)) {
      if (attachments?.files.length) {
        throw new Error("files can be attached once this turn finishes");
      }
      const receipt = await api.steer(id, text, attachments?.images, requestId, {
        message_id: requestId,
        conversation_id: id,
        target_work_id: replyTarget?.sessionId,
        target_result_id: replyTarget?.resultId,
        relation: "follow_up",
        provenance: "client.composer",
      });
      if (receipt.state === "steering_queued") {
        appendSystem(id, `Steering queued · ${receipt.request_id}`);
      }
    } else {
      markRunning(id, true);
      try {
        const routing: api.RoutingEnvelope = {
          message_id: requestId,
          conversation_id: id,
          target_work_id: replyTarget?.sessionId,
          target_result_id: replyTarget?.resultId,
          relation: relation ?? (replyTarget ? "correction" : "independent"),
          provenance: "client.composer",
        };
        let admission: api.RunAdmission;
        try {
          admission = await api.runPrompt(id, text, goal ?? thisGoal ?? undefined, attachments, requestId, routing);
        } catch (firstError) {
          // A lost HTTP response must recover the same admission. Retry only
          // transport-shaped failures; server rejections remain visible and
          // are never repeated as a different request.
          const message = firstError instanceof Error ? firstError.message.toLowerCase() : String(firstError).toLowerCase();
          const transportFailure = firstError instanceof TypeError || /network|fetch|timeout|connection|failed to fetch/.test(message);
          if (!transportFailure) throw firstError;
          admission = await api.runPrompt(id, text, goal ?? thisGoal ?? undefined, attachments, requestId, routing);
        }
        // "started" is the ordinary path and needs no notice. "queued"
        // means another admission on this session is ahead of it -- a
        // quiet transient notice, not a chat message, since it is about
        // scheduling and not conversation. "duplicate" is this exact
        // request already admitted (a retried request_id) and needs none.
        if (admission.state === "queued") {
          setNotice({ kind: "info", text: "Queued, it runs next" });
        }
      } catch (e) {
        markRunning(id, false);
        throw e;
      }
    }
  } catch (e) {
    if (e instanceof api.ApiError && e.kind === "no_ai_service") appendNeedsAiService(id);
    else appendSystem(id, `error: ${e instanceof Error ? e.message : String(e)}`);
    if (propagateError) throw e;
  }
}

/// Answer one gate. `remember` additionally persists the narrowest rule that
/// covers this call, so calls of the same shape stop asking.
///
/// Remembering never blocks the answer — the run is already paused waiting
/// on it. A call that cannot be narrowed safely (opaque shell, a one-off
/// URL) is approved and simply not remembered. That is control-plane
/// feedback, not an assistant message, so it belongs in the transient notice
/// surface rather than being appended to the transcript on every call.
export async function approve(
  requestId: string,
  ok: boolean,
  sessionId?: string | null,
  remember = false,
) {
  const id = sessionId ?? activeId();
  if (!id) return;
  if (pendingApprovals.has(requestId)) return;
  pendingApprovals.add(requestId);
  try {
    const result = await api.answerApproval(id, requestId, ok, remember);
    resolveApproval(id, requestId, ok ? "allowed" : "denied");
    if (result.learned_rule) {
      appendSystem(id, `won't ask again for ${result.learned_rule}`);
    } else if (result.learn_error) {
      setNotice({ kind: "error", text: `Allowed for this call; not remembered: ${result.learn_error}` });
    }
  } catch (e) {
    const message = `approval failed: ${e instanceof Error ? e.message : String(e)}`;
    appendSystem(id, message);
    setNotice({ kind: "error", text: `Could not record that approval: ${e instanceof Error ? e.message : String(e)}` });
  } finally {
    pendingApprovals.delete(requestId);
  }
}

export function stopRun() {
  const id = activeId();
  if (id && isRunning(id)) {
    // `cancel` no longer synthesizes a `RunFinished` -- the run's own
    // terminal event arrives once it actually stops, which can take a
    // moment (the current tool call or provider request has to unwind).
    // Hold a "stopping" state so the UI shows that between the click and
    // that event instead of nothing. `markRunning(id, false, ...)` on the
    // real `RunFinished` clears it.
    markStopping(id, true);
    void api.cancelRun(id).catch((err) => {
      markStopping(id, false);
      setNotice({ kind: "error", text: `Could not cancel this task: ${err instanceof Error ? err.message : String(err)}` });
    });
  }
}

export async function loadHealth() {
  const source = api.backendUrl();
  // The web client is same-origin, so its base is always empty; only the
  // desktop has no backend until it adopts one.
  if (!source && host.kind !== "web") return;
  try {
    const next = await api.health();
    if (source === api.backendUrl()) {
      setHealth(next);
      // With no session stream open, health is the authoritative transport
      // signal for the idle/new-task surface. Active streams retain their
      // more specific reconnect/resync state.
      if (streams.size === 0) setConnection("live");
    }
  } catch {
    setHealth(null);
    if (streams.size === 0) setConnection("offline");
  }
}

export async function sendSideQuestion(question: string) {
  const id = activeId();
  if (!id || !question.trim()) return;
  appendUser(id, question, undefined, "side");
  markRunning(id, true, "side");
  ensureSideStream(id);
  try {
    await api.runSide(id, question);
  } catch (e) {
    markRunning(id, false, "side");
    appendSystem(id, e instanceof Error ? e.message : String(e), "side");
  }
}

export function stopSide() {
  const id = activeId();
  if (id && isRunning(id, "side")) {
    void api.cancelSide(id).catch((err) => {
      setNotice({ kind: "error", text: `Could not cancel the side question: ${err instanceof Error ? err.message : String(err)}` });
    });
  }
}

function closeAllSideStreams() {
  sideStreams.forEach((stop) => stop());
  sideStreams.clear();
}

function ensureSideStream(id: string) {
  if (sideStreams.has(id)) return;
  sideStreams.set(id, watchSession(id, { side: (ev) => applyEvent(id, ev, { bucket: "side" }) }));
}

/**
 * Re-read backend state from the shell and flip the UI when it is ready.
 * The `backend-ready` event is a fast-path nicety; every caller that cares
 * about actually transitioning (project gate, project switch) awaits this,
 * so a missed event can never strand the UI.
 */
let backendRefreshEpoch = 0;

function resetWorkspaceView() {
  lastSessionByAgent.clear();
  closeAllStreams();
  closeAllSideStreams();
  setActiveId(null);
  setReplyTarget(null);
  setArmedGoal(null);
  setSplitId(null);
  setSplitFocused(false);
  setSessions([]);
  setHealth(null);
  setHydratingId(null);
  setDiffTarget(null);
  setEditorPath(null);
  setDockTab(null);
  setSideOpen(false);
  setHistoryOpen(false);
  setTranscriptViewId(null);
}

export async function refreshBackend(knownInfo?: import("./types").BackendInfo): Promise<boolean> {
  const epoch = ++backendRefreshEpoch;
  try {
    const info = knownInfo ?? await api.initBackend();
    if (epoch !== backendRefreshEpoch) return false;
    if (!info.ready) {
      api.adoptBackend(info);
      setBackend(info);
      return false;
    }
    const changedWorkspace = !!backend().cwd && backend().cwd !== info.cwd;
    api.adoptBackend(info);
    if (changedWorkspace) resetWorkspaceView();
    // Resolve the provider picture *before* publishing readiness: an error
    // thrown while the workspace mounts would propagate out of this
    // function and leave providers unset, stranding the gate.
    try {
      const p = await api.listProviders();
      if (epoch !== backendRefreshEpoch) return false;
      setProviders(p);
    } catch {
      if (epoch !== backendRefreshEpoch) return false;
      setProviders(null);
    }
    setBackend(info);
    await loadHealth();
    await refreshSessions();
    if (!activeId() && !window.location.hash.startsWith("#/s/")) await openAgentChat("vak");
    return true;
  } catch {
    return false;
  }
}

/**
 * Exchange a one-shot `?token=` in the URL for a session, then erase it.
 *
 * `vak open` puts it there so nobody has to find and paste the server's
 * token to reach their own local machine. It is removed from the URL before
 * anything else runs — a credential in an address bar reaches history, the
 * `Referer` of every subsequent request, and whatever is sharing the screen.
 *
 * Loopback only in practice: the server refuses `?token=` from any
 * non-loopback host (invariant 34), so a link like this pasted at a remote
 * deployment authenticates nothing.
 */
async function consumeTokenFromUrl(): Promise<void> {
  if (!host.authenticate) return;
  const url = new URL(window.location.href);
  const token = url.searchParams.get("token");
  if (!token) return;
  url.searchParams.delete("token");
  window.history.replaceState({}, "", url.toString());
  try {
    await host.authenticate(token);
  } catch {
    // Wrong or stale token: fall through to the normal login screen
    // rather than dead-ending on an error the user cannot act on.
  }
}

async function init() {
  await consumeTokenFromUrl();
  // A session that expires mid-use must return the client to the gate,
  // not bury the reason under repeated request failures. Only hosts that
  // have sessions can lose one; the desktop holds its token for the life
  // of the process.
  if (host.sessionStatus) {
    api.setUnauthorizedHandler(() => {
      if (!backend().ready) return; // already at the gate
      closeAllStreams();
      closeAllSideStreams();
      setBackend({ ready: false, recent_workspaces: [] });
      setNotice({ kind: "info", text: "Your session expired — sign in to continue." });
    });
  }
  await refreshBackend();
}

/**
 * Open a file the agent touched in whichever right-hand pane actually shows
 * something useful: a modified file has a diff worth reading (Changes), a
 * newly created one does not (Editor). Routing this automatically is the
 * point — the panel has five tabs and the user should not have to guess
 * which one currently holds their file.
 */
export async function openFileSmart(path: string) {
  const clean = path.trim().replace(/[.,;:!?)]'"`]+$/, "").trim();
  if (isScratchDirectory(clean) || (isDirectoryPath(clean) && clean.includes(".vak/scratch"))) {
    openWorkbenchFolder(clean);
    return;
  }
  setEditorPath(clean);
  const id = diffTarget() ?? activeId();
  if (!id) {
    setDockTab("editor");
    return;
  }
  try {
    const d = await api.readDiff(id);
    const diff = `${d.diff ?? ""}\n${d.staged_diff ?? ""}`;
    setDockTab(diffCoversPath(diff, clean) ? "diff" : "editor");
  } catch {
    setDockTab("editor");
  }
}

/** True when a unified diff contains a hunk header for `path`. */
function diffCoversPath(diff: string, path: string): boolean {
  const rel = path.replace(/^.*?\/(?=[^/]+$)/, "");
  return diff
    .split("\n")
    .some((l) => (l.startsWith("+++ ") || l.startsWith("--- ")) && (l.includes(path) || l.endsWith(rel)));
}

/**
 * Switch the active workspace.
 *
 * With no `cwd`, asks the host to choose one: a native folder dialog on
 * the desktop, and nothing at all on the web — where `pickWorkspace()`
 * answers `null` by design, because the filesystem that matters is the
 * server's and the browser cannot see it. The web path goes through the
 * workspace gate's own directory browser instead, which always passes an
 * explicit `cwd` here.
 */
export async function switchWorkspace(cwd?: string) {
  if (workspaceSwitching()) return;
  try {
    if (!cwd && !host.can("native-dialogs")) {
      setAgentPickerTab("target");
      setAgentPickerOpen(true);
      return;
    }
    const dir = cwd ?? (await host.pickWorkspace());
    if (typeof dir === "string") {
      if (dir === backend().cwd) return;
      setWorkspaceSwitching(true);
      const info = await host.openWorkspace(dir);
      await refreshBackend(info);
    }
  } catch (error) {
    setNotice({
      kind: "error",
      text: `Could not open that workspace: ${error instanceof Error ? error.message : String(error)}`,
    });
  } finally {
    setWorkspaceSwitching(false);
  }
}

function closeAllStreams() {
  streams.forEach((stop) => stop());
  streams.clear();
}

/** Focus/follow header strip above each half of a split workspace. */
function PaneBadge(props: { session: string | null; focused: boolean; onClose?: () => void }) {
  const info = createMemo(() => sessions().find((x) => x.session_id === props.session));
  return (
    <div class="pane-badge" classList={{ focused: props.focused }}>
      <button
        type="button"
        class="pane-badge-main"
        disabled={!props.session || props.focused}
        title={props.focused ? "Focused — composer, stop, and dock act here" : "Click to focus this task"}
        onClick={() => {
          if (!props.focused) swapPanes();
        }}
      >
        <span class="dot" classList={{ run: !!props.session && isRunning(props.session!) }} />
        <span class="pane-badge-title">{info()?.title || (props.session ? "Untitled task" : "No task")}</span>
        <Show when={props.session && isRunning(props.session!)}>
          <span class="pane-badge-state">Working</span>
        </Show>
      </button>
      <Show when={props.onClose}>
        <button type="button" class="pane-badge-close" title="Close split view (⌘\)" aria-label="Close split view" onClick={props.onClose}>
          <Icon name="close" size={12} />
        </button>
      </Show>
    </div>
  );
}

/**
 * Two sessions side-by-side. Positions never move: the LEFT pane always
 * shows the unfocused session, the RIGHT one the focused session
 * (`paneSessions()` encodes this). Clicking a pane's badge swaps contents so
 * that side becomes the focus target for composer/approvals/dock.
 */
function SplitPanes() {
  let stack!: HTMLDivElement;
  const panes = createMemo(() => paneSessions());

  const persistRatio = (next: number) => {
    setSplitRatio(next);
    localStorage.setItem("vak.splitRatio", String(next));
  };

  const beginDrag = (event: PointerEvent) => {
    event.preventDefault();
    const startX = event.clientX;
    const startRatio = splitRatio();
    const width = stack.getBoundingClientRect().width;
    if (width <= 0) return;
    document.body.classList.add("is-resizing");
    const move = (next: PointerEvent) => {
      persistRatio(Math.max(0.25, Math.min(0.75, startRatio + (next.clientX - startX) / width)));
    };
    const end = () => {
      document.body.classList.remove("is-resizing");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end, { once: true });
  };

  return (
    <div class="main-stack split" ref={stack} style={`--split-left:${Math.round(splitRatio() * 100)}%`}>
      <div class="pane" classList={{ focused: !splitFocused() }}>
        <PaneBadge session={panes().left} focused={!splitFocused()} />
        <ChatPane sessionId={panes().left} />
        <Show when={sideOpen()}>
          <SideChatPanel />
        </Show>
      </div>
      <div
        class="split-divider"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize split panes"
        tabIndex={0}
        onPointerDown={beginDrag}
        onDblClick={() => persistRatio(0.5)}
        onKeyDown={(e) => {
          if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
          e.preventDefault();
          persistRatio(Math.max(0.25, Math.min(0.75, splitRatio() + (e.key === "ArrowRight" ? 0.02 : -0.02))));
        }}
      />
      <div class="pane" classList={{ focused: splitFocused() }}>
        <PaneBadge session={panes().right} focused={splitFocused()} onClose={() => void toggleSplit()} />
        <Show
          when={panes().right}
          fallback={
            <div class="pane-empty">
              <Icon name="grid" size={20} />
              <p>Pick another task from the sidebar to watch it here while you work on the left.</p>
            </div>
          }
        >
          <ChatPane sessionId={panes().right} />
        </Show>
      </div>
    </div>
  );
}

export default function App() {
  onMount(() => {
    const savedSidebar = Number(localStorage.getItem("vak.sidebarWidth"));
    const savedDock = Number(localStorage.getItem("vak.dockWidth"));
    if (savedSidebar >= 220 && savedSidebar <= 360) setSidebarWidth(savedSidebar);
    if (savedDock >= 340 && savedDock <= window.innerWidth * 0.7) setDockWidth(savedDock);
    // Register the listener before the first probe: the shell boots the
    // backend during setup() and emits `backend-ready` within milliseconds,
    // so subscribing after init() loses the event to the race.
    const stopHostWatch = host.onInfoChanged((info) => {
      if (!workspaceSwitching()) void refreshBackend(info);
    });
    // The shared stream's own state is the transport signal. "Live" is
    // claimed only once it is open, and never over an unfinished resync.
    const stopStatusWatch = watchStatus((status) => {
      if (status === "open") {
        if (resyncingSessions.size === 0) setConnection("live");
      } else if (status === "offline") {
        setConnection("offline");
      } else if (status === "connecting") {
        setConnection("connecting");
      } else if (status !== "idle") {
        setConnection("reconnecting");
      }
    });
    // A bookmarked or notification-clicked deep link, applied once the
    // backend is up — `activate` needs a session list to resolve against.
    void init().then(() => {
      if (window.location.hash) void applyRoute(window.location.hash);
    });
    const onHashChange = () => void applyRoute(window.location.hash);
    window.addEventListener("hashchange", onHashChange);
    const sessionRefresh = window.setInterval(() => void refreshSessions(), 10_000);
    const healthRefresh = window.setInterval(() => void loadHealth(), 10_000);

    // The browser knows about the radio before any request times out, so
    // losing the network shows immediately rather than after a stalled
    // fetch. Coming back does NOT assert "live" on its own — that is the
    // stream's job to prove, and claiming it early is how a UI ends up
    // saying "Live" at a blank pane.
    const goOffline = () => setConnection("offline");
    const goOnline = () => setConnection("reconnecting");
    window.addEventListener("offline", goOffline);
    window.addEventListener("online", goOnline);

    let pendingG = 0;
    const keys = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) {
        if (e.key === "Escape") {
          // Every dialog is a Sheet, which closes on Escape and stops it
          // there; what reaches here is a menu, Settings or a panel.
          if (closeOpenMenus()) return;
          if (settingsOpen()) setSettingsOpen(false);
          else if (inboxOpen()) setInboxOpen(false);
          else if (sideOpen()) setSideOpen(false);
          else stopRun();
        } else if (e.key === "g" || e.key === "G") {
          pendingG = Date.now();
        } else if ((e.key === "i" || e.key === "I") && Date.now() - pendingG < 1000) {
          const target = e.target as HTMLElement | null;
          const typing =
            target &&
            (target.tagName === "INPUT" ||
              target.tagName === "TEXTAREA" ||
              target.isContentEditable);
          if (!typing) {
            pendingG = 0;
            setInboxOpen(true);
          }
        } else if (e.key.length === 1) {
          // Any printable key other than g/G resets the pending vim sequence
          // so a stray g followed by unrelated typing cannot fire g→i later.
          pendingG = 0;
        }
        return;
      }
      if (e.key === "n" || e.key === "N") {
        e.preventDefault();
        void newSession();
      } else if (e.key === ",") {
        e.preventDefault();
        setSettingsOpen(true);
      } else if (e.key === "k" || e.key === "K") {
        e.preventDefault();
        setSearchOpen(true);
      } else if (e.key === "/") {
        e.preventDefault();
        setShowShortcuts((v) => !v);
      } else if (e.key === "h" || e.key === "H") {
        e.preventDefault();
        if (activeId()) setHistoryOpen((v) => !v);
      } else if (e.key === ";") {
        e.preventDefault();
        setSideOpen((v: boolean) => {
          if (!v && activeId()) ensureSideStream(activeId()!);
          return !v;
        });
      } else if (e.key === "d" || e.key === "D") {
        e.preventDefault();
        setDockTab((t) => (t === "diff" ? null : "diff"));
      } else if (e.key === "\\") {
        e.preventDefault();
        void toggleSplit();
      } else if (e.key === "`") {
        e.preventDefault();
        setDockTab((t) => (t === "terminal" ? null : "terminal"));
      } else if (e.key === "b" || e.key === "B") {
        e.preventDefault();
        setSidebarOpen((value) => !value);
      }
    };
    window.addEventListener("keydown", keys);
    const stopMenuDismissal = dismissMenusOnPressOutside();
    onCleanup(() => {
      window.removeEventListener("keydown", keys);
      stopMenuDismissal();
      window.removeEventListener("hashchange", onHashChange);
      window.removeEventListener("offline", goOffline);
      window.removeEventListener("online", goOnline);
      stopHostWatch();
      stopStatusWatch();
      window.clearInterval(sessionRefresh);
      window.clearInterval(healthRefresh);
      closeAllStreams();
      closeAllSideStreams();
    });
  });

  createEffect(() => {
    if (backend().ready) void loadHealth();
  });

  // Where the window controls overlay the page (the macOS desktop shell),
  // `data-chrome="overlay"` makes the sidebar, header and Settings leave
  // room for them and the page's title bar moves and zooms the window,
  // except in full screen, where the system hides the controls.
  const [fullscreen, setFullscreen] = createSignal(false);
  onMount(() => onCleanup(host.onFullscreenChange(setFullscreen)));
  createEffect(() => {
    const overlay = backend().window_chrome === "overlay" && !fullscreen();
    if (overlay) document.documentElement.dataset.chrome = "overlay";
    else delete document.documentElement.dataset.chrome;
    if (overlay && host.dragWindow) onCleanup(titleBarGestures(host));
  });

  // "system" is resolved here rather than in CSS so one attribute always
  // names the palette actually in force — every rule, and anything reading
  // a token out of the DOM (the terminal's theme, for one), sees the same
  // answer instead of half of them tracking a media query and half not.
  const [systemDark, setSystemDark] = createSignal(
    typeof matchMedia === "function" ? matchMedia("(prefers-color-scheme: dark)").matches : true,
  );
  onMount(() => {
    if (typeof matchMedia !== "function") return;
    const query = matchMedia("(prefers-color-scheme: dark)");
    const sync = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    query.addEventListener("change", sync);

    // Rotating a phone, or dragging a window across the breakpoint,
    // changes which layout is in force — the sidebar goes from column to
    // overlay — so the panel's open state has to follow, or a rotation
    // leaves an overlay covering the transcript.
    const narrow = matchMedia("(max-width: 900px)");
    const syncWidth = (e: MediaQueryListEvent) => {
      setNarrowViewport(e.matches);
      setSidebarOpen(!e.matches);
    };
    narrow.addEventListener("change", syncWidth);

    onCleanup(() => {
      query.removeEventListener("change", sync);
      narrow.removeEventListener("change", syncWidth);
    });
  });

  createEffect(() => {
    const theme =
      uiPreferences.theme === "system"
        ? systemDark()
          ? "dark"
          : "light"
        : uiPreferences.theme;
    document.documentElement.dataset.theme = theme;
    document.documentElement.dataset.visualPack = uiPreferences.visualPack;
    void host.setWindowIcon?.(uiPreferences.visualPack).catch((error) => {
      console.warn("Could not update the window icon:", error);
    });
    const favicon = document.querySelector<HTMLLinkElement>('link[rel="icon"]');
    if (favicon) {
      favicon.href = uiPreferences.visualPack === "dimensional"
        ? `${import.meta.env.BASE_URL}assets/brand/dimensional/app-icon-64.png`
        : `${import.meta.env.BASE_URL}favicon.svg`;
      favicon.type = uiPreferences.visualPack === "dimensional" ? "image/png" : "image/svg+xml";
    }
    // Tells the browser which way to paint its own chrome: form controls,
    // scrollbars, and the space behind the page during load.
    document.documentElement.style.colorScheme = theme === "light" ? "light" : "dark";
    document.documentElement.dataset.compactSidebar = String(uiPreferences.compactSidebar);
    document.documentElement.dataset.reduceMotion = String(uiPreferences.reduceMotion);
    document.documentElement.style.setProperty("--text-scale", String(uiPreferences.textScale / 100));
    document.documentElement.style.setProperty("--code-scale", String(uiPreferences.codeScale / 100));
    document.documentElement.style.setProperty("--sans", interfaceFonts[uiPreferences.interfaceFont].stack);
    document.documentElement.style.setProperty("--display", interfaceFonts[uiPreferences.interfaceFont].stack);
    document.documentElement.style.setProperty("--content", uiPreferences.contentFont === "inherit" ? interfaceFonts[uiPreferences.interfaceFont].stack : contentFonts[uiPreferences.contentFont].stack);
    document.documentElement.style.setProperty("--mono", codeFonts[uiPreferences.codeFont].stack);
  });

  return (
    <Show
      when={backend().ready ? backend() : null}
      fallback={<WorkspaceGate />}
    >
      {(info) => (
        <div
          class="app"
          classList={{
            "sidebar-collapsed": !sidebarOpen(),
            "canvas-active": canvasOpen(),
            "canvas-split-active": canvasOpen() && canvasMode() === "split",
            "canvas-focused-active": canvasOpen() && canvasMode() === "focused",
          }}
          style={`--sidebar-width:${sidebarWidth()}px;--dock-width:${dockWidth()}px`}
        >
          <Sidebar />
          <Show when={workspaceSwitching()}><div class="workspace-switching"><span class="dot run" />Opening workspace…</div></Show>
          <Show when={sidebarOpen()}><ResizeHandle side="sidebar" /></Show>
          <div class="main">
            <WorkspaceHeader />
            <Show
              when={inboxOpen()}
              fallback={
                <>
                  <Show
                    when={splitId()}
                    fallback={
                      <div class="main-stack">
                        <ChatPane />
                        <Show when={sideOpen()}>
                          <SideChatPanel />
                        </Show>
                      </div>
                    }
                  >
                    <SplitPanes />
                  </Show>
                  <Composer cwd={info().cwd ?? ""} />
                </>
              }
            >
              <InboxPage />
            </Show>
          </div>
          <Show when={dockTab()}>
            {(tab) => (
              <>
              <ResizeHandle side="dock" />
              <div
                class="dock"
                data-dock={tab()}
                data-testid="advanced-workspace-dock"
                role="complementary"
                aria-label={`Task details: ${dockLabel(tab())}`}
              >
                <div class="dock-tabs" data-titlebar>
                  {/* Only general-purpose views are pinned — pinning a
                      dev-only tool (Changes/Terminal) here would show up as
                      permanent chrome in every conversation, undoing the
                      general/developer split made in the header's menu. */}
                  <For each={[
                    ["workbench", "Files", "preview"],
                    ["preview", "Live preview", "preview"],
                  ] as const}>
                    {([id, label, icon]) => (
                      <button
                        class="dock-tab"
                        type="button"
                        aria-label={label}
                        classList={{ on: tab() === id }}
                        aria-pressed={tab() === id}
                        onClick={() => setDockTab(id)}
                      >
                        <Icon name={icon as IconName} />
                        <span>{label}</span>
                      </button>
                    )}
                  </For>
                  <details class="dock-more">
                    <summary class="dock-tab" aria-label="More workspace views"><Icon name="tune" /><span>More</span></summary>
                    <div class="dock-more-menu" data-titlebar="false">
                      <For each={[
                        ["agents", "Parallel work", "grid"],
                        ["feeds", "Sources", "bell"],
                        ["commitments", "Open promises", "shield"],
                      ] as const}>
                        {([id, label, icon]) => <button class="dock-tab" type="button" aria-label={label} aria-pressed={tab() === id} onClick={(event) => { setDockTab(id); event.currentTarget.closest("details")?.removeAttribute("open"); }}><Icon name={icon as IconName} /><span>{label}</span></button>}
                      </For>
                      <div class="menu-group-label">Developer</div>
                      <For each={[
                        ["diff", "Changes", "diff"], ["terminal", "Terminal", "terminal"],
                        ["editor", "Editor", "file"], ["pr", "Pull request", "git"],
                      ] as const}>
                        {([id, label, icon]) => <button class="dock-tab" type="button" aria-label={label} aria-pressed={tab() === id} onClick={(event) => { setDockTab(id); event.currentTarget.closest("details")?.removeAttribute("open"); }}><Icon name={icon as IconName} /><span>{label}</span></button>}
                      </For>
                    </div>
                  </details>
                  <button
                    type="button"
                    class="dock-close"
                    title="Close pane"
                    aria-label="Close workspace pane"
                    onClick={() => setDockTab(null)}
                  >
                    <Icon name="close" />
                  </button>
                </div>
                <Suspense fallback={<Skeleton kind="text" class="pane-loading" label="Loading" />}>
                  <Show when={tab() === "workbench"}>
                    <WorkbenchPanel />
                  </Show>
                  <Show when={tab() === "diff"}>
                    <DiffPane sessionId={diffTarget() ?? activeId()} />
                  </Show>
                  <Show when={tab() === "terminal"}>
                    <TerminalPane sessionId={activeId()} />
                  </Show>
                  <Show when={tab() === "editor"}>
                    <EditorPane />
                  </Show>
                <Show when={tab() === "pr"}>
                  <PrPanel sessionId={activeId()} />
                </Show>
                  <Show when={tab() === "preview"}>
                    <PreviewPane />
                  </Show>
                  <Show when={tab() === "agents"}>
                    <WorkersPanel sessionId={activeId()} />
                  </Show>
                  <Show when={tab() === "feeds"}>
                    <FeedsPanel />
                  </Show>
                  <Show when={tab() === "commitments"}>
                    <CommitmentsPanel />
                  </Show>
                </Suspense>
              </div>
              </>
            )}
          </Show>
          <StatusBar />
          {/* Incomplete setup never blocks the workspace. An empty
              conversation's greeting carries this card instead. */}
          <Show when={greetingsShown() === 0}>
            <SetupBanner />
          </Show>
          <BudgetBanner />
          <Show when={showShortcuts()}>
            <ShortcutsModal />
          </Show>
          <Show when={bestOfOpen()}>
            <BestOfNDialog />
          </Show>
          <TasksModal />
          <Show when={historyOpen()}>
            <CheckpointsModal />
          </Show>
          <Show when={receiptsOpen()}>
            <ReceiptsModal />
          </Show>
          <WorkModal />
          <Show when={transcriptViewId()}>
            <TranscriptModal />
          </Show>
          {/* Hidden playback element for /voice/speak narration; the store's
              speak() helper drives it via registerVoiceAudioElement. */}
          <audio
            ref={(el) => registerVoiceAudioElement(el)}
            style="display:none"
            aria-hidden="true"
          />
          <Toast />
          <SearchModal />
          <FeedsModal />
          <AgentPickerModal />
          <AgentCreateWizard />
          <ConnectSheet />
          <Suspense><ArtifactCanvas /></Suspense>
          <Show when={settingsOpen()}>
            <Suspense fallback={<Skeleton kind="blocks" class="modal-loading" label="Loading settings" />}><Settings /></Suspense>
          </Show>
        </div>
      )}
    </Show>
  );
}

function dockLabel(tab: import("./store").DockTab): string {
  return {
    workbench: "Result",
    diff: "Review",
    terminal: "Activity",
    preview: "Live preview",
    editor: "Files",
    pr: "Pull request",
    agents: "Workers",
    feeds: "Feeds",
    commitments: "Commitments",
  }[tab];
}
