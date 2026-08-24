import { createEffect, onCleanup, onMount, Show, For } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  activeId,
  appendSystem,
  sessions,
  appendUser,
  applyEvent,
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
  providers,
  setupNeeded,
  setProviders,
  setSetupNeeded,
  setTasksOpen,
  tasksOpen,
  historyOpen,
  setHistoryOpen,
  settingsOpen,
  setSettingsOpen,
  uiPreferences,
  workspaceSwitching,
  setWorkspaceSwitching,
} from "./store";
import type { SessionSummary } from "./types";
import * as api from "./api";

// Armed goal consumed by the next prompt (docs/design/27 Phase H).
let armedGoal: { objective: string; criteria: string[] } | null = null;
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
import PreviewPane from "./components/PreviewPane";
import ProjectGate from "./components/ProjectGate";
import WorkspaceHeader from "./components/WorkspaceHeader";
import Icon, { type IconName } from "./components/Icon";
import ResizeHandle from "./components/ResizeHandle";
import Toast from "./components/Toast";
import Settings from "./components/Settings";

const streams = new Map<string, EventSource>();
const sideStreams = new Map<string, EventSource>();

export async function refreshSessions() {
  const source = api.backendUrl();
  if (!source) return;
  try {
    const res = await api.listSessions();
    if (source === api.backendUrl()) setSessions(res.sessions);
  } catch {
    /* backend restarting */
  }
}

async function hydrate(id: string) {
  setHydratingId(id);
  try {
    const t = await api.transcript(id);
    if (!isRunning(id)) {
      hydrateFromTranscript(id, t.messages);
      setUsageFor(id, t.usage);
    }
  } catch (error) {
    if (!isRunning(id)) {
      appendSystem(id, `Could not load this task: ${error instanceof Error ? error.message : String(error)}`);
    }
  } finally {
    setHydratingId((current) => (current === id ? null : current));
  }
}

function openStream(id: string) {
  if (streams.has(id)) return;
  const es = api.openEventStream(
    id,
    (ev) => applyEvent(id, ev, { onFinish: (s) => onFinished(id, s) }),
    () => {},
  );
  streams.set(id, es);
}

async function notify(title: string, body: string) {
  if (!uiPreferences.notifications) return;
  try {
    const n = await import("@tauri-apps/plugin-notification");
    let granted = await n.isPermissionGranted();
    if (!granted) granted = (await n.requestPermission()) === "granted";
    if (granted) n.sendNotification({ title, body });
  } catch {
    /* notifications optional */
  }
}

function onFinished(id: string, summary: string) {
  void refreshSessions();
  if (document.hidden && id === activeId()) {
    const s = sessions().find((x) => x.session_id === id);
    void notify("VakCoder run finished", `${s?.title ?? "Session"} — ${summary}`);
  }
}

