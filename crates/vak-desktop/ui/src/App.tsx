import { createEffect, onCleanup, onMount, Show, For } from "solid-js";
import { listen } from "@tauri-apps/api/event";
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
import ProjectGate from "./components/ProjectGate";

const streams = new Map<string, EventSource>();
const sideStreams = new Map<string, EventSource>();

export async function refreshSessions() {
  try {
    const res = await api.listSessions();
    setSessions(res.sessions);
  } catch {
    /* backend restarting */
  }
}

async function hydrate(id: string) {
  try {
    const t = await api.transcript(id);
    if (!isRunning(id)) {
      hydrateFromTranscript(id, t.messages);
      setUsageFor(id, t.usage);
    }
  } catch {
    /* run in progress — live stream covers it */
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
    void notify("vakcoder run finished", `${s?.title ?? "Session"} — ${summary}`);
  }
}

export async function activate(id: string) {
  setActiveId(id);
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
    console.error(e);
  }
}

export async function sendPrompt(text: string) {
  const id = activeId();
  if (!id || !text.trim()) return;
  appendUser(id, text);
  try {
    if (isRunning(id)) {
      await api.steer(id, text);
    } else {
      markRunning(id, true);
      try {
        await api.runPrompt(id, text);
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
  try {
    setHealth(await api.health());
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

async function init() {
  const info = await api.initBackend();
  setBackend(info);
  if (info.ready) {
    await Promise.all([refreshSessions(), loadHealth()]);
  }
}

function closeAllStreams() {
  streams.forEach((es) => es.close());
  streams.clear();
}

export default function App() {
  onMount(() => {
    void init();
    const un1 = listen("backend-ready", () => {
      closeAllStreams();
      closeAllSideStreams();
      setActiveId(null);
      void init();
    });

    const keys = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) {
        if (e.key === "Escape") stopRun();
        return;
      }
      if (e.key === "n" || e.key === "N") {
        e.preventDefault();
        void newSession();
      } else if (e.key === "/") {
        e.preventDefault();
        setShowShortcuts((v) => !v);
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
      }
    };
    window.addEventListener("keydown", keys);
    onCleanup(() => {
      window.removeEventListener("keydown", keys);
      un1.then((f) => f());
      closeAllStreams();
      closeAllSideStreams();
    });
  });

  createEffect(() => {
    if (backend().ready) void loadHealth();
  });

  return (
    <Show when={backend().ready ? backend() : null} fallback={<ProjectGate />}>
      {(info) => (
        <div class="app">
          <Sidebar />
          <div class="main">
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
              <div class="dock" data-dock={tab()}>
                <div class="dock-tabs">
                  <For each={["diff", "terminal", "editor", "pr"] as const}>
                    {(t) => (
                      <button
                        class="dock-tab"
                        classList={{ on: tab() === t }}
                        onClick={() => setDockTab(t)}
                      >
                        {t}
                      </button>
                    )}
                  </For>
                  <button
                    class="dock-close"
                    title="Close pane"
                    onClick={() => setDockTab(null)}
                  >
                    ✕
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
              </div>
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
        </div>
      )}
    </Show>
  );
}
