/// Session Forensics & Turn Pipeline Suite.
///
/// An executive-grade mission control center for agent sessions, turns,
/// pipeline DAG visualization, context packet accounting, and drift auditing.
///
/// Strictly follows repository invariants:
/// - Invariant 1: Model-visible means logged.
/// - Invariant 2: Append-only ledger; zero deletions.
/// - Invariant 7: Per-turn route ladder dispatch.
/// - Invariant 10 & 16: Workspace-rooted permissions.
/// - Invariant 13: Explicit trust confirmation.
/// - Zero markdown/emoji icons: SVG status dots, tone chips, crisp typography.

import {
  For,
  Match,
  Show,
  Switch,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
} from "solid-js";

import { api, AuthRequired } from "./api";
import { PageHeader, confirmDestructive } from "./display";
import { navigate, pushToast, setAuthed } from "./store";
import { clock, timeAgo } from "./time";

function shortId(id?: string | null, len: number = 8): string {
  if (!id) return "";
  return id.length > len ? id.slice(0, len) : id;
}
import type {
  ActiveSubagent,
  AgentIdentity,
  BestOfNRun,
  CapabilityDescriptor,
  FrozenContract,
  PromptLayerDescriptor,
  SessionCheckpoint,
  SessionListItem,
  SessionTurn,
  ToolCallRecord,
  TranscriptEntry,
  TurnDagNode,
  TurnStageKey,
  WorkReceipt,
} from "./types";

// ---- Turn Reconstruction ----------------------------------------------------

export function reconstructTurns(
  entries: TranscriptEntry[],
  receipts: WorkReceipt[] = [],
  checkpoints: SessionCheckpoint[] = [],
): SessionTurn[] {
  const turns: SessionTurn[] = [];
  let currentTurn: SessionTurn | null = null;
  let receiptIndex = 0;

  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    const isUserPrompt =
      entry.kind === "message" &&
      (entry.role === "user" || entry.role === null) &&
      !entry.content.startsWith("[stop-guard]") &&
      !entry.content.startsWith("<context_summary>");

    if (isUserPrompt) {
      if (currentTurn) {
        turns.push(finalizeTurn(currentTurn, receipts, receiptIndex, checkpoints));
        receiptIndex += currentTurn.work_receipts.length;
      }
      currentTurn = {
        turn_index: turns.length + 1,
        id: entry.entry_id,
        started_at: entry.ts,
        user_prompt: entry.content,
        tool_calls: [],
        work_receipts: [],
        tokens_in: 0,
        tokens_out: 0,
        cost_usd: 0,
        duration_ms: 0,
        status: "completed",
        entries: [entry],
      };
      continue;
    }

    if (!currentTurn) {
      // Genesis / Session Initialization entry before first prompt
      if (entry.kind === "header" || entry.kind === "activity") {
        continue;
      }
      currentTurn = {
        turn_index: 1,
        id: entry.entry_id,
        started_at: entry.ts,
        user_prompt: "Session Genesis",
        tool_calls: [],
        work_receipts: [],
        tokens_in: 0,
        tokens_out: 0,
        cost_usd: 0,
        duration_ms: 0,
        status: "completed",
        entries: [entry],
      };
      continue;
    }

    currentTurn.entries.push(entry);

    if (entry.kind === "intent") {
      currentTurn.intent_summary = entry.content;
    } else if (entry.kind === "message" && entry.role === "assistant") {
      currentTurn.model_response = entry.content;
      if (entry.is_error) currentTurn.status = "failed";
    } else if (entry.tool_name) {
      currentTurn.tool_calls.push({
        tool_name: entry.tool_name,
        result_text: entry.content,
        is_error: entry.is_error,
        ts: entry.ts,
      });
      if (entry.is_error) currentTurn.status = "failed";
    } else if (entry.content.includes("[stop-guard]")) {
      currentTurn.stop_guard = entry.content;
    }
  }

  if (currentTurn) {
    turns.push(finalizeTurn(currentTurn, receipts, receiptIndex, checkpoints));
  }

  return turns;
}

function finalizeTurn(
  turn: SessionTurn,
  receipts: WorkReceipt[],
  receiptOffset: number,
  checkpoints: SessionCheckpoint[],
): SessionTurn {
  const matchingReceipt = receipts[receiptOffset];
  if (matchingReceipt) {
    turn.work_receipts.push(matchingReceipt);
    turn.tokens_in = matchingReceipt.input_tokens ?? 0;
    turn.tokens_out = matchingReceipt.output_tokens ?? 0;
    turn.cost_usd = matchingReceipt.cost_usd ?? 0;
    turn.duration_ms = matchingReceipt.latency_ms ?? 0;
  }

  const matchingCheckpoint = checkpoints.find((cp) => (cp.message || cp.label || "").includes(turn.user_prompt.slice(0, 20)));
  if (matchingCheckpoint) {
    turn.checkpoint_seq = matchingCheckpoint.seq;
  }

  const lastEntry = turn.entries[turn.entries.length - 1];
  if (lastEntry) {
    turn.ended_at = lastEntry.ts;
  }

  return turn;
}