export async function activate(id: string) {
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

export async function sendPrompt(
  text: string,
  goal?: { objective: string; criteria: string[] },
) {
  if (!text.trim()) return;
  // Typing into the empty state is the natural way to start: create the task
  // rather than silently dropping the prompt because nothing is selected.
  let id = activeId();
  if (!id) {
    await newSession();
    id = activeId();
    if (!id) return; // newSession already surfaced why
  }
  // Goal mode (docs/design/27 Phase H): /goal arms, bare /goal shows
  // status, /goal off disarms — mirroring the TUI.
  if (text.trim().startsWith("/goal")) {
    const arg = text.trim().slice(5).trim();
    if (!arg) {
      appendSystem(id, armedGoal ? `🎯 armed: ${armedGoal.objective} (${armedGoal.criteria.length} criteria)` : "no goal armed · usage: /goal <objective> -- c1; c2");
      return;
    }
    if (arg === "off") {
      armedGoal = null;
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
    armedGoal = { objective, criteria };
    appendSystem(id, `🎯 goal armed (${criteria.length} criteria) — next prompt will be audited`);
    return;
  }

  const thisGoal = armedGoal;
  armedGoal = null;
  appendUser(id, text);
  try {
    if (isRunning(id)) {
      await api.steer(id, text);
    } else {
      markRunning(id, true);
      try {
        await api.runPrompt(id, text, goal ?? thisGoal ?? undefined);
      } catch (e) {
        markRunning(id, false);
        throw e;
      }
    }
  } catch (e) {
    appendSystem(id, `error: ${e instanceof Error ? e.message : String(e)}`);
  }
}

export async function approve(requestId: string, ok: boolean) {
  const id = activeId();
  if (!id) return;
  try {
    await api.answerApproval(id, requestId, ok);
    resolveApproval(id, requestId, ok ? "allowed" : "denied");
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
  setSessions([]);
  setHealth(null);
  setHydratingId(null);
  setDiffTarget(null);
  setEditorPath(null);
  setDockTab(null);
  setSideOpen(false);
  setHistoryOpen(false);
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
    // Resolve the provider picture *before* publishing readiness. Flipping
    // backend() first mounts the workspace for an instant with a stale
    // setupNeeded, and any error thrown by that render would propagate out
    // of this function and leave providers unset — stranding the gate.
    try {
      const p = await api.listProviders();
      if (epoch !== backendRefreshEpoch) return false;
      setProviders(p);
      setSetupNeeded(!p.current_configured);
    } catch {
      if (epoch !== backendRefreshEpoch) return false;
      setProviders(null);
      setSetupNeeded(false);
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

/** Pick a different project folder and reboot the backend against it. */
export async function switchProject(cwd?: string) {
  if (workspaceSwitching()) return;
  try {
    const dir = cwd ?? await open({ directory: true, multiple: false, title: "Open a project" });
    if (typeof dir === "string") {
      if (dir === backend().cwd) return;
      setWorkspaceSwitching(true);
      const info = await invoke<import("./types").BackendInfo>("start_backend", { cwd: dir });
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
}

export default function App() {
  onMount(() => {
    const savedSidebar = Number(localStorage.getItem("vakcoder.sidebarWidth"));
    const savedDock = Number(localStorage.getItem("vakcoder.dockWidth"));
    if (savedSidebar >= 220 && savedSidebar <= 360) setSidebarWidth(savedSidebar);
    if (savedDock >= 340 && savedDock <= window.innerWidth * 0.7) setDockWidth(savedDock);
    // Register the listener before the first probe: the shell boots the
    // backend during setup() and emits `backend-ready` within milliseconds,
    // so subscribing after init() loses the event to the race.
    const un1 = listen<import("./types").BackendInfo>("backend-ready", (event) => {
      if (!workspaceSwitching()) void refreshBackend(event.payload);
    });
    void init();
    const sessionRefresh = window.setInterval(() => void refreshSessions(), 10_000);

    const keys = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) {
        if (e.key === "Escape") {
          if (showShortcuts()) setShowShortcuts(false);
          else if (settingsOpen()) setSettingsOpen(false);
          else if (bestOfOpen()) setBestOfOpen(false);
          else if (tasksOpen()) setTasksOpen(false);
          else if (historyOpen()) setHistoryOpen(false);
          else if (sideOpen()) setSideOpen(false);
          else stopRun();
        }
        return;
      }
      if (e.key === "n" || e.key === "N") {
        e.preventDefault();
        void newSession();
      } else if (e.key === ",") {
        e.preventDefault();
        setSettingsOpen(true);
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
      un1.then((f) => f());
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
      when={backend().ready && !setupNeeded() ? backend() : null}
      fallback={<ProjectGate />}
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
            <div class="main-stack">
              <ChatPane />
              <Show when={sideOpen()}>
                <SideChatPanel />
              </Show>
            </div>
            <Composer cwd={info().cwd ?? ""} />
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
              </div>
              </>
            )}
          </Show>
          <StatusBar />
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
          <Toast />
          <Show when={settingsOpen()}><Settings /></Show>
        </div>
      )}
    </Show>
  );
}
