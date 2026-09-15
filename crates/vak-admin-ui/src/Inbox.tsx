/// Attention inbox suite (docs/design/29-personal-os.md P6, docs/design/31-network-resilience.md).
///
/// The store-and-forward attention layer: every gateway::deliver push records
/// an append-only entry at <home>/inbox.jsonl, so unattended signals survive
/// even with zero chat transports configured.
///
/// Read state is tracked via ack tombstones (Invariant 2: nothing is ever
/// deleted or rewritten). Monotonically growing ledger bounded by MAX_SCAN (10,000).
///
/// Zero markdown/emoji icons: styling is built from SVG status dots, tone chips,
/// and crisp typography.

import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal } from "solid-js";

import { api } from "./api";
import { PageHeader } from "./display";
import { navigate, pushToast, route } from "./store";
import { timeAgo } from "./time";
import type { InboxEntry, PendingApproval, SkillProposal } from "./types";

export type InboxFilterCategory =
  | "all"
  | "action"
  | "approvals"
  | "budget"
  | "skills"
  | "heartbeat"
  | "tasks";

export const INBOX_KIND_LABELS: Record<string, string> = {
  task_summary: "scheduled task",
  approval_pending: "needs approval",
  approval_denied: "refused",
  budget_alert: "budget alert",
  digest: "digest",
  heartbeat: "status check-in",
  proposal_opened: "new skill proposed",
};

export const INBOX_KIND_TONES: Record<string, "warning" | "danger" | "info" | "success" | "accent" | "neutral"> = {
  task_summary: "success",
  approval_pending: "warning",
  approval_denied: "danger",
  budget_alert: "danger",
  digest: "neutral",
  heartbeat: "info",
  proposal_opened: "accent",
};

export const INBOX_KIND_ACCENTS: Record<string, string> = {
  approval_pending: "#f59e0b",
  approval_denied: "#ef4444",
  budget_alert: "#f43f5e",
  proposal_opened: "#8b5cf6",
  heartbeat: "#06b6d4",
  task_summary: "#10b981",
  digest: "#64748b",
};

export function isActionNeeded(kind: string): boolean {
  return kind === "approval_pending" || kind === "budget_alert" || kind === "proposal_opened";
}