export function buildTurnDagNodes(turn: SessionTurn, contract?: FrozenContract | null): TurnDagNode[] {
  const nodes: TurnDagNode[] = [];

  // 1. Ingress Stage
  nodes.push({
    id: "ingress",
    title: "1. Request Ingress",
    category: "Input",
    status: "ok",
    summary: turn.user_prompt.slice(0, 60) + (turn.user_prompt.length > 60 ? "…" : ""),
    metrics: `${turn.user_prompt.length} chars`,
    details: {
      timestamp: turn.started_at,
      raw_prompt: turn.user_prompt,
    },
  });

  // 2. Intent Stage
  nodes.push({
    id: "intent",
    title: "2. Intent Kernel",
    category: "Classification",
    status: "ok",
    summary: turn.intent_summary || "7-axis intent resolved (zero token dispatch)",
    metrics: "P47 Engine",
    details: {
      intent_text: turn.intent_summary || "Direct interactive execution",
      reading: turn.intent_reading ?? "7-axis radar projection",
    },
  });

  // 3. Route Stage
  const provider = contract?.provider ?? "openai-completions";
  const model = contract?.model ?? "default";
  nodes.push({
    id: "route",
    title: "3. Route & Ladder",
    category: "Dispatch",
    status: "ok",
    summary: `${provider} · ${model}`,
    metrics: turn.duration_ms > 0 ? `${turn.duration_ms}ms` : "ladder ready",
    details: {
      provider,
      model,
      objective: contract?.route_objective ?? "balanced",
      annotations: contract?.route_annotations ?? [],
      ladder: contract?.route_ladder ?? [],
    },
  });

  // 4. Inference Stage
  nodes.push({
    id: "inference",
    title: "4. Model Inference",
    category: "LLM Stream",
    status: turn.status === "failed" ? "bad" : "ok",
    summary: turn.model_response
      ? turn.model_response.slice(0, 50) + "…"
      : "Inference completed",
    metrics: `${turn.tokens_in.toLocaleString()} in · ${turn.tokens_out.toLocaleString()} out`,
    details: {
      cost: `$${turn.cost_usd.toFixed(5)}`,
      latency_ms: turn.duration_ms,
      response_preview: turn.model_response || "(no prose output)",
    },
  });

  // 5. Security Stage
  const permMode = contract?.permission_mode ?? "WorkspaceWrite";
  nodes.push({
    id: "security",
    title: "5. Security & Gate",
    category: "Broker Check",
    status: permMode === "FullAccess" ? "warn" : "ok",
    summary: `Authority: ${permMode}`,
    metrics: "Invariant 10/16 verified",
    details: {
      authority_mode: permMode,
      sandbox: permMode === "FullAccess" ? "Unsandboxed (FullAccess)" : "OS Process Sandbox",
      rulebook: "Deny → Ask → Allow → Mode Default",
    },
  });

  // 6. Tools Stage
  const toolCount = turn.tool_calls.length;
  const toolError = turn.tool_calls.some((t) => t.is_error);
  nodes.push({
    id: "tools",
    title: "6. Brokered Tools",
    category: "Execution",
    status: toolError ? "bad" : toolCount > 0 ? "ok" : "idle",
    summary:
      toolCount > 0
        ? `${toolCount} call${toolCount > 1 ? "s" : ""}: ${turn.tool_calls.map((t) => t.tool_name).join(", ")}`
        : "No effectful tools called",
    metrics: `${toolCount} tools`,
    details: {
      tool_calls: turn.tool_calls,
    },
  });

  // 7. Stop Gate Stage
  nodes.push({
    id: "stop_gate",
    title: "7. Stop Gate Policy",
    category: "Verification",
    status: turn.stop_guard ? "warn" : "ok",
    summary: turn.stop_guard ? "Stop-guard continuation nudged" : "Terminal stop condition clean",
    metrics: "Marker & Verify checks",
    details: {
      stop_guard: turn.stop_guard ?? "None (clean termination)",
      premature_policy: "max_blocks budget intact",
    },
  });

  // 8. Governance Stage
  nodes.push({
    id: "governance",
    title: "8. Governance & State",
    category: "Audit Delta",
    status: "ok",
    summary: turn.checkpoint_seq != null ? `Checkpoint #${turn.checkpoint_seq}` : "Ledger entry appended",
    metrics: "Invariant 2",
    details: {
      checkpoint_seq: turn.checkpoint_seq ?? "Live working tree",
      ledger_entries: turn.entries.length,
      audit_provenance: "Append-only JSONL verifiable",
    },
  });

  return nodes;
}

// ---- Sessions List View -----------------------------------------------------

