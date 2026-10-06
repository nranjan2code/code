import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import {
  inboxUnread,
  setInboxUnread,
  setInboxOpen,
  setTasksOpen,
  setTaskFocusId,
  setReplyTarget,
} from "../store";
import * as api from "../api";
import type { InboxEntry } from "../api";
import { relAgo } from "../time";
import Icon from "./Icon";
import { activate, notifyOnce } from "../App";

const POLL_MS = 20_000;
// Server clamps to the same ceiling (DEFAULT_INBOX_LIMIT / MAX_SCAN).
const LIMIT = 200;

const KIND_CLASS: Record<string, string> = {
  task_summary: "k-green",
  approval_pending: "k-amber",
  approval_denied: "k-red",
  budget_alert: "k-red",
  digest: "k-purple",
  heartbeat: "k-blue",
  proposal_opened: "k-warm",
  routine_failed: "k-red",
  intake_match: "k-blue",
};

function kindLabel(kind: string): string {
  return ({
    task_summary: "Task result",
    routine_failed: "Automation couldn’t run",
    approval_pending: "Needs your review",
    approval_denied: "Not approved",
    budget_alert: "Usage alert",
    digest: "Digest",
    heartbeat: "Check-in",
    proposal_opened: "New suggestion",
    intake_match: "From your sources",
  } as Record<string, string>)[kind] ?? "Update";
}

/**
 * Inbox page (docs/design/29-personal-os.md P6): newest-first notifications
 * with per-kind chips, inline expansion, and ack actions. The unread dot
 * state is derived by intersecting the All and Unread windows — entries
 * themselves carry no read flag. Refreshes on the shared 20s cadence.
 */
