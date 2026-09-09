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
  itemsOf,
  presentationMode,
  setPresentationMode,
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
  const taskStatus = createMemo(() => {
    const id = activeId();
    if (!id) return "New task";
    if (itemsOf(id).some((item) => item.kind === "approval" && !item.resolved)) return "Needs your decision";
    if (isRunning(id)) return retryOf(id) ? "Retrying" : "Working";
    return "Ready";
  });
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
            <Show when={presentationMode() === "advanced" && activeId()}>
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
        <div class="presentation-mode-segmented" role="radiogroup" aria-label="Presentation mode">
          <button
            type="button"
            role="radio"
            class="mode-pill-btn"
            classList={{ active: presentationMode() === "everyday" }}
            aria-checked={presentationMode() === "everyday"}
            onClick={() => setPresentationMode("everyday")}
          >
            Everyday
          </button>
          <button
            type="button"
            role="radio"
            class="mode-pill-btn"
            classList={{ active: presentationMode() === "advanced" }}
            aria-checked={presentationMode() === "advanced"}
            onClick={() => setPresentationMode("advanced")}
          >
            Advanced
          </button>
        </div>

        <details class="workspace-more">
          <summary class="icon-button has-tooltip" data-tooltip="More options" aria-label="More options"><Icon name="more" size={16} /></summary>
          <div class="workspace-more-menu" role="menu">
            <Show when={activeId()}>
              <button role="menuitem" onClick={() => setSideOpen(!sideOpen())}><Icon name="chat" />Side question ⌘;</button>
              <button role="menuitem" onClick={() => setBestOfOpen(true)}><Icon name="layers" />Compare approaches</button>
              <button role="menuitem" onClick={() => void toggleSplit()}><Icon name="grid" />Split view ⌘\</button>
              <button role="menuitem" onClick={() => setHistoryOpen(true)}><Icon name="history" />History</button>
              <button role="menuitem" onClick={() => setReceiptsOpen(true)}><Icon name="receipt" />Dispatch forensics</button>
              <button role="menuitem" onClick={() => setWorkOpen(true)}><span class="menu-letter">W</span>Managed work</button>
              <button role="menuitem" onClick={() => void exportTranscript()} disabled={exporting()}><Icon name="download" />Download transcript</button>
            </Show>
            <button role="menuitem" onClick={() => setSearchOpen(true)}><Icon name="search" />Search ⌘K</button>
          </div>
        </details>
      </div>
    </header>
  );
}
