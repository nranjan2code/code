import { createEffect, createMemo, createSignal, onCleanup, Show } from "solid-js";
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
  setSearchOpen,
  setSideOpen,
  setSidebarOpen,
  sideOpen,
  sidebarOpen,
  splitId,
} from "../store";
import * as api from "../api";
import { toggleSplit } from "../App";
import Icon, { type IconName } from "./Icon";

const INBOX_POLL_MS = 20_000;

/** Badge counters stay one glyph wide: 100+ collapses to 99+. */
const countLabel = (n: number) => (n > 99 ? "99+" : String(n));

const tools: { id: "workbench" | "preview" | "diff" | "terminal" | "editor" | "pr"; label: string; icon: IconName }[] = [
  { id: "workbench", label: "Workbench", icon: "terminal" },
  { id: "preview", label: "Preview", icon: "preview" },
  { id: "diff", label: "Changes", icon: "diff" },
  { id: "terminal", label: "Terminal", icon: "code" },
  { id: "editor", label: "Editor", icon: "file" },
  { id: "pr", label: "Pull request", icon: "git" },
];
const primaryTools = tools.filter((tool) => ["workbench", "diff", "terminal"].includes(tool.id));
const secondaryTools = tools.filter((tool) => !["workbench", "diff", "terminal"].includes(tool.id));

export default function WorkspaceHeader() {
  const session = createMemo(() => sessions().find((item) => item.session_id === activeId()));
  const title = createMemo(() => session()?.title || (activeId() ? "Untitled task" : "New task"));
  const [exporting, setExporting] = createSignal(false);

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
          <button class="icon-button has-tooltip" data-tooltip="Show sidebar ⌘B" aria-label="Show sidebar" onClick={() => setSidebarOpen(true)}><Icon name="sidebar" /></button>
        </Show>
        <div class="workspace-title">
        <div class="workspace-title-row">
          <h1>{title()}</h1>
          <Show when={activeId()}>
            <span class="run-state" classList={{ active: isRunning(activeId()) }}>
              <span class="dot" classList={{ run: isRunning(activeId()) }} role="img"
                aria-label={isRunning(activeId()) ? "Running" : "Idle"} />
              {retryOf(activeId())
                ? `Retrying · attempt ${retryOf(activeId())!.attempt}`
                : isRunning(activeId())
                  ? "Working"
                  : "Ready"}
            </span>
          </Show>
        </div>
        <div class="workspace-meta">
          {activeId() ? `Task ${activeId()!.slice(0, 8)}` : "Choose a task or start a new one"}
        </div>
        </div>
      </div>
      <div class="workspace-actions" aria-label="Workspace tools">
        <div class="workspace-action-group task-actions" role="group" aria-label="Task views">
        <button
          class="icon-button has-tooltip"
          data-tooltip="Side question ⌘;"
          classList={{ on: sideOpen() }}
          aria-pressed={sideOpen()}
          aria-label="Ask a side question"
          disabled={!activeId()}
          onClick={() => setSideOpen((value) => !value)}
        >
          <Icon name="chat" />
        </button>
        <button
          class="icon-button has-tooltip"
          data-tooltip="Compare approaches"
          aria-label="Compare multiple approaches"
          disabled={!activeId()}
          onClick={() => setBestOfOpen(true)}
        >
          <Icon name="layers" />
        </button>
        <button
          class="icon-button has-tooltip"
          data-tooltip={'Split view ⌘\\'}
          classList={{ on: !!splitId() }}
          aria-pressed={!!splitId()}
          aria-label="Toggle split view"
          disabled={!activeId()}
          onClick={() => void toggleSplit()}
        >
          <Icon name="grid" />
        </button>
        </div>
        <details class="workspace-more">
          <summary class="icon-button has-tooltip" data-tooltip="More task actions" aria-label="More task actions"><Icon name="tune" /></summary>
          <div class="workspace-more-menu" role="menu">
            <button role="menuitem" onClick={() => setHistoryOpen(true)} disabled={!activeId()}><Icon name="history" />History</button>
            <button role="menuitem" onClick={() => setReceiptsOpen(true)} disabled={!activeId()}><Icon name="receipt" />Dispatch forensics</button>
            <button role="menuitem" onClick={() => setWorkOpen(true)} disabled={!activeId()}><span class="menu-letter">W</span>Managed work</button>
            <button role="menuitem" onClick={() => void exportTranscript()} disabled={!activeId() || exporting()}><Icon name="download" />Download transcript</button>
          </div>
        </details>
        <span class="action-separator" aria-hidden="true" />
        <button
          class="icon-button has-tooltip inbox-bell"
          data-tooltip="Inbox"
          classList={{ on: inboxOpen() }}
          aria-pressed={inboxOpen()}
          aria-label={inboxUnread() > 0 ? `Inbox, ${countLabel(inboxUnread())} unread` : "Inbox"}
          onClick={() => setInboxOpen(!inboxOpen())}
        >
          <Icon name="bell" />
          <Show when={inboxUnread() > 0}>
            <span class="bell-count" aria-hidden="true">{countLabel(inboxUnread())}</span>
          </Show>
        </button>
        <button
          class="icon-button has-tooltip"
          data-tooltip="Recall search ⌘K"
          aria-label="Recall search across sessions"
          onClick={() => setSearchOpen(true)}
        >
          <Icon name="search" />
        </button>
        <span class="action-separator" aria-hidden="true" />
        <div class="workspace-action-group" role="group" aria-label="Workspace surfaces">
        {primaryTools.map((tool) => (
          <button
            class={`icon-button has-tooltip tool-${tool.id}`}
            data-tooltip={tool.label}
            classList={{ on: dockTab() === tool.id }}
            aria-pressed={dockTab() === tool.id}
            aria-label={tool.label}
            onClick={() => setDockTab((current) => (current === tool.id ? null : tool.id))}
          >
            <Icon name={tool.icon} />
          </button>
        ))}
        <details class="workspace-more surface-more">
          <summary class="icon-button has-tooltip" data-tooltip="More workspace surfaces" aria-label="More workspace surfaces"><Icon name="tune" /></summary>
          <div class="workspace-more-menu" role="menu">
            {secondaryTools.map((tool) => (
              <button role="menuitem" onClick={() => setDockTab(tool.id)}><Icon name={tool.icon} />{tool.label}</button>
            ))}
          </div>
        </details>
        </div>
        <span class="action-separator" aria-hidden="true" />
        <button
          class="icon-button has-tooltip"
          data-tooltip={dockTab() ? "Close right panel" : "Open right panel"}
          aria-label={dockTab() ? "Close right panel" : "Open right panel"}
          aria-pressed={!!dockTab()}
          classList={{ on: !!dockTab() }}
          onClick={() => setDockTab((current) => current ? null : "workbench")}
        >
          <Icon name="sidebar" />
        </button>
      </div>
    </header>
  );
}
