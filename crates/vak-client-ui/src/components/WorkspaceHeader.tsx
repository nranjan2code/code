import { createEffect, createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import { host } from "../host";
import {
  activeId,
  dockTab,
  inboxOpen,
  inboxUnread,
  isRunning,
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
} from "../store";
import * as api from "../api";
import { toggleSplit } from "../App";
import AgentMark from "./AgentMark";
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
  const taskStatus = createMemo(() => {
    const id = activeId();
    if (!id) return "New task";
    if (itemsOf(id).some((item) => item.kind === "approval" && !item.resolved)) return "Needs your decision";
    if (isRunning(id)) return retryOf(id) ? "Retrying" : "Working";
    return "Ready";
  });
  const [exporting, setExporting] = createSignal(false);

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
      const outcome = await host.saveText(name, md, "text/markdown");
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
    <header class="workspace-head">
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
              title="Switch Agent Specialist"
            >
              <h1 style="margin: 0; font-size: 15px; font-weight: 600; display: inline-flex; align-items: center; gap: 6px;">
                <AgentMark character={titleGlyph()} size={20} working={isRunning(activeId())} />
                <span>{title()}</span>
                <span style="font-size: 11px; opacity: 0.6;">▾</span>
              </h1>
            </button>

            <button
              type="button"
              class="target-dir-pill"
              style="display: inline-flex; align-items: center; gap: 4px; padding: 2px 7px; border-radius: 999px; background: var(--surface-raised); border: 1px solid var(--border-soft); font-size: 11.5px; color: var(--muted); cursor: pointer; text-decoration: none;"
              onClick={() => {
                setAgentPickerTab("target");
                setAgentPickerOpen(true);
              }}
              title={`Project Working Directory: ${workspaceCwd() || "default"}`}
              aria-label={`Project Working Directory: ${workspaceCwd() || "default"}`}
            >
              <Icon name="folder" size={12} />
              <span>{workspaceCwd() ? (workspaceCwd() as string).split("/").pop() || "root" : "workspace"}</span>
            </button>

            <Show when={activeId()}>
              <span class="run-state" classList={{ active: isRunning(activeId()) }}>
                <span class="dot" classList={{ run: isRunning(activeId()) }} role="img"
                  aria-label={isRunning(activeId()) ? "Running" : "Idle"} />
                {taskStatus()}
              </span>
            </Show>
          </div>
        </div>
      </div>
      <div class="workspace-actions" aria-label="Workspace tools">
        <Show when={activeId()}>
          <button
            type="button"
            class="workspace-details-button"
            classList={{ active: Boolean(dockTab()) }}
            aria-expanded={Boolean(dockTab())}
            onClick={() => setDockTab(dockTab() ? null : "workbench")}
          >
            <Icon name="tune" size={14} />
            <span>Details</span>
          </button>
        </Show>

        <details class="workspace-more">
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
              <button type="button" role="menuitem" onClick={(event) => { closeMoreMenu(event); setReceiptsOpen(true); }}><Icon name="receipt" />Activity receipts</button>
              <button type="button" role="menuitem" onClick={(event) => { closeMoreMenu(event); setWorkOpen(true); }}><span class="menu-letter">W</span>Background tasks</button>
              <button type="button" role="menuitem" onClick={(event) => { closeMoreMenu(event); void exportTranscript(); }} disabled={exporting()}><Icon name="download" />Download transcript</button>
            </Show>
          </div>
        </details>
      </div>
    </header>
  );
}
