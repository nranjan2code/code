import { createEffect, createMemo, onCleanup, onMount, Show, For } from "solid-js";
import { host } from "./host";
import {
  activeId,
  appendSystem,
  sessions,
  appendUser,
  applyEvent,
  applyPresentationEvent,
  backend,
  isRunning,
  markRunning,
  resolveApproval,
  resetSessionView,
  setActiveId,
  setBackend,
  setDockTab,
  setEditorPath,
  setDiffTarget,
  setHealth,
  setShowShortcuts,
  setSessions,
  setUsageFor,
  hydrateFromTranscript,
  hydrateFromPresentation,
  clearPresentation,
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
  sidebarOpen,
  sidebarWidth,
  setNotice,
  setBestOfOpen,
  setProviders,
  setTasksOpen,
  tasksOpen,
  historyOpen,
  setHistoryOpen,
   receiptsOpen,
  setReceiptsOpen,
  setWorkOpen,
  searchOpen,
  setSearchOpen,
  settingsOpen,
  setSettingsOpen,
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
  setArmedGoal,
  goalAppliesTo,
} from "./store";
import type { SessionSummary } from "./types";
import * as api from "./api";
import Sidebar from "./components/Sidebar";
import ChatPane from "./components/ChatPane";
import Composer from "./components/Composer";
import StatusBar from "./components/StatusBar";
import DiffPane from "./components/DiffPane";
import TerminalPane from "./components/TerminalPane";
import EditorPane from "./components/EditorPane";
import ShortcutsModal from "./components/ShortcutsModal";
import SideChatPanel from "./components/SideChatPanel";
import BestOfNDialog from "./components/BestOfNDialog";
import PrPanel from "./components/PrPanel";
import TasksModal from "./components/TasksModal";
import CheckpointsModal from "./components/CheckpointsModal";
import ReceiptsModal from "./components/ReceiptsModal";
import WorkModal from "./components/WorkModal";
import PreviewPane from "./components/PreviewPane";
import SubagentsPanel from "./components/SubagentsPanel";
import CommitmentsPanel from "./components/CommitmentsPanel";
import WorkspaceGate from "./components/WorkspaceGate";
import WorkspaceHeader from "./components/WorkspaceHeader";
import Icon, { type IconName } from "./components/Icon";
import ResizeHandle from "./components/ResizeHandle";
import Toast from "./components/Toast";
import Settings from "./components/Settings";
import BudgetBanner from "./components/BudgetBanner";
import SearchModal from "./components/SearchModal";
import FeedsPanel from "./components/FeedsPanel";
import FeedsModal from "./components/FeedsModal";
import SetupBanner from "./components/SetupBanner";
import TranscriptModal from "./components/TranscriptModal";
import InboxPage from "./components/InboxPage";

const streams = new Map<string, EventSource>();
const presentationStreams = new Map<string, EventSource>();
const sideStreams = new Map<string, EventSource>();

export async function refreshSessions() {
  const source = api.backendUrl();
  if (!source) return;
  try {
    const res = await api.listSessions();
    if (source === api.backendUrl()) {
      setSessions(res.sessions);
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
      // EventSource is constructed but its connection is still
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
      for (const s of res.sessions) {
        if (!s.running && isRunning(s.session_id)) {
          markRunning(s.session_id, false);
          if (visible.has(s.session_id)) {
            await hydrate(s.session_id);
          }
        }
      }
    }
  } catch {
    /* backend restarting */
  }
}

async function hydrate(id: string) {
  setHydratingId(id);
  try {
    const [t, presentation] = await Promise.all([
      api.transcript(id),
      api.presentation(id).catch(() => null),
    ]);
    if (!isRunning(id)) {
      hydrateFromTranscript(id, t.messages);
      if (presentation) hydrateFromPresentation(id, presentation);
      else clearPresentation(id);
      setUsageFor(id, t.usage);
    }
  } catch (error) {
    if (!isRunning(id)) {
      clearPresentation(id);
      appendSystem(id, `Could not load this task: ${error instanceof Error ? error.message : String(error)}`);
    }
  } finally {
    setHydratingId((current) => (current === id ? null : current));
  }
}

