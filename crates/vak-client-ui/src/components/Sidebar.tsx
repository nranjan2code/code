import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
  activeId,
  inboxOpen,
  inboxUnread,
  isRunning,
  sessions,
  setInboxOpen,
  setNotice,
  setSettingsOpen,
  setSettingsScope,
  setShowShortcuts,
  setSidebarOpen,
  setTasksOpen,
  setTranscriptViewId,
} from "../store";
import { activate, newSession, refreshSessions } from "../App";
import * as api from "../api";
import type { SessionSummary } from "../types";
import { relTime } from "../time";
import { host } from "../host";
import Icon from "./Icon";
import ConfirmModal, { type ConfirmConfig } from "./ConfirmModal";

type Filter = "active" | "saved";

export default function Sidebar() {
  const [filter, setFilter] = createSignal<Filter>("active");
  const [showAll, setShowAll] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [searchOpen, setSearchOpen] = createSignal(false);
  const [menu, setMenu] = createSignal<{ x: number; y: number; session: SessionSummary } | null>(null);
  const [confirmConfig, setConfirmConfig] = createSignal<ConfirmConfig | null>(null);

  onMount(() => {
    const dismiss = () => setMenu(null);
    window.addEventListener("click", dismiss);
    onCleanup(() => window.removeEventListener("click", dismiss));
  });

  const visible = createMemo(() => {
    const needle = query().trim().toLowerCase();
    return sessions()
      .filter((session) => filter() === "saved" ? session.archived : !session.archived)
      .filter((session) => !needle || (session.title ?? "").toLowerCase().includes(needle));
  });

  const displayed = createMemo(() => showAll() || searchOpen() || filter() === "saved" ? visible() : visible().slice(0, 8));

  const applyArchive = async (session: SessionSummary, archived: boolean) => {
    try {
      if (archived && (session.running || isRunning(session.session_id))) await api.cancelRun(session.session_id);
      await api.setArchived(session.session_id, archived);
      await refreshSessions();
      if (archived && activeId() === session.session_id) void newSession();
      setNotice({ kind: "info", text: archived ? "Conversation saved." : "Conversation restored." });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not update this conversation: ${error instanceof Error ? error.message : String(error)}` });
    }
  };

  const requestArchive = (session: SessionSummary) => {
    if (session.archived) {
      void applyArchive(session, false);
      return;
    }
    setConfirmConfig({
      title: `Save “${session.title || "Untitled conversation"}”?`,
      description: "It will leave the recent list and remain available under Saved.",
      detail: session.running || isRunning(session.session_id) ? "Vak will stop the current work and preserve its partial result." : undefined,
      confirmLabel: "Save conversation",
      cancelLabel: "Cancel",
      isDanger: false,
      onConfirm: () => applyArchive(session, true),
    });
  };

  const requestDelete = (session: SessionSummary) => setConfirmConfig({
    title: `Delete “${session.title || "Untitled conversation"}”?`,
    description: "This permanently removes its conversation ledger and receipts.",
    detail: "This cannot be undone.",
    confirmLabel: "Delete permanently",
    cancelLabel: "Cancel",
    isDanger: true,
    onConfirm: async () => {
      try {
        if (session.running || isRunning(session.session_id)) await api.cancelRun(session.session_id);
        await api.deleteSession(session.session_id);
        await refreshSessions();
        if (activeId() === session.session_id) void newSession();
        setNotice({ kind: "info", text: "Conversation deleted." });
      } catch (error) {
        setNotice({ kind: "error", text: `Could not delete this conversation: ${error instanceof Error ? error.message : String(error)}` });
      }
    },
  });

  return (
    <aside class="sidebar">
      <div class="sb-head">
        <div class="brand" aria-label="Vak"><span class="brand-mark"><img src={`${import.meta.env.BASE_URL}vak-icon.png`} alt="" /></span><span>vak</span></div>
        <div class="sb-head-actions">
          <button class="icon-button subtle has-tooltip" classList={{ on: searchOpen() }} data-tooltip="Search" aria-label="Search conversations" aria-expanded={searchOpen()} onClick={() => setSearchOpen((open) => !open)}><Icon name="search" /></button>
          <button type="button" class="icon-button subtle has-tooltip" data-tooltip="Hide sidebar ⌘B" aria-label="Hide sidebar" onClick={() => setSidebarOpen(false)}><Icon name="sidebar" /></button>
        </div>
      </div>

      <nav class="sb-nav" aria-label="Primary">
        <button type="button" class="sb-nav-item sb-new" onClick={() => void newSession()}><Icon name="add" /><span>New conversation</span></button>
        <button type="button" class="sb-nav-item" classList={{ active: filter() === "active" && !showAll() }} onClick={() => { setFilter("active"); setShowAll(false); void newSession(); }}><Icon name="spark" /><span>Home</span></button>
        <button type="button" class="sb-nav-item" classList={{ active: filter() === "active" && showAll() }} onClick={() => { setFilter("active"); setShowAll(true); }}><Icon name="chat" /><span>Conversations</span></button>
        <button type="button" class="sb-nav-item" onClick={() => setTasksOpen(true)}><Icon name="timer" /><span>Scheduled</span></button>
        <button type="button" class="sb-nav-item" classList={{ active: inboxOpen() }} aria-label={inboxUnread() ? `Inbox, ${inboxUnread()} unread` : "Inbox"} onClick={() => setInboxOpen(!inboxOpen())}><Icon name="bell" /><span>Inbox</span><Show when={inboxUnread()}><small class="sb-nav-count">{inboxUnread() > 99 ? "99+" : inboxUnread()}</small></Show></button>
        <button type="button" class="sb-nav-item" classList={{ active: filter() === "saved" }} onClick={() => { setFilter("saved"); setShowAll(true); }}><Icon name="archive" /><span>Saved</span></button>
      </nav>

      <Show when={searchOpen()}>
        <div class="sb-search-wrap"><Icon name="search" /><input class="sb-search" type="search" aria-label="Search conversations" placeholder="Search conversations" value={query()} ref={(element) => requestAnimationFrame(() => element.focus())} onInput={(event) => setQuery(event.currentTarget.value)} /></div>
      </Show>

      <div class="sb-section-row"><span class="sb-section-title">{filter() === "saved" ? "Saved" : showAll() ? "Conversations" : "Recent"}</span></div>
      <div class="sb-list">
        <Show when={displayed().length} fallback={<div class="sb-empty"><Icon name="chat" size={20} /><span>{filter() === "saved" ? "Nothing saved yet" : "Start a conversation"}</span><small>{filter() === "saved" ? "Saved conversations remain available here." : "Ask Vak anything or hand over a task."}</small></div>}>
          <For each={displayed()}>{(session) => (
            <div class="sb-item" classList={{ active: activeId() === session.session_id }} onContextMenu={(event) => { event.preventDefault(); setMenu({ x: event.clientX, y: event.clientY, session }); }}>
              <button type="button" class="sb-item-main" onClick={() => void activate(session.session_id)}>
                <span class="session-icon"><Icon name="chat" size={14} /></span>
                <span class="sb-item-copy"><span class="sb-title">{session.title || "Untitled conversation"}</span></span>
              </button>
              <span class="sb-time">{session.running || isRunning(session.session_id) ? "Working" : relTime(session.updated_at)}</span>
            </div>
          )}</For>
        </Show>
      </div>

      <div class="sidebar-footer">
        <button type="button" class="sidebar-settings" onClick={() => { setSettingsScope("user"); setSettingsOpen(true); }}><Icon name="gear" /><span>Settings</span><kbd>⌘,</kbd></button>
        <Show when={host.logout}><button class="sidebar-help has-tooltip" data-tooltip="Sign out" aria-label="Sign out" onClick={() => void host.logout?.().then(() => window.location.reload())}><Icon name="shield" size={13} /></button></Show>
        <button class="sidebar-help has-tooltip" data-tooltip="Keyboard shortcuts" aria-label="Keyboard shortcuts" onClick={() => setShowShortcuts(true)}><span>?</span></button>
      </div>

      <Show when={menu()}>{(current) => (
        <div class="context-menu" style={{ top: `${current().y}px`, left: `${Math.min(current().x, window.innerWidth - 190)}px` }} onClick={(event) => event.stopPropagation()}>
          <button class="context-menu-item" onClick={() => { setMenu(null); setTranscriptViewId(current().session.session_id); }}><Icon name="history" size={13} /><span>View details</span></button>
          <button class="context-menu-item" onClick={() => { const session = current().session; setMenu(null); requestArchive(session); }}><Icon name={current().session.archived ? "restore" : "archive"} size={13} /><span>{current().session.archived ? "Restore" : "Save"}</span></button>
          <Show when={current().session.archived}><><div class="context-menu-divider" /><button class="context-menu-item danger" onClick={() => { const session = current().session; setMenu(null); requestDelete(session); }}><Icon name="trash" size={13} /><span>Delete permanently</span></button></></Show>
        </div>
      )}</Show>

      <ConfirmModal config={confirmConfig()} onClose={() => setConfirmConfig(null)} />
    </aside>
  );
}
