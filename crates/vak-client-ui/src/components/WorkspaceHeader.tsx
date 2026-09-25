import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show } from "solid-js";
import { host } from "../host";
import {
  activeId,
  dockTab,
  inboxOpen,
  inboxUnread,
  isRunning,
  isStopping,
  retryOf,
  sessions,
  setBestOfOpen,
  setDockTab,
  setHistoryOpen,
  setInboxOpen,
  setInboxUnread,
  setNotice,
  setReceiptsOpen,
  setWorkOpen,
  setSideOpen,
  setSidebarOpen,
  sideOpen,
  sidebarOpen,
  splitId,
  itemsOf,
  agentForSession,
  setAgentPickerOpen,
  setAgentPickerTab,
  backend,
  coworkingPresence,
} from "../store";
import * as api from "../api";
import { toggleSplit } from "../App";
import AgentMark from "./AgentMark";
import CoworkingShare from "./CoworkingShare";
import Icon, { type IconName } from "./Icon";

const INBOX_POLL_MS = 20_000;

/** Badge counters stay one glyph wide: 100+ collapses to 99+. */
const countLabel = (n: number) => (n > 99 ? "99+" : String(n));

// Split so the menu doesn't present dev-only tools (diff/terminal/editor/PR)
// with the same weight as general ones in a non-coding conversation — every
// chat gets Workbench/Preview up front; the rest sit under their own label.
const generalTools: { id: "workbench" | "preview"; label: string; icon: IconName }[] = [
  { id: "workbench", label: "Workbench", icon: "terminal" },
  { id: "preview", label: "Preview", icon: "preview" },
];
const devTools: { id: "diff" | "terminal" | "editor" | "pr"; label: string; icon: IconName }[] = [
  { id: "diff", label: "Changes", icon: "diff" },
  { id: "terminal", label: "Terminal", icon: "code" },
  { id: "editor", label: "Editor", icon: "file" },
  { id: "pr", label: "Pull request", icon: "git" },
];
const tools = [...generalTools, ...devTools];

