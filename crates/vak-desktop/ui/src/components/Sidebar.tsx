import { createMemo, createSignal, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  activeId,
  backend,
  isRunning,
  sessions,
  setBestOfOpen,
  setShowShortcuts,
  setSidebarOpen,
  setNotice,
  setSettingsOpen,
  setTasksOpen,
} from "../store";
import { activate, newSession, refreshBackend, refreshSessions } from "../App";
import * as api from "../api";
import type { SessionSummary } from "../types";
import Icon from "./Icon";

type Filter = "all" | "active" | "idle" | "archived";

function timeLabel(iso?: string | null): string {
  if (!iso) return "";
  const date = new Date(iso);
  const seconds = (Date.now() - date.getTime()) / 1000;
  if (seconds < 60) return "now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h`;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export default function Sidebar() {
  const [filter, setFilter] = createSignal<Filter>("all");
  const [query, setQuery] = createSignal("");

  const visible = createMemo(() => {
    let list = sessions();
    if (filter() === "archived") {
      list = list.filter((session) => session.archived);
    } else {
      list = list.filter((session) => !session.archived);
      if (filter() === "active") list = list.filter((session) => session.running || isRunning(session.session_id));
      if (filter() === "idle") list = list.filter((session) => !session.running && !isRunning(session.session_id));
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
        <div class="brand" aria-label="vakcoder">
          <span class="brand-mark"><Icon name="spark" size={17} /></span>
          <span>vakcoder</span>
        </div>
        <div class="sb-head-actions">
          <button class="icon-button subtle has-tooltip" data-tooltip="Shortcuts" aria-label="Keyboard shortcuts" onClick={() => setShowShortcuts(true)}><span class="shortcut-glyph">⌘</span></button>
          <button class="icon-button subtle has-tooltip" data-tooltip="Hide sidebar ⌘B" aria-label="Hide sidebar" onClick={() => setSidebarOpen(false)}><Icon name="sidebar" /></button>
        </div>
      </div>

      <div class="sb-primary-actions">
        <button class="btn primary sb-new" onClick={() => void newSession()}>
          <Icon name="add" />
          <span>New task</span>
          <kbd>⌘N</kbd>
        </button>
        <div class="sb-action-grid">
          <button class="sb-action" onClick={() => setTasksOpen(true)}>
            <Icon name="timer" />
            <span>Automations</span>
          </button>
          <button class="sb-action" disabled={!activeId()} onClick={() => setBestOfOpen(true)}>
            <Icon name="layers" />
            <span>Compare</span>
          </button>
        </div>
      </div>

      <div class="sb-search-wrap">
        <Icon name="search" />
        <input
          class="sb-search"
          type="search"
          aria-label="Search tasks"
          placeholder="Search tasks"
          value={query()}
          onInput={(event) => setQuery(event.currentTarget.value)}
        />
      </div>

      <div class="sb-section-row">
        <span class="sb-section-title">Tasks</span>
        <div class="sb-filters" aria-label="Filter tasks">
          <For each={["all", "active", "idle", "archived"] as Filter[]}>
            {(item) => (
              <button class="filter-button" classList={{ on: filter() === item }} onClick={() => setFilter(item)}>
                {item}
              </button>
            )}
          </For>
        </div>
      </div>

      <div class="sb-list">
        <Show when={visible().length} fallback={<div class="sb-empty"><Icon name="chat" size={20} /><span>{filter() === "archived" ? "Nothing archived" : "No tasks here yet"}</span><small>{filter() === "archived" ? "Archived tasks stay in the ledger and can be restored anytime." : "Start with a clear outcome and vakcoder will handle the work."}</small></div>}>
          <For each={visible()}>
            {(session: SessionSummary) => (
              <button class="sb-item" classList={{ active: activeId() === session.session_id }} onClick={() => void activate(session.session_id)}>
                <span class="session-icon"><Icon name="chat" size={14} /></span>
                <span class="sb-item-copy">
                  <span class="sb-title">{session.title || "Untitled task"}</span>
                  <span class="sb-item-meta">
                    <Show
                      when={!session.archived}
                      fallback={<span>archived</span>}
                    >
                      <span class="dot" classList={{ run: session.running || isRunning(session.session_id) }} />
                      {session.running || isRunning(session.session_id) ? "Working" : `${session.entries ?? 0} events`}
                    </Show>
                  </span>
                </span>
                <span
                  role="button"
                  class={`sb-archive has-tooltip`}
                  data-tooltip={session.archived ? "Restore task" : "Archive task"}
                  aria-label={session.archived ? "Restore task" : "Archive task"}
                  onClick={(e) => { e.stopPropagation(); void toggleArchive(session, !session.archived); }}
                >
                  <Icon name={session.archived ? "restore" : "archive"} size={13} />
                </span>
                <span class="sb-time">{timeLabel(session.updated_at)}</span>
              </button>
            )}
          </For>
        </Show>
      </div>

      <div class="sidebar-footer">
        <button class="sidebar-settings" onClick={() => setSettingsOpen(true)}><Icon name="gear" /><span>Settings</span><kbd>⌘,</kbd></button>
      </div>
    </aside>
  );
}
