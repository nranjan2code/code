import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
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
  presentationMode,
} from "../store";
import { activate, newSession, refreshBackend, refreshSessions, switchWorkspace } from "../App";
import { host } from "../host";
import * as api from "../api";
import type { SessionSummary } from "../types";
import { relTime } from "../time";
import Icon from "./Icon";

import ConfirmModal, { type ConfirmConfig } from "./ConfirmModal";

type Filter = "all" | "archived";

type WorkspaceGroup = { cwd: string; name: string; sessions: SessionSummary[] };

export default function Sidebar() {
  const [filter, setFilter] = createSignal<Filter>("all");
  const [everydayTasksOpen, setEverydayTasksOpen] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [searchOpen, setSearchOpen] = createSignal(false);
  const [contextMenu, setContextMenu] = createSignal<{ x: number; y: number; cwd: string; name: string } | null>(null);
  const [sessionContextMenu, setSessionContextMenu] = createSignal<{ x: number; y: number; session: SessionSummary } | null>(null);
  const [confirmConfig, setConfirmConfig] = createSignal<ConfirmConfig | null>(null);

  onMount(() => {
    const dismiss = () => {
      setContextMenu(null);
      setSessionContextMenu(null);
    };
    window.addEventListener("click", dismiss);
    onCleanup(() => window.removeEventListener("click", dismiss));
  });

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
      const projects = [backend().cwd, ...(backend().recent_workspaces ?? [])].filter(
        (cwd, index, items): cwd is string => !!cwd && items.indexOf(cwd) === index,
      );
      const needle = query().trim().toLowerCase();
      for (const cwd of projects) {
        const name = cwd.split(/[\\/]/).filter(Boolean).pop() || "Current workspace";
        if (!needle || name.toLowerCase().includes(needle)) grouped.set(cwd, []);
      }
    }
    for (const session of visible()) {
      // A session's OWN recorded cwd decides its group, not whichever
      // project happens to be open right now — the reverse (as this used
      // to read) put every session, including every one from every other
      // project, under the current workspace, so every *other* project
      // group always rendered as "No tasks" whether or not it had any.
      const cwd = session.cwd || backend().cwd || "";
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

  /// Remove a project from the list. Its history is untouched.
  const executeForgetWorkspace = async (cwd: string, name: string) => {
    try {
      // 1. If any task in this workspace is running, stop it gracefully
      const wsSessions = sessions().filter((s) => (s.cwd || backend().cwd) === cwd);
      for (const s of wsSessions) {
        if (isRunning(s.session_id) || s.running) {
          try {
            await api.cancelRun(s.session_id);
          } catch {
            // continue
          }
        }
      }

      // 2. If it's the active workspace, switch to another workspace first
      if (backend().cwd === cwd) {
        const all = groups();
        const other = all.find((g) => g.cwd !== cwd)?.cwd
          || (backend().recent_workspaces ?? []).find((c) => c !== cwd);
        if (other) {
          await switchWorkspace(other);
        } else {
          try {
            await switchWorkspace("~/vak-home");
          } catch {
            // continue with forget
          }
        }
      }
      await host.forgetWorkspace?.(cwd);
      await api.forgetWorkspace(cwd);
      await refreshBackend();
      await refreshSessions();

      // 3. If current active chat was in the removed workspace, reset canvas
      if (wsSessions.some((s) => s.session_id === activeId())) {
        void newSession();
      }

      setNotice({
        kind: "info",
        text: `Removed "${name}" from list. Any active runs were stopped. Local files and history remain safely on disk.`,
      });
    } catch (error) {
      setNotice({
        kind: "error",
        text: `Could not remove that workspace: ${error instanceof Error ? error.message : String(error)}`,
      });
    }
  };

  const requestForgetWorkspace = (cwd: string, name: string) => {
    setConfirmConfig({
      title: `Remove "${name}" from workspace list?`,
      description: `This removes the workspace from your sidebar. Your local files, git branches, commits, and session ledgers are completely safe and remain untouched on disk.`,
      detail: backend().cwd === cwd
        ? "This is your currently active workspace. Vak will safely switch you to another workspace and stop any active runs."
        : "Any tasks currently running in this workspace will be safely stopped.",
      confirmLabel: "Remove Workspace",
      cancelLabel: "Keep Workspace",
      isDanger: true,
      onConfirm: async () => {
        await executeForgetWorkspace(cwd, name);
      },
    });
  };

  const executeToggleArchive = async (session: SessionSummary, next: boolean) => {
    try {
      // If archiving a running task, stop it cleanly
      if (next && (isRunning(session.session_id) || session.running)) {
        try {
          await api.cancelRun(session.session_id);
        } catch (error) {
          throw new Error(`could not stop the active task before archiving: ${error instanceof Error ? error.message : String(error)}`);
        }
      }
      await api.setArchived(session.session_id, next);
      await refreshSessions();

      // If the archived chat is the active chat on screen, clear the canvas!
      if (next && activeId() === session.session_id) {
        const remaining = sessions().filter((s) => !s.archived && s.session_id !== session.session_id);
        if (remaining.length > 0) {
          void activate(remaining[0].session_id);
        } else {
          void newSession();
        }
      }

      setNotice({
        kind: "info",
        text: next ? "Task archived and cleared from canvas. You can view or restore it anytime in Archived." : "Task restored.",
      });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not ${next ? "archive" : "restore"} that task: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  const requestToggleArchive = (session: SessionSummary, next: boolean) => {
    if (!next) {
      void executeToggleArchive(session, false);
      return;
    }
    setConfirmConfig({
      title: `Archive "${session.title || "Untitled task"}"?`,
      description: "This task will be removed from your active sidebar list and cleared from the canvas. All message history and receipts are safely preserved in the ledger.",
      detail: (isRunning(session.session_id) || session.running)
        ? "This task is currently working. Archiving it will safely stop the active run and save partial output."
        : "You can view or restore this task anytime from the Archived section.",
      confirmLabel: "Archive Task",
      cancelLabel: "Cancel",
      isDanger: false,
      onConfirm: async () => {
        await executeToggleArchive(session, true);
      },
    });
  };

  const requestDeleteSession = (session: SessionSummary) => {
    setConfirmConfig({
      title: `Permanently delete "${session.title || "Untitled task"}"?`,
      description: "This task and its conversation events will be deleted from Vak's ledger. This action cannot be undone.",
      detail: (isRunning(session.session_id) || session.running)
        ? "This task is currently active. Deleting it will stop the run and erase its ledger."
        : "All events and receipts for this task will be permanently erased.",
      confirmLabel: "Delete Task",
      cancelLabel: "Cancel",
      isDanger: true,
      onConfirm: async () => {
        try {
          if (isRunning(session.session_id) || session.running) {
            try {
              await api.cancelRun(session.session_id);
            } catch (error) {
              throw new Error(`could not stop the active task before deletion: ${error instanceof Error ? error.message : String(error)}`);
            }
          }
          await api.deleteSession(session.session_id);
          await refreshSessions();
          if (activeId() === session.session_id) {
            const remaining = sessions().filter((s) => s.session_id !== session.session_id && !s.archived);
            if (remaining.length > 0) {
              void activate(remaining[0].session_id);
            } else {
              void newSession();
            }
          }
          setNotice({ kind: "info", text: "Task deleted from history." });
        } catch (error) {
          setNotice({ kind: "error", text: `Could not delete task: ${error instanceof Error ? error.message : String(error)}` });
        }
      },
    });
  };

  return (
    <aside class="sidebar">
      <div class="sb-head">
        <div class="brand" aria-label="Vak">
          <span class="brand-mark"><img src={`${import.meta.env.BASE_URL}vak-icon.png`} alt="" /></span>
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
          <button type="button" class="icon-button subtle has-tooltip" data-tooltip="Hide sidebar ⌘B" aria-label="Hide sidebar" onClick={() => setSidebarOpen(false)}><Icon name="sidebar" /></button>
        </div>
      </div>

      <nav class="sb-nav" aria-label="Primary">
        <button type="button" class="sb-nav-item sb-new" onClick={() => void newSession()}>
          <Icon name="add" />
          <span>{presentationMode() === "everyday" ? "New conversation" : "New task"}</span>
          <Show when={presentationMode() === "advanced"}>
            <kbd>⌘N</kbd>
          </Show>
        </button>
        <Show when={presentationMode() === "everyday"}>
          <button type="button" class="sb-nav-item" classList={{ active: !everydayTasksOpen() }} onClick={() => { setEverydayTasksOpen(false); void newSession(); }}>
            <Icon name="spark" />
            <span>Home</span>
          </button>
          <button type="button" class="sb-nav-item" classList={{ active: everydayTasksOpen() }} onClick={() => setEverydayTasksOpen(true)}>
            <Icon name="chat" />
            <span>My tasks</span>
          </button>
          <button type="button" class="sb-nav-item" onClick={() => setTasksOpen(true)}>
            <Icon name="timer" />
            <span>Reminders &amp; recurring work</span>
          </button>
          <button type="button" class="sb-nav-item" classList={{ active: filter() === "archived" }} onClick={() => setFilter(filter() === "archived" ? "all" : "archived")}>
            <Icon name="archive" />
            <span>Saved</span>
          </button>
        </Show>
        <Show when={presentationMode() === "advanced"}>
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
        </Show>
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

      <Show when={presentationMode() === "everyday"}>
        <div class="sb-list">
          <div class="sb-section-row">
            <span class="sb-section-title">{everydayTasksOpen() ? "My tasks" : "Recent"}</span>
          </div>
          <For each={everydayTasksOpen() ? visible() : visible().slice(0, 3)}>
            {(session) => (
              <div
                class="sb-item"
                classList={{ active: activeId() === session.session_id }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setSessionContextMenu({ x: e.clientX, y: e.clientY, session });
                }}
              >
                <button class="sb-item-main" onClick={() => void activate(session.session_id)}>
                  <span class="session-icon"><Icon name="chat" size={14} /></span>
                  <span class="sb-item-copy">
                    <span class="sb-title">{session.title || "Untitled conversation"}</span>
                  </span>
                </button>
              </div>
            )}
          </For>
          <Show when={!everydayTasksOpen() && visible().length > 3}>
            <div class="sb-section-row" style={{ "margin-top": "12px" }}>
              <span class="sb-section-title">Earlier</span>
            </div>
            <For each={visible().slice(3)}>
              {(session) => (
                <div
                  class="sb-item"
                  classList={{ active: activeId() === session.session_id }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setSessionContextMenu({ x: e.clientX, y: e.clientY, session });
                  }}
                >
                  <button class="sb-item-main" onClick={() => void activate(session.session_id)}>
                    <span class="session-icon"><Icon name="chat" size={14} /></span>
                    <span class="sb-item-copy">
                      <span class="sb-title">{session.title || "Untitled conversation"}</span>
                    </span>
                  </button>
                </div>
              )}
            </For>
          </Show>
        </div>
      </Show>

      <Show when={presentationMode() === "advanced"}>
        <div class="sb-section-row">
          <span class="sb-section-title">{filter() === "archived" ? "Archived" : "Workspaces"}</span>
          <Show when={filter() === "archived"}>
            <button type="button" class="sb-section-action" onClick={() => setFilter("all")}>Done</button>
          </Show>
          <Show when={filter() !== "archived"}>
            <button class="sb-section-add has-tooltip" data-tooltip="Open workspace" aria-label="Open workspace" disabled={workspaceSwitching()} onClick={() => void switchWorkspace()}><Icon name="add" size={14} /></button>
          </Show>
        </div>

        <div class="sb-list">
          <Show when={groups().length} fallback={<div class="sb-empty"><Icon name="folder" size={20} /><span>{filter() === "archived" ? "Nothing archived" : "No workspaces yet"}</span><small>{filter() === "archived" ? "Archived tasks stay in the ledger and can be restored anytime." : "Open a workspace to start a task with its files and history."}</small></div>}>
            <For each={groups()}>
              {(group) => (
                <section class="workspace-group">
                  <div
                    class="workspace-group-row"
                    classList={{ active: backend().cwd === group.cwd && filter() !== "archived" }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setContextMenu({ x: e.clientX, y: e.clientY, cwd: group.cwd, name: group.name });
                    }}
                  >
                    <button class="workspace-group-head" title={group.cwd} disabled={workspaceSwitching()} onClick={() => void switchWorkspace(group.cwd)}>
                      <Icon name="folder" size={14} />
                      <span>{group.name}</span>
                      <Show when={group.sessions.length}><small>{group.sessions.length}</small></Show>
                    </button>
                    <Show when={filter() !== "archived"}>
                      <button
                        class="workspace-forget has-tooltip"
                        data-tooltip={backend().cwd === group.cwd ? `Close & remove ${group.name} from list (keeps files)` : `Remove ${group.name} from list (keeps files)`}
                        aria-label={`Remove ${group.name} from the workspace list`}
                        disabled={workspaceSwitching()}
                        onClick={(e) => { e.stopPropagation(); void requestForgetWorkspace(group.cwd, group.name); }}
                      >
                        <Icon name="close" size={12} />
                      </button>
                    </Show>
                  </div>
                  <For each={group.sessions}>
                    {(session: SessionSummary) => (
                      <div
                        class="sb-item"
                        classList={{ active: activeId() === session.session_id }}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setSessionContextMenu({ x: e.clientX, y: e.clientY, session });
                        }}
                      >
                    <button type="button" class="sb-item-main" onClick={() => void activate(session.session_id)}>
                          <span class="session-icon"><Icon name="chat" size={14} /></span>
                          <span class="sb-item-copy">
                            <span class="sb-title">{session.title || "Untitled task"}</span>
                            <span class="sb-item-meta">
                              <Show when={!session.archived} fallback={<span>archived</span>}>
                                <span class="dot" classList={{ run: session.running || isRunning(session.session_id) }}
                                  role="img"
                                  aria-label={session.running || isRunning(session.session_id) ? "Running" : "Idle"} />
                                {session.running || isRunning(session.session_id) ? "Working" : "Conversation"}
                              </Show>
                            </span>
                          </span>
                        </button>
                        <button type="button" class="sb-view has-tooltip" data-tooltip="Read-only history" aria-label={`View transcript of ${session.title || "untitled task"}`} onClick={(e) => { e.stopPropagation(); setTranscriptViewId(session.session_id); }}>
                          <Icon name="history" size={13} />
                        </button>
                        <button type="button" class="sb-archive has-tooltip" data-tooltip={session.archived ? "Restore task" : "Archive task"} aria-label={session.archived ? "Restore task" : "Archive task"} onClick={(e) => { e.stopPropagation(); void requestToggleArchive(session, !session.archived); }}>
                          <Icon name={session.archived ? "restore" : "archive"} size={13} />
                        </button>
                        <Show when={session.archived}>
                          <button type="button" class="sb-archive has-tooltip danger" data-tooltip="Permanently delete task" aria-label={`Delete ${session.title || "untitled task"}`} onClick={(e) => { e.stopPropagation(); void requestDeleteSession(session); }}>
                            <Icon name="trash" size={13} />
                          </button>
                        </Show>
                        <span class="sb-time">{relTime(session.updated_at)}</span>
                      </div>
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
      </Show>

      <div class="sidebar-footer">
        <button type="button" class="sidebar-settings" onClick={() => { setSettingsScope("user"); setSettingsOpen(true); }}><Icon name="gear" /><span>Settings</span><kbd>⌘,</kbd></button>
        {/* Only where a session is a thing that exists. A session you
            cannot end is a problem on any machine someone else can reach,
            and the desktop has none to end. */}
        <Show when={host.logout}>
          <button
            class="sidebar-help has-tooltip"
            data-tooltip="Sign out"
            aria-label="Sign out"
            onClick={() => {
              // Called ON `host`, not through a detached reference: the
              // implementation happens not to use `this` today, and that
              // is not a property worth depending on.
              void host.logout?.().then(() => window.location.reload());
            }}
          >
            <Icon name="shield" size={13} />
          </button>
        </Show>
        <button class="sidebar-help has-tooltip" data-tooltip="Keyboard shortcuts" aria-label="Keyboard shortcuts" onClick={() => setShowShortcuts(true)}><span>?</span></button>
      </div>

      <Show when={contextMenu()}>
        {(menu) => (
          <div
            class="context-menu"
            style={{ top: `${menu().y}px`, left: `${Math.min(menu().x, window.innerWidth - 190)}px` }}
            onClick={(e) => e.stopPropagation()}
          >
            <Show when={backend().cwd !== menu().cwd}>
              <button
                class="context-menu-item"
                onClick={() => {
                  const c = menu().cwd;
                  setContextMenu(null);
                  void switchWorkspace(c);
                }}
              >
                <Icon name="folder" size={13} />
                <span>Switch to workspace</span>
              </button>
            </Show>
            <button
              class="context-menu-item"
              onClick={async () => {
                setContextMenu(null);
                try {
                  await navigator.clipboard.writeText(menu().cwd);
                  setNotice({ kind: "info", text: `Copied path to clipboard.` });
                } catch {
                  setNotice({ kind: "error", text: "Could not copy the folder path. Clipboard access was denied." });
                }
              }}
            >
              <Icon name="copy" size={13} />
              <span>Copy folder path</span>
            </button>
            <div class="context-menu-divider" />
            <button
              class="context-menu-item danger"
              onClick={() => {
                const { cwd, name } = menu();
                setContextMenu(null);
                void requestForgetWorkspace(cwd, name);
              }}
            >
              <Icon name="close" size={13} />
              <span>Remove from list</span>
            </button>
          </div>
        )}
      </Show>

      <Show when={sessionContextMenu()}>
        {(menu) => (
          <div
            class="context-menu"
            style={{ top: `${menu().y}px`, left: `${Math.min(menu().x, window.innerWidth - 190)}px` }}
            onClick={(e) => e.stopPropagation()}
          >
            <button
              class="context-menu-item"
              onClick={() => {
                const id = menu().session.session_id;
                setSessionContextMenu(null);
                void activate(id);
              }}
            >
              <Icon name="chat" size={13} />
              <span>Open task</span>
            </button>
            <button
              class="context-menu-item"
              onClick={() => {
                const id = menu().session.session_id;
                setSessionContextMenu(null);
                setTranscriptViewId(id);
              }}
            >
              <Icon name="history" size={13} />
              <span>View transcript</span>
            </button>
            <button
              class="context-menu-item"
              onClick={async () => {
                const id = menu().session.session_id;
                setSessionContextMenu(null);
                try {
                  await navigator.clipboard.writeText(id);
                  setNotice({ kind: "info", text: "Copied task ID to clipboard." });
                } catch {
                  setNotice({ kind: "error", text: "Could not copy the task ID. Clipboard access was denied." });
                }
              }}
            >
              <Icon name="copy" size={13} />
              <span>Copy task ID</span>
            </button>
            <div class="context-menu-divider" />
            <button
              class="context-menu-item"
              onClick={() => {
                const s = menu().session;
                setSessionContextMenu(null);
                void requestToggleArchive(s, !s.archived);
              }}
            >
              <Icon name={menu().session.archived ? "restore" : "archive"} size={13} />
              <span>{menu().session.archived ? "Restore task" : "Archive task"}</span>
            </button>
            <button
              class="context-menu-item danger"
              onClick={() => {
                const s = menu().session;
                setSessionContextMenu(null);
                void requestDeleteSession(s);
              }}
            >
              <Icon name="trash" size={13} />
              <span>Delete permanently</span>
            </button>
          </div>
        )}
      </Show>

      <ConfirmModal config={confirmConfig()} onClose={() => setConfirmConfig(null)} />
    </aside>
  );
}