const reconnectTimers = new Map<string, ReturnType<typeof setTimeout>>();

function openStream(id: string) {
  if (streams.has(id)) return;
  const es = api.openEventStream(
    id,
    (ev) => applyEvent(id, ev, { onFinish: (s) => onFinished(id, s) }),
    () => {
      // A plain browser-level connection error is not necessarily fatal:
      // EventSource retries transient failures on its own per spec. Only
      // intervene once it has actually given up (readyState CLOSED) --
      // otherwise this races the browser's own reconnect and can tear
      // down a connection that would have recovered by itself.
      if (es.readyState !== EventSource.CLOSED) return;
      // Without this, a permanently closed connection left `streams`
      // holding a dead entry forever: openStream's own guard above then
      // refused to ever reopen it for this session, so a run that
      // finished after the drop had no path left to reach the UI --
      // the backend would complete and durably log the reply while the
      // task stayed on "Working" until the app was relaunched.
      if (streams.get(id) === es) streams.delete(id);
      if (reconnectTimers.has(id)) return;
      reconnectTimers.set(
        id,
        setTimeout(() => {
          reconnectTimers.delete(id);
          openStream(id);
        }, 2000),
      );
    },
  );
  streams.set(id, es);
  if (!presentationStreams.has(id)) {
    const presentation = api.openPresentationStream(
      id,
      (event) => applyPresentationEvent(id, event),
      () => {
        if (presentation.readyState === EventSource.CLOSED && presentationStreams.get(id) === presentation) {
          presentationStreams.delete(id);
        }
      },
    );
    presentationStreams.set(id, presentation);
  }
}

const NOTIFY_DEDUPE_MS = 5 * 60 * 1000;
const lastNotifyAt = new Map<string, number>();

/** Native notification with per-source 5-minute dedupe (desktop round 2). */
export async function notifyOnce(source: string, title: string, body: string) {
  const now = Date.now();
  if (now - (lastNotifyAt.get(source) ?? 0) < NOTIFY_DEDUPE_MS) return;
  lastNotifyAt.set(source, now);
  await notify(title, body);
}

async function notify(title: string, body: string) {
  if (!uiPreferences.notifications) return;
  // OS notification centre on the desktop, the Web Notifications API in a
  // browser — the host knows which, and both are best-effort.
  await host.notify(title, body);
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
  void Promise.all([refreshSessions(), hydrate(id)]);
  if (document.hidden && id === activeId()) {
    const s = sessions().find((x) => x.session_id === id);
    void notify("Vak run finished", `${s?.title ?? "Session"} — ${summary}`);
  }
}

export async function activate(id: string) {
  // Choosing a task always lands back in the workspace view.
  setInboxOpen(false);
  // Clicking the session already shown in the other pane focuses it there
  // instead of duplicating it across both panes.
  if (splitId() && id === splitId()) {
    swapPanes();
    return;
  }
  setActiveId(id);
  if (sessions().find((session) => session.session_id === id)?.running) {
    markRunning(id, true);
  }
  // Persisted sessions must be attached server-side before they can accept
  // runs; for live sessions the server keeps the existing handle (idempotent).
  try {
    await api.attachSession(id);
  } catch (e) {
    appendSystem(id, `could not resume this task: ${e instanceof Error ? e.message : String(e)}`);
  }
  openStream(id);
  await hydrate(id);
}