export default function WorkspaceHeader() {
  const session = createMemo(() => sessions().find((item) => item.session_id === activeId()));
  const title = createMemo(() => agentForSession(activeId()).name);
  const titleGlyph = createMemo(() => agentForSession(activeId()).character);
  // Each Agent now has its own workspace (server-resolved per agent id), so
  // the pill must reflect the active session's cwd, not the process-global
  // one — otherwise every agent shows the same directory regardless of which
  // is selected.
  const workspaceCwd = createMemo(() => session()?.cwd || backend().cwd);
  // Setup state is read from the server on every open (store.ts): the header
  // names only what needs attention, so a ready conversation shows no status.
  const [setup] = createResource(() => api.onboarding());
  const needsService = () => { const state = setup(); return !!state && !state.core_ready && state.provider?.state === "incomplete"; };
  const taskStatus = createMemo(() => {
    const id = activeId();
    if (!id) return "";
    if (itemsOf(id).some((item) => item.kind === "approval" && !item.resolved)) return "Needs your decision";
    if (isRunning(id)) return isStopping(id) ? "Stopping…" : retryOf(id) ? "Retrying" : "Working";
    return needsService() ? "Needs an AI service" : "";
  });
  const characterState = createMemo(() => {
    const id = activeId();
    if (id && itemsOf(id).some((item) => item.kind === "approval" && !item.resolved)) return "waiting" as const;
    return isRunning(id) ? "working" as const : "idle" as const;
  });
  const [exporting, setExporting] = createSignal(false);
  const [sharing, setSharing] = createSignal(false);

  const closeMoreMenu = (event: MouseEvent) => {
    event.currentTarget instanceof HTMLElement
      && event.currentTarget.closest("details")?.removeAttribute("open");
  };

  // Unread badge shares the BudgetBanner's polling cadence; the inbox page
  // also publishes counts on its refreshes, so the two stay in sync.
  const pollUnread = async () => {
    try {
      const res = await api.inboxUnreadCount();
      setInboxUnread(res.count);
    } catch {
      /* backend restarting — keep the last known count */
    }
  };
  createEffect(() => {
    void pollUnread();
    const t = setInterval(() => void pollUnread(), INBOX_POLL_MS);
    onCleanup(() => clearInterval(t));
  });

  // Markdown transcript export (docs/design/29-personal-os.md P4): the
  // shared renderer's output is fetched from the router and written to a
  // path the user picked in the native save dialog. Without a dialog (or in
  // a plain browser build) fall back to a blob download.
  const exportTranscript = async () => {
    const id = activeId();
    if (!id || exporting()) return;
    setExporting(true);
    try {
      const md = await api.transcriptMarkdown(id);
      const name = `${(title().replace(/[^\w.-]+/g, "-").trim() || "transcript")}-${id.slice(0, 8)}.md`;
      // The host decides how bytes reach the operator: a native save
      // dialog on the desktop, a download in a browser (where the
      // filesystem on the other end of a dialog would be the wrong one).
      const outcome = await host.saveFile(name, new TextEncoder().encode(md), "text/markdown");
      if (outcome.kind === "saved") {
        setNotice({ kind: "info", text: `Transcript saved to ${outcome.path}` });
      } else if (outcome.kind === "downloaded") {
        setNotice({ kind: "info", text: `Transcript downloaded as ${name}` });
      }
    } catch (e) {
      setNotice({ kind: "error", text: `Could not export transcript: ${e instanceof Error ? e.message : String(e)}` });
    } finally {
      setExporting(false);
    }
  };

  return (
    <><header class="workspace-head">
      <div class="workspace-leading">
        <Show when={!sidebarOpen()}>
          <button type="button" class="icon-button has-tooltip" data-tooltip="Show sidebar ⌘B" aria-label="Show sidebar" onClick={() => setSidebarOpen(true)}><Icon name="sidebar" /></button>
        </Show>
        <div class="workspace-title">
          <div class="workspace-title-row" style="display: flex; align-items: center; gap: 8px;">
            <button
              type="button"
              class="agent-header-btn"
              style="display: inline-flex; align-items: center; gap: 6px; background: transparent; border: none; padding: 2px 6px; border-radius: var(--radius-sm); cursor: pointer; color: var(--text);"
              onClick={() => {
                setAgentPickerTab("fleet");
                setAgentPickerOpen(true);
              }}
              title="Switch agent"
            >
              <h1 style="margin: 0; font-size: 16px; font-weight: 600; display: inline-flex; align-items: center; gap: 6px;">
                <AgentMark character={titleGlyph()} motion={agentForSession(activeId()).animation} size={20} state={characterState()} interactive />
                <span>{title()}</span>
                <span style="font-size: 12px; opacity: 0.6;">▾</span>
              </h1>
            </button>

            <button
              type="button"
              class="target-dir-pill"
              style="display: inline-flex; align-items: center; gap: 4px; padding: 2px 7px; border-radius: 999px; background: var(--surface-raised); border: 1px solid var(--border-soft); font-size: 13px; color: var(--muted); cursor: pointer; text-decoration: none;"
              onClick={() => {
                setAgentPickerTab("target");
                setAgentPickerOpen(true);
              }}
              title={`Folder: ${workspaceCwd() || "default"}`}
              aria-label={`Folder: ${workspaceCwd() || "default"}`}
            >
              <Icon name="folder" size={12} />
              <span>{workspaceCwd() ? (workspaceCwd() as string).split("/").pop() || "root" : "workspace"}</span>
            </button>

            <Show when={activeId() && taskStatus()}>
              <span class="run-state" classList={{ active: isRunning(activeId()) }}>
                <span class="dot" classList={{ run: isRunning(activeId()) }} role="img"
                  aria-label={isRunning(activeId()) ? "Running" : "Idle"} />
                {taskStatus()}
              </span>
            </Show>
            <Show when={coworkingPresence(activeId()).length > 0}>
              <div class="coworking-presence" aria-label="People here now">
                <For each={coworkingPresence(activeId()).slice(0, 3)}>
                  {(person) => <span class="coworking-presence-person" title={`${person.display_name} is here`}>{person.display_name.slice(0, 1).toLocaleUpperCase()}</span>}
                </For>
                <span>{coworkingPresence(activeId()).length === 1 ? `${coworkingPresence(activeId())[0].display_name} is here` : `${coworkingPresence(activeId()).length} people here`}</span>
              </div>
            </Show>
          </div>
        </div>
      </div>
      <div class="workspace-actions" aria-label="Workspace tools">
        <Show when={activeId()}>
          <button type="button" class="workspace-details-button" aria-label="Share" title="Share" onClick={() => setSharing(true)}><Icon name="chat" size={14} /><span>Share</span></button>
          <button
            type="button"
            class="workspace-details-button"
            classList={{ active: Boolean(dockTab()) }}
            aria-label="Details"
            title="Details"
            aria-expanded={Boolean(dockTab())}
            onClick={() => setDockTab(dockTab() ? null : "workbench")}
          >
            <Icon name="tune" size={14} />
            <span>Details</span>
          </button>
        </Show>

        <details class="workspace-more" data-menu>
          <summary class="icon-button has-tooltip" data-tooltip="More options" aria-label="More options"><Icon name="more" size={16} /></summary>
          <div class="workspace-more-menu" role="menu">
            <Show when={activeId()}>
              <For each={generalTools}>
                {(tool) => <button type="button" role="menuitem" onClick={(event) => { closeMoreMenu(event); setDockTab(tool.id); }}><Icon name={tool.icon} />{tool.label}</button>}
              </For>
              <div class="menu-group-label">Developer</div>
              <For each={devTools}>
                {(tool) => <button type="button" role="menuitem" onClick={(event) => { closeMoreMenu(event); setDockTab(tool.id); }}><Icon name={tool.icon} />{tool.label}</button>}
              </For>
              <button type="button" role="menuitem" onClick={(event) => { closeMoreMenu(event); setSideOpen(!sideOpen()); }}><Icon name="chat" />Side question ⌘;</button>
              <button type="button" role="menuitem" onClick={(event) => { closeMoreMenu(event); setBestOfOpen(true); }}><Icon name="layers" />Compare approaches</button>
              <button type="button" role="menuitem" onClick={(event) => { closeMoreMenu(event); void toggleSplit(); }}><Icon name="grid" />Split view ⌘\</button>
              <button type="button" role="menuitem" onClick={(event) => { closeMoreMenu(event); setHistoryOpen(true); }}><Icon name="history" />History</button>
              <button type="button" role="menuitem" onClick={(event) => { closeMoreMenu(event); setReceiptsOpen(true); }}><Icon name="receipt" />Activity log</button>
              <button type="button" role="menuitem" onClick={(event) => { closeMoreMenu(event); setWorkOpen(true); }}><Icon name="sync" />Background tasks</button>
              <button type="button" role="menuitem" onClick={(event) => { closeMoreMenu(event); void exportTranscript(); }} disabled={exporting()}><Icon name="download" />Download transcript</button>
            </Show>
          </div>
        </details>
      </div>
    </header><Show when={sharing() && activeId()}>{(id) => <CoworkingShare sessionId={id()} onClose={() => setSharing(false)} />}</Show></>
  );
}