export function Inbox() {
  const [unreadOnly, setUnreadOnly] = createSignal(false);
  const [searchQuery, setSearchQuery] = createSignal("");
  const [selectedCategory, setSelectedCategory] = createSignal<InboxFilterCategory>("all");
  const [ackingId, setAckingId] = createSignal("");
  const [batchAcking, setBatchAcking] = createSignal(false);
  const [selectedEntryId, setSelectedEntryId] = createSignal<string | null>(null);

  // Live resources
  const [inboxData, { refetch: refetchInbox }] = createResource(unreadOnly, (u) => api.inbox(u));
  const [approvalsData, { refetch: refetchApprovals }] = createResource(() => api.approvals().catch(() => ({ approvals: [], total: 0 })));
  const [proposalsData, { refetch: refetchProposals }] = createResource(() => api.skillProposals().catch(() => ({ proposals: [] })));

  const refreshAll = async () => {
    await Promise.all([refetchInbox(), refetchApprovals(), refetchProposals()]);
  };

  const entries = () => inboxData()?.entries ?? [];
  const unreadCount = () => inboxData()?.unread_count ?? 0;

  // Actions count
  const actionItems = createMemo(() => entries().filter((e) => isActionNeeded(e.kind)));
  const actionCount = () => actionItems().length;
  const pendingApprovalsCount = () => entries().filter((e) => e.kind === "approval_pending").length;

  // Filtered entries
  const filteredEntries = createMemo(() => {
    const q = searchQuery().trim().toLowerCase();
    const cat = selectedCategory();

    return entries().filter((entry) => {
      // Category filter
      if (cat === "action" && !isActionNeeded(entry.kind)) return false;
      if (cat === "approvals" && entry.kind !== "approval_pending" && entry.kind !== "approval_denied") return false;
      if (cat === "budget" && entry.kind !== "budget_alert") return false;
      if (cat === "skills" && entry.kind !== "proposal_opened") return false;
      if (cat === "heartbeat" && entry.kind !== "heartbeat") return false;
      if (cat === "tasks" && entry.kind !== "task_summary" && entry.kind !== "digest") return false;

      // Text query filter
      if (q) {
        const titleMatch = entry.title.toLowerCase().includes(q);
        const bodyMatch = entry.body.toLowerCase().includes(q);
        const sessionMatch = entry.session_id?.toLowerCase().includes(q);
        const taskMatch = entry.task_id?.toLowerCase().includes(q);
        const keyMatch = entry.dedupe_key?.toLowerCase().includes(q);
        const kindMatch = (INBOX_KIND_LABELS[entry.kind] ?? entry.kind).toLowerCase().includes(q);
        if (!titleMatch && !bodyMatch && !sessionMatch && !taskMatch && !keyMatch && !kindMatch) {
          return false;
        }
      }

      return true;
    });
  });

  // Single Ack
  const handleAck = async (id: string) => {
    setAckingId(id);
    try {
      await api.inboxAck(id);
      await refetchInbox();
      pushToast("info", "Marked as read");
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setAckingId("");
    }
  };

  // Batch Ack all unread
  const handleBatchAck = async () => {
    const unreadEntries = entries();
    if (unreadEntries.length === 0) return;
    setBatchAcking(true);
    try {
      await Promise.all(unreadEntries.map((e) => api.inboxAck(e.id).catch(() => {})));
      await refetchInbox();
      pushToast("info", `Marked ${unreadEntries.length} items as read`);
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setBatchAcking(false);
    }
  };

  // Find matching pending approval by session_id
  const findApprovalForSession = (sessionId?: string | null): PendingApproval | undefined => {
    if (!sessionId) return undefined;
    return approvalsData()?.approvals.find((a) => a.session_id === sessionId);
  };

  // Quick Approval Action
  const handleResolveApproval = async (sessionId: string, requestId: string, approved: boolean, entryId: string) => {
    try {
      await api.answer(sessionId, requestId, approved, false);
      pushToast("info", approved ? "Approval granted" : "Action refused");
      await api.inboxAck(entryId).catch(() => {});
      await refreshAll();
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    }
  };

  // Quick Skill Proposal Promotion
  const handlePromoteSkill = async (proposalId: string, entryId: string) => {
    try {
      await api.promoteProposal(proposalId);
      pushToast("info", "Promoted skill proposal");
      await api.inboxAck(entryId).catch(() => {});
      await refreshAll();
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    }
  };

  return (
    <div class="view inbox-view">
      <PageHeader
        title="Attention Inbox"
        description="Store-and-forward attention layer. Actionable approvals, budget warnings, skill proposals, and unattended check-ins."
      />

      {/* 1. Executive Posture Deck (KPI Strip) */}
      <div class="inbox-posture-deck">
        {/* Card 1: Unread Attention */}
        <div class="inbox-stat-card" data-tone={unreadCount() > 0 ? "warn" : "ok"}>
          <div class="inbox-stat-head">
            <span class="inbox-stat-label">Unread Attention</span>
            <span class="inbox-stat-pill" classList={{
              "pill-ok": unreadCount() === 0,
              "pill-warn": unreadCount() > 0 && pendingApprovalsCount() === 0,
              "pill-bad": pendingApprovalsCount() > 0,
            }}>
              {unreadCount() === 0 ? "ALL CAUGHT UP" : pendingApprovalsCount() > 0 ? "BLOCKING" : "ATTENTION"}
            </span>
          </div>
          <div class="inbox-stat-value">
            {unreadCount()}
            <span class="inbox-stat-unit">signals unread</span>
          </div>
          <div class="inbox-stat-sub">
            {unreadCount() === 0
              ? "Zero unread alerts in active window."
              : `${unreadCount()} unread items captured in inbox ledger.`}
          </div>
        </div>

        {/* Card 2: Action Required */}
        <div class="inbox-stat-card" data-tone={actionCount() > 0 ? "bad" : "ok"}>
          <div class="inbox-stat-head">
            <span class="inbox-stat-label">Action Required</span>
            <span class="inbox-stat-pill" classList={{
              "pill-bad": actionCount() > 0,
              "pill-ok": actionCount() === 0,
            }}>
              {actionCount() > 0 ? "ACTION NEEDED" : "NOMINAL"}
            </span>
          </div>
          <div class="inbox-stat-value">
            {actionCount()}
            <span class="inbox-stat-unit">decisions</span>
          </div>
          <div class="inbox-stat-sub">
            {pendingApprovalsCount()} approvals · {entries().filter((e) => e.kind === "budget_alert").length} budget · {entries().filter((e) => e.kind === "proposal_opened").length} proposals
          </div>
        </div>

        {/* Card 3: Delivery Chokepoint */}
        <div class="inbox-stat-card" data-tone="info">
          <div class="inbox-stat-head">
            <span class="inbox-stat-label">Delivery Chokepoint</span>
            <span class="inbox-stat-pill pill-cyan">P6 STORE & FORWARD</span>
          </div>
          <div class="inbox-stat-value">
            Zero-Transport
            <span class="inbox-stat-unit">guarantee</span>
          </div>
          <div class="inbox-stat-sub">
            Unattended signals survive with zero chat transports configured.
          </div>
        </div>

        {/* Card 4: Ledger Retention */}
        <div class="inbox-stat-card" data-tone="neutral">
          <div class="inbox-stat-head">
            <span class="inbox-stat-label">Retention & Audit</span>
            <span class="inbox-stat-pill pill-slate">INVARIANT 2</span>
          </div>
          <div class="inbox-stat-value">
            Append-Only
            <span class="inbox-stat-unit">10k window</span>
          </div>
          <div class="inbox-stat-sub">
            Tombstone acks preserve history; never deletes or rewrites entries.
          </div>
        </div>
      </div>

      {/* 2. Filter Toolbar & Search */}
      <div class="inbox-toolbar-container">
        {/* Top bar: Search + Scope + Batch Actions */}
        <div class="inbox-controls-row">
          <div class="inbox-search-box">
            <input
              type="text"
              placeholder="Filter by title, body, session, task, or kind…"
              value={searchQuery()}
              onInput={(e) => setSearchQuery(e.currentTarget.value)}
              aria-label="Filter inbox items"
            />
            <Show when={searchQuery()}>
              <button
                type="button"
                class="inbox-search-clear"
                onClick={() => setSearchQuery("")}
                title="Clear search"
              >
                Clear
              </button>
            </Show>
          </div>

          <div class="inbox-scope-group">
            <label class="inbox-toggle-label">
              <input
                type="checkbox"
                checked={unreadOnly()}
                onChange={(e) => setUnreadOnly(e.currentTarget.checked)}
              />
              <span>Unread only</span>
            </label>
          </div>

          <div class="inbox-actions-group">
            <Show when={unreadCount() > 0}>
              <button
                type="button"
                class="button small ghost"
                disabled={batchAcking()}
                onClick={() => void handleBatchAck()}
                title="Acknowledge all unread entries in current view"
              >
                {batchAcking() ? "Acknowledging…" : "Mark all as read"}
              </button>
            </Show>
            <button
              type="button"
              class="button small ghost"
              onClick={() => void refreshAll()}
              title="Refresh attention inbox and active gates"
            >
              Refresh
            </button>
          </div>
        </div>

        {/* Category Pills Strip */}
        <div class="inbox-category-strip" role="tablist">
          <button
            type="button"
            class="inbox-cat-chip"
            classList={{ active: selectedCategory() === "all" }}
            onClick={() => setSelectedCategory("all")}
          >
            All Items
            <span class="inbox-cat-badge">{entries().length}</span>
          </button>

          <button
            type="button"
            class="inbox-cat-chip inbox-cat-action"
            classList={{ active: selectedCategory() === "action" }}
            onClick={() => setSelectedCategory("action")}
          >
            Needs Action
            <span class="inbox-cat-badge" classList={{ "badge-urgent": actionCount() > 0 }}>
              {actionCount()}
            </span>
          </button>

          <button
            type="button"
            class="inbox-cat-chip"
            classList={{ active: selectedCategory() === "approvals" }}
            onClick={() => setSelectedCategory("approvals")}
          >
            Approvals
            <span class="inbox-cat-badge">
              {entries().filter((e) => e.kind === "approval_pending" || e.kind === "approval_denied").length}
            </span>
          </button>

          <button
            type="button"
            class="inbox-cat-chip"
            classList={{ active: selectedCategory() === "budget" }}
            onClick={() => setSelectedCategory("budget")}
          >
            Budgets & FinOps
            <span class="inbox-cat-badge">
              {entries().filter((e) => e.kind === "budget_alert").length}
            </span>
          </button>

          <button
            type="button"
            class="inbox-cat-chip"
            classList={{ active: selectedCategory() === "skills" }}
            onClick={() => setSelectedCategory("skills")}
          >
            Skill Proposals
            <span class="inbox-cat-badge">
              {entries().filter((e) => e.kind === "proposal_opened").length}
            </span>
          </button>

          <button
            type="button"
            class="inbox-cat-chip"
            classList={{ active: selectedCategory() === "heartbeat" }}
            onClick={() => setSelectedCategory("heartbeat")}
          >
            Heartbeats & Status
            <span class="inbox-cat-badge">
              {entries().filter((e) => e.kind === "heartbeat").length}
            </span>
          </button>

          <button
            type="button"
            class="inbox-cat-chip"
            classList={{ active: selectedCategory() === "tasks" }}
            onClick={() => setSelectedCategory("tasks")}
          >
            Automations & Digests
            <span class="inbox-cat-badge">
              {entries().filter((e) => e.kind === "task_summary" || e.kind === "digest").length}
            </span>
          </button>
        </div>
      </div>

      {/* 3. Item List or Empty State */}
      <Show when={!inboxData.loading} fallback={<div class="empty">Loading attention ledger…</div>}>
        <Show
          when={filteredEntries().length > 0}
          fallback={
            <div class="inbox-zero-card">
              <div class="inbox-zero-dot">
                <span class="cdot cdot-done" style="width: 14px; height: 14px;" />
              </div>
              <h3 class="inbox-zero-title">
                {unreadOnly() ? "Inbox zero: all signals caught up" : "No entries match this filter"}
              </h3>
              <p class="inbox-zero-desc">
                {unreadOnly()
                  ? "Vak captures unattended approval escalations, heartbeat check-ins, FinOps budget alerts, and scheduled task summaries here."
                  : "Try clearing search filters or selecting a different category to view historical entries."}
              </p>
              <Show when={unreadOnly() && entries().length > 0}>
                <button
                  type="button"
                  class="button ghost small"
                  style="margin-top: 12px;"
                  onClick={() => setUnreadOnly(false)}
                >
                  View historical acknowledged entries ({entries().length})
                </button>
              </Show>
            </div>
          }
        >
          <div class="inbox-items-feed">
            <For each={filteredEntries()}>
              {(entry) => {
                const tone = () => INBOX_KIND_TONES[entry.kind] ?? "neutral";
                const accentColor = () => INBOX_KIND_ACCENTS[entry.kind] ?? "#64748b";
                const matchingApproval = () => findApprovalForSession(entry.session_id);
                const isSelected = () => selectedEntryId() === entry.id;

                return (
                  <div
                    class="inbox-card"
                    classList={{
                      "inbox-card-selected": isSelected(),
                      "inbox-card-urgent": isActionNeeded(entry.kind),
                    }}
                    style={{ "--card-accent": accentColor() }}
                  >
                    <div class="inbox-card-header">
                      <div class="inbox-card-header-left">
                        {/* Status Dot */}
                        <span
                          class="cdot"
                          classList={{
                            "cdot-active": entry.kind === "approval_pending",
                            "cdot-blocked": entry.kind === "budget_alert" || entry.kind === "approval_denied",
                            "cdot-done": entry.kind === "task_summary",
                            "cdot-waiting": entry.kind === "proposal_opened",
                            "cdot-held": entry.kind === "heartbeat" || entry.kind === "digest",
                          }}
                        />

                        {/* Tone Chip */}
                        <span class={`chip chip-tone-${tone()}`}>
                          {INBOX_KIND_LABELS[entry.kind] ?? entry.kind.replaceAll("_", " ")}
                        </span>

                        {/* Title */}
                        <strong class="inbox-card-title">{entry.title}</strong>
                      </div>

                      <div class="inbox-card-header-right">
                        {/* Origin State Badge */}
                        <Show when={entry.session_id}>
                          <span
                            class="chip chip-phrase"
                            data-on={entry.origin_state === "available"}
                            title={
                              entry.origin_state === "available"
                                ? "Session is warm or active in memory"
                                : "Session is stored on disk"
                            }
                          >
                            {entry.origin_state === "available" ? "session · active" : "session · disk"}
                          </span>
                        </Show>

                        {/* Timestamp */}
                        <span class="inbox-card-when" title={entry.ts}>
                          {timeAgo(entry.ts)}
                        </span>
                      </div>
                    </div>

                    {/* Body content */}
                    <div class="inbox-card-body">
                      <div class="inbox-card-snippet">{entry.body}</div>

                      {/* Metadata tags strip */}
                      <div class="inbox-meta-strip">
                        <Show when={entry.session_id}>
                          <button
                            type="button"
                            class="inbox-meta-tag"
                            onClick={() => navigate(`#/sessions/${entry.session_id}`)}
                            title="Jump to session transcript"
                          >
                            <span class="meta-tag-label">session:</span>
                            <span class="meta-tag-val">{entry.session_id}</span>
                          </button>
                        </Show>

                        <Show when={entry.task_id}>
                          <button
                            type="button"
                            class="inbox-meta-tag"
                            onClick={() => navigate("#/automations")}
                            title="Jump to automations and tasks"
                          >
                            <span class="meta-tag-label">task:</span>
                            <span class="meta-tag-val">{entry.task_id}</span>
                          </button>
                        </Show>

                        <Show when={entry.result_id}>
                          <span class="inbox-meta-tag">
                            <span class="meta-tag-label">result:</span>
                            <span class="meta-tag-val">{entry.result_id}</span>
                          </span>
                        </Show>

                        <Show when={entry.dedupe_key}>
                          <span class="inbox-meta-tag" title="Delivery deduplication key">
                            <span class="meta-tag-label">dedupe:</span>
                            <span class="meta-tag-val">{entry.dedupe_key}</span>
                          </span>
                        </Show>
                      </div>

                      {/* Interactive Gate Resolver: If approval_pending has live gate in session */}
                      <Show when={entry.kind === "approval_pending" && matchingApproval()}>
                        {(approval) => (
                          <div class="inbox-gate-box">
                            <div class="inbox-gate-header">
                              <span class="inbox-gate-title">Active Execution Gate</span>
                              <span class="inbox-gate-tool">Tool: {approval().tool}</span>
                            </div>
                            <div class="inbox-gate-reason">{approval().reason}</div>
                            <Show when={approval().args_json}>
                              <pre class="inbox-gate-args">{approval().args_json}</pre>
                            </Show>
                            <div class="inbox-gate-actions">
                              <button
                                type="button"
                                class="button small"
                                style="background: var(--ok); color: #fff; border: none;"
                                onClick={() =>
                                  void handleResolveApproval(
                                    approval().session_id,
                                    approval().request_id,
                                    true,
                                    entry.id
                                  )
                                }
                              >
                                Approve Execution
                              </button>
                              <button
                                type="button"
                                class="button small ghost"
                                style="color: var(--bad); border-color: rgba(239, 68, 68, 0.4);"
                                onClick={() =>
                                  void handleResolveApproval(
                                    approval().session_id,
                                    approval().request_id,
                                    false,
                                    entry.id
                                  )
                                }
                              >
                                Refuse
                              </button>
                              <button
                                type="button"
                                class="button small ghost"
                                onClick={() => navigate(`#/sessions/${approval().session_id}`)}
                              >
                                Open in Session
                              </button>
                            </div>
                          </div>
                        )}
                      </Show>

                      {/* Interactive Skill Proposal Action */}
                      <Show when={entry.kind === "proposal_opened" && entry.title.includes("proposal")}>
                        <div class="inbox-skill-box">
                          <div class="inbox-skill-header">
                            <span>Governed Skill Proposal Review</span>
                            <button
                              type="button"
                              class="button small ghost"
                              onClick={() => navigate("#/skills")}
                            >
                              Open Skill Registry
                            </button>
                          </div>
                        </div>
                      </Show>
                    </div>

                    {/* Action Bar Footer */}
                    <div class="inbox-card-footer">
                      <div class="inbox-card-footer-left">
                        <Show when={entry.session_id}>
                          <button
                            type="button"
                            class="button small ghost"
                            onClick={() => navigate(`#/sessions/${entry.session_id}`)}
                          >
                            View session
                          </button>
                        </Show>

                        <Show when={entry.kind === "budget_alert"}>
                          <button
                            type="button"
                            class="button small ghost"
                            onClick={() => navigate("#/finops")}
                          >
                            Inspect FinOps & Budgets
                          </button>
                        </Show>

                        <Show when={entry.kind === "task_summary"}>
                          <button
                            type="button"
                            class="button small ghost"
                            onClick={() => navigate("#/automations")}
                          >
                            View automations
                          </button>
                        </Show>

                        <Show when={entry.kind === "proposal_opened"}>
                          <button
                            type="button"
                            class="button small ghost"
                            onClick={() => navigate("#/skills")}
                          >
                            View skill proposals
                          </button>
                        </Show>
                      </div>

                      <div class="inbox-card-footer-right">
                        <button
                          type="button"
                          class="button small ghost"
                          disabled={ackingId() === entry.id}
                          onClick={() => void handleAck(entry.id)}
                          title="Append tombstone to mark entry as read (Invariant 2)"
                        >
                          {ackingId() === entry.id ? "Acknowledging…" : "Mark as read"}
                        </button>
                      </div>
                    </div>
                  </div>
                );
              }}
            </For>
          </div>
        </Show>
      </Show>

      {/* 4. Attention Layer Architecture & Invariants Card */}
      <div class="inbox-architecture-card">
        <div class="inbox-arch-head">
          <span class="inbox-arch-badge">ARCHITECTURE & INVARIANTS</span>
          <strong class="inbox-arch-title">Attention Layer Contract (docs/design/29-personal-os.md P6)</strong>
        </div>
        <div class="inbox-arch-grid">
          <div class="inbox-arch-item">
            <strong>Store-and-Forward Floor</strong>
            <p>
              Every <code>gateway::deliver</code> invocation automatically appends to{" "}
              <code>&lt;home&gt;/inbox.jsonl</code>. If Telegram, Discord, or Slack channels are down,
              or zero transports are configured, unattended signals land durably here without loss.
            </p>
          </div>
          <div class="inbox-arch-item">
            <strong>Append-Only Read State (Invariant 2)</strong>
            <p>
              Acknowledging an item appends an <code>AckLine</code> tombstone with an FNV-1a reference.
              Ledger entries are never mutated or deleted on disk, preserving complete operational provenance.
            </p>
          </div>
          <div class="inbox-arch-item">
            <strong>Anti-Nag Heartbeat Cadence (P7)</strong>
            <p>
              Proactive background check-ins run on cheap models during designated quiet windows.
              Only urgent findings generate push notifications, while routine telemetry parks quietly in the inbox.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