export async function newSession() {
  try {
    const res = await api.createSession();
    resetSessionView(res.session_id);
    await activate(res.session_id);
    await refreshSessions();
  } catch (e) {
    setNotice({ kind: "error", text: `Could not create a task: ${e instanceof Error ? e.message : String(e)}` });
  }
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
  attachments?: { mime: string; data: string }[],
  // Send to a specific session instead of the focused one — e.g. a diff
  // pane bound to a best-of-N child via `diffTarget`, which is not
  // necessarily `activeId()`. Without this, DiffPane's "review" always
  // reviewed the *focused* session's diff copy regardless of which
  // session's changes were actually on screen.
  targetId?: string | null,
) {
  if (!text.trim() && !(attachments && attachments.length)) return;
  let id = targetId ?? activeId();
  if (!id) {
    // Typing into the empty state is the natural way to start: create the
    // task rather than silently dropping the prompt because nothing is
    // selected. Only applies to the focused-session path — a caller
    // naming an explicit `targetId` means an existing session.
    await newSession();
    id = activeId();
    if (!id) return; // newSession already surfaced why
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

  const thisGoal = goalAppliesTo(id);
  setArmedGoal(null);
  appendUser(id, text);
  try {
    if (isRunning(id)) {
      await api.steer(id, text, attachments);
    } else {
      markRunning(id, true);
      try {
        await api.runPrompt(id, text, goal ?? thisGoal ?? undefined, attachments);
      } catch (e) {
        markRunning(id, false);
        throw e;
      }
    }
  } catch (e) {
    appendSystem(id, `error: ${e instanceof Error ? e.message : String(e)}`);
  }
}

/// Answer one gate. `remember` additionally persists the narrowest rule that
/// covers this call, so calls of the same shape stop asking.
///
/// Remembering never blocks the answer — the run is already paused waiting
/// on it. A call that cannot be narrowed safely (opaque shell, a one-off
/// URL) is approved and simply not remembered, and says so in the
/// transcript rather than failing silently or turning into a refusal.
export async function approve(
  requestId: string,
  ok: boolean,
  sessionId?: string | null,
  remember = false,
) {
  const id = sessionId ?? activeId();
  if (!id) return;
  try {
    const result = await api.answerApproval(id, requestId, ok, remember);
    resolveApproval(id, requestId, ok ? "allowed" : "denied");
    if (result.learned_rule) {
      appendSystem(id, `won't ask again for ${result.learned_rule}`);
    } else if (result.learn_error) {
      appendSystem(id, `allowed, but not remembered: ${result.learn_error}`);
    }
  } catch (e) {
    appendSystem(id, `approval failed: ${e instanceof Error ? e.message : String(e)}`);
  }
}

export function stopRun() {
  const id = activeId();
  if (id && isRunning(id)) void api.cancelRun(id);
}

export async function loadHealth() {
  const source = api.backendUrl();
  if (!source) return;
  try {
    const next = await api.health();
    if (source === api.backendUrl()) setHealth(next);
  } catch {
    setHealth(null);
  }
}

export async function sendSideQuestion(question: string) {
  const id = activeId();
  if (!id || !question.trim()) return;
  appendUser(id, question, "side");
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
  if (id && isRunning(id, "side")) void api.cancelSide(id);
}

function closeAllSideStreams() {
  sideStreams.forEach((es) => es.close());
  sideStreams.clear();
}

function ensureSideStream(id: string) {
  if (sideStreams.has(id)) return;
  const es = api.openSideStream(id, (ev) =>
    applyEvent(id, ev, { bucket: "side" }),
  );
  sideStreams.set(id, es);
}

/**
 * Re-read backend state from the shell and flip the UI when it is ready.
 * The `backend-ready` event is a fast-path nicety; every caller that cares
 * about actually transitioning (project gate, project switch) awaits this,
 * so a missed event can never strand the UI.
 */
let backendRefreshEpoch = 0;

function resetWorkspaceView() {
  closeAllStreams();
  closeAllSideStreams();
  setActiveId(null);
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
    return true;
  } catch {
    return false;
  }
}

async function init() {
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
  setEditorPath(path);
  const id = diffTarget() ?? activeId();
  if (!id) {
    setDockTab("editor");
    return;
  }
  try {
    const d = await api.readDiff(id);
    const diff = `${d.diff ?? ""}\n${d.staged_diff ?? ""}`;
    setDockTab(diffCoversPath(diff, path) ? "diff" : "editor");
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
export async function switchProject(cwd?: string) {
  if (workspaceSwitching()) return;
  try {
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
      text: `Could not open that project: ${error instanceof Error ? error.message : String(error)}`,
    });
  } finally {
    setWorkspaceSwitching(false);
  }
}

