import { createMemo, createSignal, For, Show } from "solid-js";
import {
  activeId,
  backend,
  inboxOpen,
  inboxUnread,
  isRunning,
  sessions,
  setInboxOpen,
  setTranscriptViewId,
  setShowShortcuts,
  setSidebarOpen,
  setNotice,
  setSettingsOpen,
  setSettingsScope,
  setTasksOpen,
  workspaceSwitching,
} from "../store";
import { activate, newSession, refreshSessions, switchProject } from "../App";
import * as api from "../api";
import type { SessionSummary } from "../types";
import { relTime } from "../time";
import Icon from "./Icon";

type Filter = "all" | "archived";

type WorkspaceGroup = { cwd: string; name: string; sessions: SessionSummary[] };

export default function Sidebar() {
  const [filter, setFilter] = createSignal<Filter>("all");
  const [query, setQuery] = createSignal("");
  const [searchOpen, setSearchOpen] = createSignal(false);

  const visible = createMemo(() => {
    let list = sessions();
    if (filter() === "archived") {
      list = list.filter((session) => session.archived);
    } else {
      list = list.filter((session) => !session.archived);
    }
    const needle = query().trim().toLowerCase();
    if (needle) {
      list = list.filter(
        (session) =>
          (session.title ?? "").toLowerCase().includes(needle) ||
          session.session_id.toLowerCase().includes(needle),
      );
    }
    return list;
  });

  const groups = createMemo<WorkspaceGroup[]>(() => {
    const grouped = new Map<string, SessionSummary[]>();
    if (filter() !== "archived") {
      const projects = [backend().cwd, ...(backend().recent_projects ?? [])].filter(
        (cwd, index, items): cwd is string => !!cwd && items.indexOf(cwd) === index,
      );
      const needle = query().trim().toLowerCase();
      for (const cwd of projects) {
        const name = cwd.split(/[\\/]/).filter(Boolean).pop() || "Current workspace";
        if (!needle || name.toLowerCase().includes(needle)) grouped.set(cwd, []);
      }
    }
    for (const session of visible()) {
      const cwd = backend().cwd || session.cwd || "";
      const items = grouped.get(cwd) ?? [];
      items.push(session);
      grouped.set(cwd, items);
    }
    return Array.from(grouped, ([cwd, items]) => ({
      cwd,
      name: cwd.split(/[\\/]/).filter(Boolean).pop() || "Current workspace",
      sessions: items,
    }));
  });

  const toggleArchive = async (session: SessionSummary, next: boolean) => {
    try {
      await api.setArchived(session.session_id, next);
      await refreshSessions();
    } catch (error) {
      setNotice({ kind: "error", text: `Could not ${next ? "archive" : "restore"} that task: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  return (
    <aside class="sidebar">
      <div class="sb-head">
        <div class="brand" aria-label="Vak">
          <span class="brand-mark"><img src="/vak-icon.png" alt="" /></span>
          <span>vak</span>
        </div>
        <div class="sb-head-actions">
          <button
            class="icon-button subtle has-tooltip"
            classList={{ on: searchOpen() }}
            data-tooltip="Search tasks"
            aria-label="Search tasks"
            aria-expanded={searchOpen()}
            onClick={() => setSearchOpen((open) => !open)}
          ><Icon name="search" /></button>
          <button class="icon-button subtle has-tooltip" data-tooltip="Hide sidebar ⌘B" aria-label="Hide sidebar" onClick={() => setSidebarOpen(false)}><Icon name="sidebar" /></button>
        </div>
      </div>

      <nav class="sb-nav" aria-label="Primary">
        <button class="sb-nav-item sb-new" onClick={() => void newSession()}>
          <Icon name="add" />
          <span>New task</span>
          <kbd>⌘N</kbd>
        </button>
        <button class="sb-nav-item" onClick={() => setTasksOpen(true)}>
          <Icon name="timer" />
          <span>Automations</span>
        </button>
        <button
          class="sb-nav-item"
          classList={{ active: inboxOpen() }}
          aria-label={inboxUnread() > 0 ? `Inbox, ${inboxUnread()} unread` : "Inbox"}
          onClick={() => setInboxOpen(!inboxOpen())}
        >
          <Icon name="bell" />
          <span>Inbox</span>
          <Show when={inboxUnread() > 0}>
            <small class="sb-nav-count">{inboxUnread() > 99 ? "99+" : inboxUnread()}</small>
          </Show>
        </button>
        <button class="sb-nav-item" classList={{ active: filter() === "archived" }} onClick={() => setFilter(filter() === "archived" ? "all" : "archived")}>
          <Icon name="archive" />
          <span>Archived tasks</span>
        </button>
      </nav>

      <Show when={searchOpen()}>
        <div class="sb-search-wrap">
          <Icon name="search" />
          <input
            class="sb-search"
            type="search"
            aria-label="Search tasks"
            placeholder="Search tasks"
            value={query()}
            ref={(element) => requestAnimationFrame(() => element.focus())}
            onInput={(event) => setQuery(event.currentTarget.value)}
          />
        </div>
      </Show>

      <div class="sb-section-row">
        <span class="sb-section-title">{filter() === "archived" ? "Archived" : "Projects"}</span>
        <Show when={filter() === "archived"}>
          <button class="sb-section-action" onClick={() => setFilter("all")}>Done</button>
        </Show>
        <Show when={filter() !== "archived"}>
          <button class="sb-section-add has-tooltip" data-tooltip="Open project" aria-label="Open project" disabled={workspaceSwitching()} onClick={() => void switchProject()}><Icon name="add" size={14} /></button>
        </Show>
      </div>

      <div class="sb-list">
        <Show when={groups().length} fallback={<div class="sb-empty"><Icon name="folder" size={20} /><span>{filter() === "archived" ? "Nothing archived" : "No projects yet"}</span><small>{filter() === "archived" ? "Archived tasks stay in the ledger and can be restored anytime." : "Open a project to start a task with its files and history."}</small></div>}>
          <For each={groups()}>
            {(group) => (
              <section class="workspace-group">
                <button class="workspace-group-head" classList={{ active: backend().cwd === group.cwd && filter() !== "archived" }} title={group.cwd} disabled={workspaceSwitching()} onClick={() => void switchProject(group.cwd)}>
                  <Icon name="folder" size={14} />
                  <span>{group.name}</span>
                  <Show when={group.sessions.length}><small>{group.sessions.length}</small></Show>
                </button>
                <For each={group.sessions}>
                  {(session: SessionSummary) => (
                    <button class="sb-item" classList={{ active: activeId() === session.session_id }} onClick={() => void activate(session.session_id)}>
                      <span class="session-icon"><Icon name="chat" size={14} /></span>
                      <span class="sb-item-copy">
                        <span class="sb-title">{session.title || "Untitled task"}</span>
                        <span class="sb-item-meta">
                          <Show when={!session.archived} fallback={<span>archived</span>}>
                            <span class="dot" classList={{ run: session.running || isRunning(session.session_id) }}
                              role="img"
                              aria-label={session.running || isRunning(session.session_id) ? "Running" : "Idle"} />
                            {session.running || isRunning(session.session_id) ? "Working" : `${session.entries ?? 0} events`}
                          </Show>
                        </span>
                      </span>
                      <span role="button" class="sb-view has-tooltip" data-tooltip="Read-only history" aria-label={`View transcript of ${session.title || "untitled task"}`} onClick={(e) => { e.stopPropagation(); setTranscriptViewId(session.session_id); }}>
                        <Icon name="history" size={13} />
                      </span>
                      <span role="button" class="sb-archive has-tooltip" data-tooltip={session.archived ? "Restore task" : "Archive task"} aria-label={session.archived ? "Restore task" : "Archive task"} onClick={(e) => { e.stopPropagation(); void toggleArchive(session, !session.archived); }}>
                        <Icon name={session.archived ? "restore" : "archive"} size={13} />
                      </span>
                      <span class="sb-time">{relTime(session.updated_at)}</span>
                    </button>
                  )}
                </For>
                <Show when={!group.sessions.length}>
                  <div class="workspace-empty">No tasks</div>
                </Show>
              </section>
            )}
          </For>
        </Show>
      </div>

      <div class="sidebar-footer">
        <button class="sidebar-settings" onClick={() => { setSettingsScope("user"); setSettingsOpen(true); }}><Icon name="gear" /><span>Settings</span><kbd>⌘,</kbd></button>
        <button class="sidebar-help has-tooltip" data-tooltip="Keyboard shortcuts" aria-label="Keyboard shortcuts" onClick={() => setShowShortcuts(true)}><span>?</span></button>
      </div>
    </aside>
  );
}