export default function InboxPage() {
  const [entries, setEntries] = createSignal<InboxEntry[] | null>(null);
  const [unreadIds, setUnreadIds] = createSignal<ReadonlySet<string>>(new Set());
  const [filter, setFilter] = createSignal<"all" | "unread">("all");
  const [error, setError] = createSignal<string | null>(null);
  const [expanded, setExpanded] = createSignal<string | null>(null);
  const [ackingAll, setAckingAll] = createSignal(false);

  let epoch = 0;

  const refresh = async () => {
    const mine = ++epoch;
    const prevIds = new Set(unreadIds());
    try {
      if (filter() === "unread") {
        const res = await api.listInbox(LIMIT, true);
        if (mine !== epoch) return;
        setEntries(res.entries);
        setUnreadIds(new Set(res.entries.map((e) => e.id)));
        setInboxUnread(res.unread_count);
      } else {
        const [all, un] = await Promise.all([
          api.listInbox(LIMIT, false),
          api.listInbox(LIMIT, true),
        ]);
        if (mine !== epoch) return;
        setEntries(all.entries);
        setUnreadIds(new Set(un.entries.map((e) => e.id)));
        setInboxUnread(all.unread_count);
      }
      setError(null);
      // Attention-worthy new arrivals notify while the window is hidden
      // (desktop round 2); per-entry dedupe lives in notifyOnce.
      if (document.hidden) {
        for (const e of unreadIds()) {
          if (prevIds.has(e)) continue;
          const entry = (entries() ?? []).find((x) => x.id === e);
          if (!entry) continue;
          if (
            entry.kind === "approval_pending" ||
            entry.kind === "approval_denied" ||
            entry.kind === "budget_alert" ||
            entry.kind === "routine_failed" ||
            entry.kind === "heartbeat"
          ) {
            void notifyOnce(
              `inbox:${e}`,
              `Vakyartha ${entry.kind.replace(/_/g, " ")}`,
              entry.title,
            );
          }
        }
      }
    } catch (e) {
      if (mine !== epoch) return;
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  createEffect(() => {
    void refresh();
    const t = setInterval(() => void refresh(), POLL_MS);
    onCleanup(() => clearInterval(t));
  });

  const ack = async (id: string) => {
    // Optimistic read-state; a failed call is reconciled by the refetch.
    setUnreadIds((prev) => {
      const next = new Set(prev);
      next.delete(id);
      return next;
    });
    setInboxUnread((n) => Math.max(0, n - 1));
    try {
      await api.ackInbox(id);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    await refresh();
  };

  // Bulk-acks everything currently loaded; older unread entries beyond the
  // window stay untouched (the server caps what any surface may see).
  const ackAllVisible = async () => {
    const targets = (entries() ?? []).filter((e) => unreadIds().has(e.id));
    if (!targets.length || ackingAll()) return;
    setAckingAll(true);
    const results = await Promise.allSettled(targets.map((e) => api.ackInbox(e.id)));
    const failures = results.filter((r) => r.status === "rejected").length;
    setError(failures > 0 ? `${failures} notification${failures === 1 ? "" : "s"} could not be acknowledged` : null);
    setAckingAll(false);
    await refresh();
  };

  const hasVisibleUnread = () => !!(entries() ?? []).some((e) => unreadIds().has(e.id));

  const openConversation = async (entry: InboxEntry) => {
    if (!entry.session_id) return;
    await activate(entry.session_id);
    setReplyTarget({
      sessionId: entry.session_id,
      resultId: entry.result_id ?? undefined,
      label: entry.result_id ? "this result" : "this notification",
    });
  };

  return (
    <div class="inbox-page">
      <div class="inbox-head">
        <button type="button" class="btn sm inbox-back" onClick={() => setInboxOpen(false)}>
          <Icon name="chat" size={14} />
          Back to chat
        </button>
        <h2>Inbox</h2>
        <Show when={inboxUnread() > 0}>
          <span class="badge">{inboxUnread()} unread</span>
        </Show>
        <div class="inbox-tools" role="group" aria-label="Inbox filters">
          <button
            class="filter-button"
            classList={{ on: filter() === "all" }}
            aria-pressed={filter() === "all"}
            onClick={() => setFilter("all")}
          >
            All
          </button>
          <button
            class="filter-button"
            classList={{ on: filter() === "unread" }}
            aria-pressed={filter() === "unread"}
            onClick={() => setFilter("unread")}
          >
            Unread
          </button>
          <button
            class="btn sm"
            disabled={!hasVisibleUnread() || ackingAll()}
            title="Mark every loaded notification as read"
            onClick={() => void ackAllVisible()}
          >
            Mark all visible as read
          </button>
        </div>
      </div>

      <Show when={error()}>
        <div class="inbox-error" role="alert">{error()}</div>
      </Show>

      <Show
        when={entries()}
        fallback={
          <div class="inbox-skeleton" aria-hidden="true">
            <span /><span /><span /><span /><span />
          </div>
        }
      >
        {(list) => (
          <Show
            when={list().length}
            fallback={
              <div class="inbox-empty">
                <Icon name="bell" size={26} />
                <strong>Inbox zero</strong>
                <span>
                  Nothing needs your attention. Task summaries, approvals, budget alerts, and heartbeats land here when they happen.
                </span>
              </div>
            }
          >
            <div class="inbox-list">
              <For each={list()}>
                {(entry) => (
                  <article
                    class="inbox-entry"
                    classList={{ unread: unreadIds().has(entry.id), open: expanded() === entry.id }}
                  >
                    <button
                      class="inbox-row"
                      aria-expanded={expanded() === entry.id}
                      aria-label={`View ${kindLabel(entry.kind).toLowerCase()}: ${entry.title}${unreadIds().has(entry.id) ? " (unread)" : ""}`}
                      onClick={() => setExpanded((cur) => (cur === entry.id ? null : entry.id))}
                    >
                      <span class="dot inbox-dot" classList={{ on: unreadIds().has(entry.id) }} aria-hidden="true" />
                      <span class={`kind-chip ${KIND_CLASS[entry.kind] ?? ""}`}>{kindLabel(entry.kind)}</span>
                      <span class="inbox-entry-title">{entry.title}</span>
                      <time class="inbox-time" title={new Date(entry.ts).toLocaleString()}>
                        {relAgo(entry.ts)}
                      </time>
                      <Icon name="chevron" size={13} class="icon inbox-chev" />
                    </button>
                    <Show when={expanded() === entry.id}>
                      <div class="inbox-detail">
                        <p class="inbox-body">{entry.body}</p>
                        <div class="inbox-links">
                          <Show when={entry.session_id}>
                            <Show when={entry.origin_state === "unavailable"}>
                              <span class="inbox-unavailable">
                                This conversation is no longer available.
                              </span>
                            </Show>
                            <Show when={entry.kind === "approval_pending" && entry.origin_state !== "unavailable"}>
                              <button class="btn sm primary" onClick={() => void activate(entry.session_id!)}>
                                Review request
                              </button>
                            </Show>
                            <Show when={entry.kind !== "approval_pending" && entry.origin_state !== "unavailable"}>
                              <button
                                class="btn sm"
                                onClick={() => void openConversation(entry)}
                              >
                                {entry.result_id ? "Open result conversation" : "Open conversation"}
                              </button>
                            </Show>
                          </Show>
                          <Show when={entry.task_id}>
                            <button class="btn sm primary" onClick={() => { setTaskFocusId(entry.task_id!); setTasksOpen(true); }}>
                              {entry.kind === "routine_failed" ? "Review the automation" : "View the automation"}
                            </button>
                          </Show>
                          <Show when={!entry.session_id && !entry.task_id}>
                            <span class="inbox-no-action">
                              {entry.kind === "routine_failed" || entry.kind === "approval_pending"
                                ? "No follow-up action is available here"
                                : "For your information"}
                            </span>
                          </Show>
                          <button
                            class="chip sm inbox-ack"
                            disabled={!unreadIds().has(entry.id)}
                            title={unreadIds().has(entry.id) ? "Mark as read" : "Already read"}
                            onClick={() => void ack(entry.id)}
                          >
                            {unreadIds().has(entry.id) ? "Mark as read" : "Marked as read"}
                          </button>
                        </div>
                      </div>
                    </Show>
                  </article>
                )}
              </For>
            </div>
          </Show>
        )}
      </Show>
    </div>
  );
}