function closeAllStreams() {
  streams.forEach((es) => es.close());
  streams.clear();
  presentationStreams.forEach((es) => es.close());
  presentationStreams.clear();
}

/** Focus/follow header strip above each half of a split workspace. */
function PaneBadge(props: { session: string | null; focused: boolean; onClose?: () => void }) {
  const info = createMemo(() => sessions().find((x) => x.session_id === props.session));
  return (
    <div class="pane-badge" classList={{ focused: props.focused }}>
      <button
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
        <button class="pane-badge-close" title="Close split view (⌘\)" aria-label="Close split view" onClick={props.onClose}>
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
    void init();
    const sessionRefresh = window.setInterval(() => void refreshSessions(), 10_000);

    let pendingG = 0;
    const keys = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) {
        if (e.key === "Escape") {
          if (showShortcuts()) setShowShortcuts(false);
          else if (searchOpen()) setSearchOpen(false);
          else if (settingsOpen()) setSettingsOpen(false);
          else if (bestOfOpen()) setBestOfOpen(false);
          else if (tasksOpen()) setTasksOpen(false);
          else if (historyOpen()) setHistoryOpen(false);
          else if (receiptsOpen()) setReceiptsOpen(false);
          else if (transcriptViewId()) setTranscriptViewId(null);
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
    onCleanup(() => {
      window.removeEventListener("keydown", keys);
      stopHostWatch();
      window.clearInterval(sessionRefresh);
      closeAllStreams();
      closeAllSideStreams();
    });
  });

  createEffect(() => {
    if (backend().ready) void loadHealth();
  });

  createEffect(() => {
    document.documentElement.dataset.theme = uiPreferences.theme;
    document.documentElement.dataset.compactSidebar = String(uiPreferences.compactSidebar);
    document.documentElement.dataset.reduceMotion = String(uiPreferences.reduceMotion);
    document.documentElement.style.setProperty("--text-scale", String(uiPreferences.textScale / 100));
    document.documentElement.style.setProperty("--code-scale", String(uiPreferences.codeScale / 100));
  });

  return (
    <Show
      when={backend().ready ? backend() : null}
      fallback={<WorkspaceGate />}
    >
      {(info) => (
        <div
          class="app"
          classList={{ "sidebar-collapsed": !sidebarOpen() }}
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
              <div class="dock" data-dock={tab()}>
                <div class="dock-tabs">
                  <For each={[
                    ["preview", "Preview", "preview"],
                    ["diff", "Changes", "diff"],
                    ["terminal", "Terminal", "terminal"],
                    ["editor", "Editor", "code"],
                    ["pr", "Pull request", "git"],
                    ["agents", "Subagents", "grid"],
                    ["feeds", "Feeds", "bell"],
                    ["commitments", "Commitments", "shield"],
                  ] as const}>
                    {([id, label, icon]) => (
                      <button
                        class="dock-tab"
                        classList={{ on: tab() === id }}
                        onClick={() => setDockTab(id)}
                      >
                        <Icon name={icon as IconName} />
                        <span>{label}</span>
                      </button>
                    )}
                  </For>
                  <button
                    class="dock-close"
                    title="Close pane"
                    aria-label="Close workspace pane"
                    onClick={() => setDockTab(null)}
                  >
                    <Icon name="close" />
                  </button>
                </div>
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
                  <SubagentsPanel sessionId={activeId()} />
                </Show>
                <Show when={tab() === "feeds"}>
                  <FeedsPanel />
                </Show>
                <Show when={tab() === "commitments"}>
                  <CommitmentsPanel />
                </Show>
              </div>
              </>
            )}
          </Show>
          <StatusBar />
          {/* Incomplete setup never blocks the workspace; the banner
              points at the one wizard rather than being a second one. */}
          <SetupBanner />
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
          <Show when={settingsOpen()}><Settings /></Show>
        </div>
      )}
    </Show>
  );
}
