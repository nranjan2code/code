import { createEffect, createMemo, createSignal, onCleanup, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
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
  setSettingsOpen,
  setSettingsScope,
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

const tools: { id: "preview" | "diff" | "terminal" | "editor" | "pr"; label: string; icon: IconName }[] = [
  { id: "preview", label: "Preview", icon: "preview" },
  { id: "diff", label: "Changes", icon: "diff" },
  { id: "terminal", label: "Terminal", icon: "terminal" },
  { id: "editor", label: "Editor", icon: "code" },
  { id: "pr", label: "Pull request", icon: "git" },
];

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
      let path: string | null = null;
      try {
        path = await saveDialog({
          title: "Save transcript",
          defaultPath: `${(title().replace(/[^\w.-]+/g, "-").trim() || "transcript")}-${id.slice(0, 8)}.md`,
          filters: [{ name: "Markdown", extensions: ["md"] }],
        });
      } catch {
        /* no native dialog — fall through to blob */
      }
      if (path) {
        await invoke("export_text_file", { path, contents: md });
        setNotice({ kind: "info", text: `Transcript saved to ${path}` });
      } else {
        const url = URL.createObjectURL(new Blob([md], { type: "text/markdown" }));
        const a = document.createElement("a");
        a.href = url;
        a.download = `${id.slice(0, 8)}.md`;
        a.click();
        URL.revokeObjectURL(url);
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
              <span class="dot" classList={{ run: isRunning(activeId()) }} />
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
        <div class="workspace-action-group" role="group" aria-label="Task views">
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
        <button
          class="icon-button has-tooltip"
          data-tooltip="Time travel ⌘H"
          aria-label="Time travel"
          disabled={!activeId()}
          onClick={() => setHistoryOpen(true)}
        >
          <Icon name="history" />
        </button>
        <button
          class="icon-button has-tooltip"
          data-tooltip="Dispatch forensics"
          aria-label="Dispatch forensics"
          disabled={!activeId()}
          onClick={() => setReceiptsOpen(true)}
        >
          <Icon name="receipt" />
        </button>
        <button
          class="icon-button has-tooltip"
          data-tooltip="Managed work"
          aria-label="Managed work"
          disabled={!activeId()}
          onClick={() => setWorkOpen(true)}
        >
          <span aria-hidden="true">W</span>
        </button>
        <button
          class="icon-button has-tooltip"
          data-tooltip="Download transcript (.md)"
          aria-label="Download transcript as markdown"
          disabled={!activeId() || exporting()}
          onClick={() => void exportTranscript()}
        >
          <Icon name="download" />
        </button>
        </div>
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
        {tools.map((tool) => (
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
        </div>
        <span class="action-separator" aria-hidden="true" />
        <button
          class="icon-button has-tooltip"
          data-tooltip="Project settings"
          aria-label="Open project settings"
          onClick={() => { setSettingsScope("project"); setSettingsOpen(true); }}
        >
          <Icon name="gear" />
        </button>
      </div>
    </header>
  );
}