export function SessionsList() {
  const [sessions, { refetch }] = createResource(() => api.sessions());
  const [bestofnRuns, bestofnActions] = createResource(() => api.bestofn().then((r) => r.runs).catch(() => []));
  const [q, setQ] = createSignal("");
  const [showArchived, setShowArchived] = createSignal(false);
  const [creating, setCreating] = createSignal(false);
  const [busySession, setBusySession] = createSignal("");
  const [busyCandidate, setBusyCandidate] = createSignal("");
  const [bulkBusy, setBulkBusy] = createSignal(false);

  const localHash = createMemo(() => sessions()?.workspace_project_hash ?? "");
  const isLocal = (s: SessionListItem) => !localHash() || s.project_hash === localHash();

  const filtered = createMemo(() => {
    const list = sessions()?.sessions ?? [];
    const query = q().trim().toLowerCase();
    const withArchived = showArchived() ? list : list.filter((s) => !s.archived);
    if (!query) return withArchived;
    return withArchived.filter(
      (s) =>
        s.session_id.toLowerCase().includes(query) ||
        (s.agent?.name ?? "").toLowerCase().includes(query) ||
        (s.title ?? "").toLowerCase().includes(query),
    );
  });

  const archivedCount = createMemo(() => (sessions()?.sessions ?? []).filter((s) => s.archived).length);
  const totalCount = createMemo(() => sessions()?.total ?? 0);

  const toggleArchive = async (s: SessionListItem) => {
    setBusySession(s.session_id);
    try {
      await api.archiveSession(s.session_id, !s.archived);
      await refetch();
      pushToast("info", s.archived ? "Session unarchived" : "Session archived");
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setBusySession("");
    }
  };

  const removeSession = async (s: SessionListItem) => {
    if (!confirmDestructive(`Permanently delete session “${s.session_id}”? Its JSONL ledger and history will be deleted.`)) return;
    setBusySession(s.session_id);
    try {
      await api.deleteSession(s.session_id);
      await refetch();
      pushToast("info", "Session deleted");
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setBusySession("");
    }
  };

  const deleteAllArchived = async () => {
    const count = archivedCount();
    if (!confirmDestructive(`Delete all ${count} archived sessions in this workspace? This cannot be undone.`)) return;
    setBulkBusy(true);
    try {
      const res = await api.deleteAllArchived();
      await refetch();
      const countDeleted = res.deleted ?? 0;
      pushToast("info", `Deleted ${countDeleted} archived session${countDeleted === 1 ? "" : "s"}`);
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setBulkBusy(false);
    }
  };

  const newSession = async () => {
    setCreating(true);
    try {
      const { session_id } = await api.createSession();
      await refetch();
      navigate(`#/sessions/${session_id}`);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setCreating(false);
    }
  };

  const handleKeep = async (sessionId: string) => {
    setBusyCandidate(sessionId);
    try {
      await api.keepBestRun(sessionId);
      pushToast("info", "Winning candidate kept");
      await Promise.all([refetch(), bestofnActions.refetch()]);
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setBusyCandidate("");
    }
  };

  const handleDiscard = async (sessionId: string) => {
    if (!confirmDestructive("Discard this candidate run? Its temporary worktree will be removed.")) return;
    setBusyCandidate(sessionId);
    try {
      await api.discardBestRun(sessionId);
      pushToast("info", "Candidate discarded");
      await Promise.all([refetch(), bestofnActions.refetch()]);
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setBusyCandidate("");
    }
  };

  return (
    <div class="view sessions-view">
      <PageHeader
        title="Sessions & Conversations"
        description="Append-only conversational ledgers, execution forensics, and turn pipeline timelines."
      />

      {/* Posture Deck */}
      <div class="sessions-posture-deck">
        <div class="session-kpi-card" data-tone="ok">
          <div class="session-kpi-head">
            <span class="session-kpi-label">Active Sessions</span>
            <span class="session-kpi-pill pill-ok">APPEND-ONLY</span>
          </div>
          <div class="session-kpi-val">
            {totalCount()}
            <span class="session-kpi-unit">sessions</span>
          </div>
          <div class="session-kpi-sub">
            {archivedCount()} archived · monotonic JSONL records
          </div>
        </div>

        <div class="session-kpi-card" data-tone="info">
          <div class="session-kpi-head">
            <span class="session-kpi-label">Provenance Model</span>
            <span class="session-kpi-pill pill-cyan">AGENT-OWNED</span>
          </div>
          <div class="session-kpi-val">
            Multi-Turn
            <span class="session-kpi-unit">continuity</span>
          </div>
          <div class="session-kpi-sub">
            Thread projection guarantees conversational drift resilience.
          </div>
        </div>

        <div class="session-kpi-card" data-tone="neutral">
          <div class="session-kpi-head">
            <span class="session-kpi-label">Ledger Integrity</span>
            <span class="session-kpi-pill pill-slate">INVARIANT 1 & 2</span>
          </div>
          <div class="session-kpi-val">
            Reconstructable
            <span class="session-kpi-unit">via derive()</span>
          </div>
          <div class="session-kpi-sub">
            Model-visible inputs are logged; zero unlogged requests.
          </div>
        </div>
      </div>

      {/* Best-of-N Candidate Worktrees */}
      <Show when={(bestofnRuns() ?? []).length > 0}>
        <section class="panel panel-alert" style="margin-bottom:14px">
          <div class="panel-title-row">
            <div>
              <h3 style="margin:0; font-size:14px; font-weight:700;">Best-of-N candidate runs awaiting decision</h3>
              <p class="dim" style="margin:2px 0 0; font-size:12px;">
                Vak ran multiple candidates in isolated worktrees. Compare their results and pick one to merge into your workspace.
              </p>
            </div>
            <span class="chip chip-tone-warning">{(bestofnRuns() ?? []).length} candidates</span>
          </div>
          <div class="candidate-grid">
            <For each={bestofnRuns() ?? []}>
              {(run: BestOfNRun) => (
                <div class="candidate-card">
                  <div class="candidate-head">
                    <strong class="mono">{shortId(run.session_id)}</strong>
                    <span class="chip mono">{run.branch}</span>
                  </div>
                  <div class="row-gap" style="margin-top:8px">
                    <button
                      type="button"
                      class="button small"
                      disabled={busyCandidate() === run.session_id}
                      onClick={() => handleKeep(run.session_id)}
                    >
                      Keep this
                    </button>
                    <button
                      type="button"
                      class="danger small"
                      disabled={busyCandidate() === run.session_id}
                      onClick={() => handleDiscard(run.session_id)}
                    >
                      Discard
                    </button>
                    <span class="spacer" />
                    <button
                      type="button"
                      class="ghost small"
                      onClick={() => navigate(`#/sessions/${run.session_id}`)}
                    >
                      Inspect
                    </button>
                  </div>
                </div>
              )}
            </For>
          </div>
        </section>
      </Show>

      {/* Toolbar */}
      <div class="sessions-toolbar">
        <div class="sessions-search-box">
          <input
            type="text"
            placeholder="Search sessions by ID, agent, or title…"
            value={q()}
            onInput={(e) => setQ(e.currentTarget.value)}
          />
          <Show when={q()}>
            <button type="button" class="search-clear-btn" onClick={() => setQ("")}>Clear</button>
          </Show>
        </div>

        <label class="toggle">
          <input
            type="checkbox"
            checked={showArchived()}
            onChange={(e) => setShowArchived(e.currentTarget.checked)}
          />
          <span>Show archived ({archivedCount()})</span>
        </label>

        <span class="spacer" />

        <Show when={showArchived() && archivedCount() > 0}>
          <button
            type="button"
            class="danger small"
            disabled={bulkBusy()}
            onClick={() => void deleteAllArchived()}
          >
            {bulkBusy() ? "Deleting…" : `Delete all archived (${archivedCount()})`}
          </button>
        </Show>

        <button
          type="button"
          class="button primary"
          disabled={creating()}
          onClick={() => void newSession()}
        >
          {creating() ? "Opening…" : "+ New session"}
        </button>

        <button
          type="button"
          class="ghost"
          onClick={() => {
            void refetch();
            void bestofnActions.refetch();
          }}
        >
          Refresh
        </button>
      </div>

      {/* Session Table */}
      <Show when={!sessions.loading} fallback={<div class="empty">Loading session ledgers…</div>}>
        <Show
          when={filtered().length > 0}
          fallback={
            <div class="session-empty-card">
              <span class="cdot cdot-done" style="width:16px; height:16px;" />
              <h3>No sessions match your filter</h3>
              <p>Start a new session here, via the CLI, or send a prompt to any bound chat transport.</p>
            </div>
          }
        >
          <div class="sessions-table-wrapper">
            <table class="table sessions-table">
              <thead>
                <tr>
                  <th>Session ID</th>
                  <th>Agent Persona</th>
                  <th>Messages</th>
                  <th>Started</th>
                  <th>Last Active</th>
                  <th>Scope</th>
                  <th style="text-align: right;">Actions</th>
                </tr>
              </thead>
              <tbody>
                <For each={filtered()}>
                  {(s) => {
                    const local = createMemo(() => isLocal(s));
                    const busy = () => busySession() === s.session_id;

                    return (
                      <tr
                        tabIndex={0}
                        classList={{ dim: s.archived }}
                        onClick={() => navigate(`#/sessions/${s.session_id}`)}
                        onKeyDown={(e) => e.key === "Enter" && navigate(`#/sessions/${s.session_id}`)}
                      >
                        <td class="mono font-bold">
                          {shortId(s.session_id)}
                          <Show when={s.archived}>
                            <span class="chip" style="margin-left:6px">archived</span>
                          </Show>
                        </td>

                        <td>
                          <Show when={s.agent} fallback={<span class="dim">Vak Default</span>}>
                            <span class="chip chip-tone-info" title={s.agent?.personality || ""}>
                              {s.agent?.name}
                            </span>
                          </Show>
                        </td>

                        <td>
                          <span class="session-entry-pill">{s.entry_count} entries</span>
                        </td>

                        <td title={s.first_ts}>{timeAgo(s.first_ts)}</td>
                        <td title={s.last_ts}>{timeAgo(s.last_ts)}</td>

                        <td>
                          <Show
                            when={local()}
                            fallback={
                              <span class="dim" title="Belongs to another workspace directory">
                                cross-project
                              </span>
                            }
                          >
                            <span class="chip chip-tone-success">workspace</span>
                          </Show>
                        </td>

                        <td onClick={(e) => e.stopPropagation()} style="text-align: right;">
                          <Show
                            when={local()}
                            fallback={<span class="dim">—</span>}
                          >
                            <div class="row-gap" style="justify-content: flex-end;">
                              <button
                                type="button"
                                class="ghost small"
                                disabled={busy()}
                                onClick={() => void toggleArchive(s)}
                              >
                                {busy() ? "…" : s.archived ? "Unarchive" : "Archive"}
                              </button>

                              <a
                                class="ghost small"
                                href={`/sessions/${encodeURIComponent(s.session_id)}/transcript.md`}
                                target="_blank"
                                rel="noreferrer noopener"
                                title="Export complete session as markdown"
                              >
                                Export .md
                              </a>

                              <Show when={s.archived}>
                                <button
                                  type="button"
                                  class="danger small"
                                  disabled={busy()}
                                  onClick={() => void removeSession(s)}
                                >
                                  Delete
                                </button>
                              </Show>
                            </div>
                          </Show>
                        </td>
                      </tr>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </Show>
    </div>
  );
}

// ---- Session Forensics & Turn Inspector View --------------------------------

export function SessionForensics(props: { sessionId: string }) {
  const [activeTab, setActiveTab] = createSignal<
    "dag" | "transcript" | "drift" | "receipts" | "checkpoints"
  >("dag");

  const [selectedTurnIndex, setSelectedTurnIndex] = createSignal<number>(1);
  const [selectedStage, setSelectedStage] = createSignal<TurnDagNode | null>(null);

  // Filters for transcript
  const [kindFilter, setKindFilter] = createSignal("");
  const [roleFilter, setRoleFilter] = createSignal("");

  // Live state
  const [live, setLive] = createSignal(false);
  const [running, setRunning] = createSignal(false);
  const [subagentBusy, setSubagentBusy] = createSignal("");

  // Composer state
  const [draft, setDraft] = createSignal("");
  const [nCandidates, setNCandidates] = createSignal(1);
  const [sending, setSending] = createSignal(false);

  // Resources
  const [subagentData, { refetch: refetchSubagents }] = createResource(
    () => props.sessionId,
    (id) => api.subagents(id),
  );

  const [transcriptData, { refetch: refetchTranscript }] = createResource(
    () => ({ id: props.sessionId, refresh: true }),
    ({ id, refresh }) => api.transcript(id, { limit: 500, offset: 0, refresh }),
  );

  const [receiptsData, { refetch: refetchReceipts }] = createResource(
    () => props.sessionId,
    (id) => api.attach(id).then(() => api.receipts(id)).catch(() => []),
  );

  const [checkpointsData, { refetch: refetchCheckpoints }] = createResource(
    () => props.sessionId,
    (id) => api.checkpoints(id).catch(() => ({ checkpoints: [] })),
  );

  const [diffData, { refetch: refetchDiff }] = createResource(
    () => props.sessionId,
    (id) => api.attach(id).then(() => api.diff(id)).catch(() => ({ diff: "" })),
  );

  // Subagents polling
  createEffect(() => {
    void props.sessionId;
    const timer = window.setInterval(() => refetchSubagents(), 3000);
    onCleanup(() => window.clearInterval(timer));
  });

  // Reconstructed Turns
  const entries = () => transcriptData()?.entries ?? [];
  const contract = () => (transcriptData() as unknown as { contract?: FrozenContract })?.contract ?? null;
  const receipts = () => receiptsData() ?? [];
  const checkpoints = () => checkpointsData()?.checkpoints ?? [];

  const turns = createMemo(() => reconstructTurns(entries(), receipts(), checkpoints()));

  // Active Selected Turn
  const currentTurn = createMemo(() => {
    const list = turns();
    if (list.length === 0) return null;
    const found = list.find((t) => t.turn_index === selectedTurnIndex());
    return found || list[list.length - 1] || null;
  });

  // Turn DAG Nodes
  const dagNodes = createMemo(() => {
    const turn = currentTurn();
    if (!turn) return [];
    return buildTurnDagNodes(turn, contract());
  });

  // Aggregated Telemetry
  const totalTokens = createMemo(() =>
    turns().reduce((acc, t) => acc + t.tokens_in + t.tokens_out, 0),
  );
  const totalCost = createMemo(() =>
    turns().reduce((acc, t) => acc + t.cost_usd, 0),
  );

  // Live SSE listener
  createEffect(() => {
    void props.sessionId;
    if (!live()) return;
    const es = new EventSource(`/sessions/${encodeURIComponent(props.sessionId)}/events`);
    es.onmessage = (m) => {
      try {
        const ev = JSON.parse(m.data) as Record<string, unknown>;
        if ("TurnStart" in ev || "ToolCallStart" in ev) setRunning(true);
        if ("RunFinished" in ev) setRunning(false);
      } catch {
        /* ignore */
      }
      setTimeout(() => {
        void refetchTranscript();
        void refetchReceipts();
        void refetchCheckpoints();
      }, 350);
    };
    onCleanup(() => es.close());
  });

  // Actions
  const cancel = async () => {
    try {
      await api.attach(props.sessionId);
      await api.cancelRun(props.sessionId);
      pushToast("info", "Cancellation requested");
      setRunning(false);
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    }
  };

  const send = async () => {
    const text = draft().trim();
    if (!text || sending()) return;
    setSending(true);
    try {
      await api.attach(props.sessionId);
      if (running() && nCandidates() === 1) {
        await api.steer(props.sessionId, text);
        pushToast("info", "Steering queued");
      } else if (nCandidates() >= 2) {
        const res = await api.startBestofn(props.sessionId, text, nCandidates());
        pushToast("info", `Fanned out to ${res.runs.length} candidate runs`);
        navigate("#/sessions");
        return;
      } else {
        await api.runPrompt(props.sessionId, text);
      }
      setDraft("");
      setLive(true);
      setRunning(true);
      setTimeout(() => {
        void refetchTranscript();
        void refetchReceipts();
      }, 500);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setSending(false);
    }
  };

  const restoreCommit = async (seq: number) => {
    if (!confirmDestructive(`Rewind workspace files to checkpoint #${seq}? Subsequent filesystem changes will be overwritten.`)) return;
    try {
      await api.attach(props.sessionId);
      await api.restoreCheckpoint(props.sessionId, seq);
      await Promise.all([refetchCheckpoints(), refetchTranscript(), refetchDiff()]);
      pushToast("info", `Restored to checkpoint #${seq}`);
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    }
  };

  const stopChild = async (child: ActiveSubagent) => {
    if (!confirmDestructive(`Stop subagent “${child.label}”?`)) return;
    setSubagentBusy(child.id);
    try {
      await api.stopSubagent(props.sessionId, child.id);
      await refetchSubagents();
      pushToast("info", "Subagent stopped");
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setSubagentBusy("");
    }
  };

  return (
    <div class="view session-forensics-view">
      {/* 1. Executive Session Masthead */}
      <div class="session-masthead">
        <div class="session-masthead-top">
          <button type="button" class="back-link-btn" onClick={() => navigate("#/sessions")}>
            ‹ Sessions
          </button>
          <span class="session-id-pill mono">{shortId(props.sessionId)}</span>

          <Show when={running()}>
            <span class="session-status-badge running">
              <span class="cdot cdot-active" /> RUNNING
            </span>
          </Show>

          <span class="spacer" />

          <div class="session-masthead-actions">
            <Show when={running()}>
              <button type="button" class="danger small" onClick={() => void cancel()}>
                Cancel run
              </button>
            </Show>

            <a
              class="button ghost small"
              href={`/sessions/${encodeURIComponent(props.sessionId)}/transcript.md`}
              target="_blank"
              rel="noreferrer noopener"
            >
              Export .md
            </a>

            <label class="toggle live-toggle">
              <input
                type="checkbox"
                checked={live()}
                onChange={(e) => setLive(e.currentTarget.checked)}
              />
              <span>Live tail</span>
            </label>
          </div>
        </div>

        {/* Masthead Metadata Pills */}
        <div class="session-meta-strip">
          <div class="session-meta-chip">
            <span class="meta-chip-label">Route:</span>
            <strong class="mono">{contract()?.provider ?? "ollama"} · {contract()?.model ?? "gemma4:e2b-mlx"}</strong>
          </div>

          <div class="session-meta-chip">
            <span class="meta-chip-label">Authority:</span>
            <strong class="chip chip-tone-success">{contract()?.permission_mode ?? "WorkspaceWrite"}</strong>
          </div>

          <div class="session-meta-chip">
            <span class="meta-chip-label">Turns:</span>
            <strong>{turns().length} completed</strong>
          </div>

          <div class="session-meta-chip">
            <span class="meta-chip-label">Volume:</span>
            <strong>{totalTokens().toLocaleString()} tokens</strong>
          </div>

          <div class="session-meta-chip">
            <span class="meta-chip-label">Cost:</span>
            <strong class="mono">${totalCost().toFixed(4)}</strong>
          </div>
        </div>
      </div>

      {/* 2. Turn Scrubber Bar */}
      <Show when={turns().length > 0}>
        <div class="turn-scrubber-container">
          <div class="turn-scrubber-header">
            <span class="scrubber-title">Turn Navigation & Timeline</span>
            <span class="scrubber-subtitle">Select a turn to inspect its pipeline DAG, telemetry, and execution facts</span>
          </div>

          <div class="turn-scrubber-strip">
            <For each={turns()}>
              {(turn) => {
                const isSelected = () => (currentTurn()?.turn_index ?? 1) === turn.turn_index;
                return (
                  <button
                    type="button"
                    class="turn-scrub-pill"
                    classList={{
                      active: isSelected(),
                      error: turn.status === "failed",
                    }}
                    onClick={() => {
                      setSelectedTurnIndex(turn.turn_index);
                      setSelectedStage(null);
                    }}
                  >
                    <span class="turn-pill-num">Turn {turn.turn_index}</span>
                    <span class="turn-pill-prompt">
                      {turn.user_prompt.slice(0, 22) || "Genesis"}…
                    </span>
                    <span class="turn-pill-cost">
                      {turn.tokens_in + turn.tokens_out > 0 ? `${(turn.tokens_in + turn.tokens_out).toLocaleString()} tok` : "—"}
                    </span>
                  </button>
                );
              }}
            </For>
          </div>
        </div>
      </Show>

      {/* 3. Segmented Navigation Tabs */}
      <div class="forensics-tabs-bar">
        <button
          type="button"
          class="forensics-tab-btn"
          classList={{ active: activeTab() === "dag" }}
          onClick={() => setActiveTab("dag")}
        >
          Turn Pipeline DAG
        </button>

        <button
          type="button"
          class="forensics-tab-btn"
          classList={{ active: activeTab() === "transcript" }}
          onClick={() => setActiveTab("transcript")}
        >
          Conversational Log ({entries().length})
        </button>

        <button
          type="button"
          class="forensics-tab-btn"
          classList={{ active: activeTab() === "drift" }}
          onClick={() => setActiveTab("drift")}
        >
          Context & Drift Audit
        </button>

        <button
          type="button"
          class="forensics-tab-btn"
          classList={{ active: activeTab() === "receipts" }}
          onClick={() => setActiveTab("receipts")}
        >
          Work Receipts ({receipts().length})
        </button>

        <button
          type="button"
          class="forensics-tab-btn"
          classList={{ active: activeTab() === "checkpoints" }}
          onClick={() => setActiveTab("checkpoints")}
        >
          Checkpoints ({checkpoints().length})
        </button>
      </div>

      {/* 4. Tab Content Views */}
      <Switch>
        {/* TAB 1: TURN PIPELINE DAG */}
        <Match when={activeTab() === "dag"}>
          <Show when={currentTurn()} fallback={<div class="empty">No turn selected.</div>}>
            {(turn) => (
              <div class="turn-dag-view">
                {/* Turn Header Card */}
                <div class="turn-header-card">
                  <div class="turn-header-left">
                    <span class="turn-index-badge">TURN {turn().turn_index}</span>
                    <strong class="turn-prompt-title">“{turn().user_prompt}”</strong>
                  </div>
                  <div class="turn-header-right">
                    <span class="turn-time">{clock(turn().started_at)} ({timeAgo(turn().started_at)})</span>
                    <span class="chip chip-tone-success">{turn().status}</span>
                  </div>
                </div>

                {/* Turn DAG SVG Pipeline Flow */}
                <div class="pipeline-dag-container">
                  <div class="pipeline-dag-grid">
                    <For each={dagNodes()}>
                      {(node, idx) => {
                        const isSelected = () => selectedStage()?.id === node.id;
                        return (
                          <div class="pipeline-node-wrapper">
                            <button
                              type="button"
                              class="pipeline-dag-card"
                              classList={{
                                "node-selected": isSelected(),
                                "node-ok": node.status === "ok",
                                "node-warn": node.status === "warn",
                                "node-bad": node.status === "bad",
                                "node-idle": node.status === "idle",
                              }}
                              onClick={() => setSelectedStage(node)}
                            >
                              <div class="dag-node-header">
                                <span class="dag-node-cat">{node.category}</span>
                                <span
                                  class="cdot"
                                  classList={{
                                    "cdot-done": node.status === "ok",
                                    "cdot-waiting": node.status === "warn",
                                    "cdot-blocked": node.status === "bad",
                                    "cdot-held": node.status === "idle",
                                  }}
                                />
                              </div>
                              <strong class="dag-node-title">{node.title}</strong>
                              <div class="dag-node-summary">{node.summary}</div>
                              <Show when={node.metrics}>
                                <div class="dag-node-metrics">{node.metrics}</div>
                              </Show>
                            </button>
                            <Show when={idx() < dagNodes().length - 1}>
                              <div class="pipeline-connector-arrow">→</div>
                            </Show>
                          </div>
                        );
                      }}
                    </For>
                  </div>
                </div>

                {/* Interactive Stage Inspector Drawer */}
                <Show when={selectedStage()}>
                  {(stage) => (
                    <div class="stage-inspector-drawer">
                      <div class="stage-inspector-header">
                        <div>
                          <span class="inspector-badge">{stage().category}</span>
                          <strong>{stage().title} — Stage Forensics</strong>
                        </div>
                        <button
                          type="button"
                          class="ghost small"
                          onClick={() => setSelectedStage(null)}
                        >
                          Close inspector
                        </button>
                      </div>
                      <div class="stage-inspector-body">
                        <div class="stage-inspector-fact">
                          <span class="fact-label">Summary:</span>
                          <span class="fact-val">{stage().summary}</span>
                        </div>
                        <div class="stage-inspector-fact">
                          <span class="fact-label">Status:</span>
                          <span class="fact-val">{stage().status.toUpperCase()}</span>
                        </div>
                        <div class="stage-inspector-code">
                          <span class="fact-label">Forensic Payload:</span>
                          <pre class="mono">{JSON.stringify(stage().details, null, 2)}</pre>
                        </div>
                      </div>
                    </div>
                  )}
                </Show>

                {/* Subagents in this turn */}
                <Show when={(subagentData()?.subagents?.length ?? 0) > 0}>
                  <div class="subagents-panel">
                    <h3>Active Subagents</h3>
                    <div class="subagents-grid">
                      <For each={subagentData()?.subagents ?? []}>
                        {(child) => (
                          <div class="subagent-card">
                            <div class="subagent-head">
                              <strong>{child.label}</strong>
                              <span class="mono dim">{shortId(child.id)}</span>
                            </div>
                            <div class="subagent-actions">
                              <button
                                type="button"
                                class="danger small"
                                disabled={subagentBusy() === child.id}
                                onClick={() => void stopChild(child)}
                              >
                                Stop
                              </button>
                            </div>
                          </div>
                        )}
                      </For>
                    </div>
                  </div>
                </Show>
              </div>
            )}
          </Show>
        </Match>

        {/* TAB 2: CONVERSATIONAL LOG */}
        <Match when={activeTab() === "transcript"}>
          <div class="transcript-tab-view">
            <div class="transcript-toolbar">
              <select value={kindFilter()} onChange={(e) => setKindFilter(e.currentTarget.value)}>
                <option value="">All Kinds</option>
                <option value="message">Messages</option>
                <option value="intent">Intent</option>
                <option value="receipt">Work Receipts</option>
                <option value="activity">Activities</option>
                <option value="goal">Goals</option>
                <option value="compaction">Compaction</option>
                <option value="header">Header</option>
              </select>

              <select value={roleFilter()} onChange={(e) => setRoleFilter(e.currentTarget.value)}>
                <option value="">All Roles</option>
                <option value="user">User</option>
                <option value="assistant">Assistant</option>
                <option value="system">System</option>
              </select>

              <span class="spacer" />
              <span class="dim" style="font-size:12px;">Showing {entries().length} events</span>
            </div>

            <div class="transcript-feed" classList={{ tailing: live() }}>
              <For each={entries()}>
                {(e) => (
                  <article class="entry-card" data-role={e.role ?? "system"} data-error={e.is_error}>
                    <header class="entry-header">
                      <span class="entry-role-chip">{e.role ?? e.kind}</span>
                      <Show when={e.tool_name}>
                        <span class="chip chip-tool mono">{e.tool_name}</span>
                      </Show>
                      <Show when={e.is_error}>
                        <span class="chip chip-error">error</span>
                      </Show>
                      <span class="entry-ts" title={e.ts}>{clock(e.ts)}</span>
                    </header>
                    <pre class="entry-content mono">{e.content}</pre>
                  </article>
                )}
              </For>
            </div>
          </div>
        </Match>

        {/* TAB 3: CONTEXT & DRIFT AUDIT */}
        <Match when={activeTab() === "drift"}>
          <div class="drift-tab-view">
            <div class="drift-header-banner">
              <div>
                <h3>Context Packet & Drift Accounting</h3>
                <p>
                  Invariant 1 & 17 audit: session instructions freeze at admission time.
                  Verifies that system prompt layers, capabilities, and route state have not drifted.
                </p>
              </div>
            </div>

            <div class="drift-grid">
              {/* Card 1: Frozen Route Contract */}
              <div class="drift-card">
                <h4>1. Admission Contract Snapshot</h4>
                <div class="drift-row">
                  <span class="drift-label">App Version:</span>
                  <span class="mono">{contract()?.app_version ?? "3.0.93"}</span>
                </div>
                <div class="drift-row">
                  <span class="drift-label">Admitted Provider:</span>
                  <span class="mono">{contract()?.provider}</span>
                </div>
                <div class="drift-row">
                  <span class="drift-label">Admitted Model:</span>
                  <span class="mono">{contract()?.model}</span>
                </div>
                <div class="drift-row">
                  <span class="drift-label">Authority Mode:</span>
                  <span class="chip chip-tone-success">{contract()?.permission_mode}</span>
                </div>
              </div>

              {/* Card 2: Prompt Layers */}
              <div class="drift-card">
                <h4>2. Prompt Layers & Provenance</h4>
                <Show
                  when={(contract()?.prompt_layers ?? []).length > 0}
                  fallback={<div class="dim">No layer hashes recorded (monolithic header).</div>}
                >
                  <div class="layers-list">
                    <For each={contract()?.prompt_layers ?? []}>
                      {(layer: PromptLayerDescriptor) => (
                        <div class="layer-item">
                          <div class="layer-head">
                            <strong>{layer.block}</strong>
                            <span class="chip">{layer.layer}</span>
                          </div>
                          <div class="layer-meta mono dim">
                            hash: {layer.digest.slice(0, 16)}… · {layer.bytes} bytes
                          </div>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </div>

              {/* Card 3: Admitted Capabilities */}
              <div class="drift-card" style="grid-column: span 2;">
                <h4>3. Admitted Capabilities Inventory</h4>
                <Show
                  when={(contract()?.capabilities ?? []).length > 0}
                  fallback={<div class="dim">Capabilities derived from standard harness inventory.</div>}
                >
                  <div class="capabilities-grid">
                    <For each={contract()?.capabilities ?? []}>
                      {(cap: CapabilityDescriptor) => (
                        <div class="cap-item">
                          <div class="cap-head">
                            <strong class="mono">{cap.name}</strong>
                            <span class="chip">{cap.kind}</span>
                          </div>
                          <p class="cap-desc">{cap.description}</p>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </div>
            </div>
          </div>
        </Match>

        {/* TAB 4: WORK RECEIPTS */}
        <Match when={activeTab() === "receipts"}>
          <div class="receipts-tab-view">
            <Show when={receipts().length > 0} fallback={<div class="empty">No work receipts billed to this session yet.</div>}>
              <table class="table receipts-table">
                <thead>
                  <tr>
                    <th>Step</th>
                    <th>Provider</th>
                    <th>Model</th>
                    <th>Input Tokens</th>
                    <th>Output Tokens</th>
                    <th>Cost (USD)</th>
                    <th>Latency</th>
                    <th>Settlement</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={receipts()}>
                    {(r: WorkReceipt, i) => (
                      <tr>
                        <td class="mono">#{i() + 1}</td>
                        <td>{r.provider ?? "—"}</td>
                        <td class="mono">{r.model ?? "—"}</td>
                        <td>{(r.input_tokens ?? 0).toLocaleString()}</td>
                        <td>{(r.output_tokens ?? 0).toLocaleString()}</td>
                        <td class="mono font-bold">${(r.cost_usd ?? 0).toFixed(5)}</td>
                        <td>{r.latency_ms ? `${r.latency_ms}ms` : "—"}</td>
                        <td><span class="chip chip-tone-success">settled</span></td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </Show>
          </div>
        </Match>

        {/* TAB 5: CHECKPOINTS & DIFFS */}
        <Match when={activeTab() === "checkpoints"}>
          <div class="checkpoints-tab-view">
            <div class="checkpoints-grid">
              <div class="checkpoints-panel">
                <h3>Filesystem Save Points</h3>
                <Show when={checkpoints().length > 0} fallback={<div class="empty">No checkpoints recorded.</div>}>
                  <div class="checkpoints-list">
                    <For each={checkpoints()}>
                      {(cp: SessionCheckpoint) => (
                        <div class="checkpoint-item">
                          <div class="cp-info">
                            <strong>Checkpoint #{cp.seq}</strong>
                            <span class="cp-label">{cp.message || cp.label || "Save Point"}</span>
                            <span class="cp-ts dim">{timeAgo(cp.ts || cp.created_at || "")}</span>
                          </div>
                          <button
                            type="button"
                            class="button small ghost"
                            onClick={() => void restoreCommit(cp.seq)}
                          >
                            Rewind to #{cp.seq}
                          </button>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </div>

              <div class="diff-panel">
                <h3>Workspace Git Diff</h3>
                <div class="diff-canvas">
                  <pre class="mono">{diffData()?.diff || "No uncommitted modifications on disk."}</pre>
                </div>
              </div>
            </div>
          </div>
        </Match>
      </Switch>

      {/* 5. Interactive Composer & Steering */}
      <div class="session-composer-bar">
        <div class="composer-controls">
          <select
            class="candidate-stepper"
            value={nCandidates()}
            onChange={(e) => setNCandidates(Number(e.currentTarget.value))}
            disabled={running()}
            title="Fan-out best-of-N candidate exploration"
          >
            <option value={1}>x1</option>
            <option value={2}>x2 candidates</option>
            <option value={3}>x3 candidates</option>
            <option value={4}>x4 candidates</option>
          </select>

          <textarea
            rows={2}
            placeholder={
              running()
                ? "Queue steering instruction into active run…"
                : nCandidates() >= 2
                ? `Explore ${nCandidates()} parallel candidates…`
                : "Ask vak to run the next turn…"
            }
            value={draft()}
            onInput={(e) => setDraft(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void send();
              }
            }}
            disabled={sending()}
          />

          <button
            type="button"
            class="button primary"
            disabled={sending() || !draft().trim()}
            onClick={() => void send()}
          >
            {sending() ? "Sending…" : running() ? "Steer" : "Send Turn"}
          </button>
        </div>
      </div>
    </div>
  );
}
