import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { api, AuthRequired } from "./api";
import { clock, shortId, timeAgo } from "./time";
import {
  approvalsVersion, authed, conn, connectEvents, disconnectEvents, feed, navigate, pushToast,
  route, sessionsVersion, setAuthed, statsVersion, toasts,
} from "./store";
import type {
  BestOfNRun, ConfigInfo, DiscoveredModelsResponse, FinOpsStatus, HookConfig,
  InboxEntry, McpServerConfig, MemoryItem, OpsStatus, PendingApproval, ProviderSummary,
  SearchHit, SecurityEvent, SessionCheckpoint, SessionDiff, SessionListItem,
  SkillItem, SkillProposal, TaskItem, TranscriptEntry, WorkReceipt,
} from "./types";

// Unread inbox badge: polled lightly while signed in.
const [unread, setUnread] = createSignal(0);

// ---- icons (inline, stroke style) ------------------------------------------

const Icon = (props: { d: string; size?: number }) => (
  <svg
    width={props.size ?? 16}
    height={props.size ?? 16}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path d={props.d} />
  </svg>
);

const ICONS = {
  overview: "M3 3v18h18M7 15l4-6 4 4 5-8",
  sessions: "M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM23 21v-2a4 4 0 0 0-3-3.87",
  integrations: "M16.5 9.4 7.55 4.24a1.78 1.78 0 0 0-2.5 1.55v12.42a1.78 1.78 0 0 0 2.5 1.55L16.5 14.6a1.78 1.78 0 0 0 0-3.2z M21 12h-3 M3 12h1",
  memory: "M4 19.5A2.5 2.5 0 0 1 6.5 17H20 M4 4.5A2.5 2.5 0 0 1 6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15z",
  search: "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35",
  inbox: "M22 12h-6l-2 3h-4l-2-3H2M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
  security: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z",
  settings: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
};

// ---- Login -----------------------------------------------------------------

function Login() {
  const [token, setToken] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");

  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    if (!token().trim() || busy()) return;
    setBusy(true);
    setError("");
    try {
      await api.login(token().trim());
      setAuthed(true);
      connectEvents();
      navigate("#/overview");
    } catch (err) {
      setError(err instanceof AuthRequired || `${err}`.includes("401") ? "Invalid token" : `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="login-wrap">
      <form class="login-card" onSubmit={submit}>
        <div class="login-logo">◆</div>
        <h1>vakcoder admin</h1>
        <p class="hint">Paste the token printed by <code>vakcoder serve</code></p>
        <input
          type="password"
          placeholder="access token"
          autocomplete="current-password"
          autofocus
          value={token()}
          onInput={(e) => setToken(e.currentTarget.value)}
          classList={{ shake: !!error() }}
        />
        <Show when={error()}>
          <div class="login-error">{error()}</div>
        </Show>
        <button type="submit" disabled={busy() || !token().trim()}>
          {busy() ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </div>
  );
}

// ---- Overview --------------------------------------------------------------

function StatCard(props: { label: string; value: string | number; sub?: string; tone?: string; progress?: number }) {
  return (
    <div class="stat-card" data-tone={props.tone ?? "default"}>
      <div class="stat-value">{props.value}</div>
      <div class="stat-label">{props.label}</div>
      <Show when={props.progress != null}>
        <div class="progress-bar">
          <div
            class={`progress-fill ${props.progress! > 90 ? "alert" : props.progress! > 75 ? "warn" : ""}`}
            style={{ width: `${Math.min(100, Math.max(0, props.progress!))}%` }}
          />
        </div>
      </Show>
      <Show when={props.sub}>
        <div class="stat-sub">{props.sub}</div>
      </Show>
    </div>
  );
}

function Overview() {
  const [health, healthActions] = createResource(statsVersion, () => api.health());
  const [sessions] = createResource(statsVersion, () => api.sessions());
  const [security] = createResource(statsVersion, () => api.security(500));
  const [finops] = createResource(statsVersion, () => api.finops().catch(() => null));
  const [ops] = createResource(statsVersion, () => api.opsStatus().catch(() => null));

  const recentSecurity = createMemo(
    () =>
      (security()?.events ?? []).filter((e) => Date.now() - new Date(e.ts).getTime() < 86_400_000)
        .length,
  );

  const feedItems = createMemo(() => [...feed()].reverse());

  const spendUSD = createMemo(() => finops()?.total_spend_usd ?? finops()?.total_cost ?? 0);
  const capUSD = createMemo(() => finops()?.budget_cap_usd ?? null);
  const spendProgress = createMemo(() => {
    const cap = capUSD();
    return cap && cap > 0 ? (spendUSD() / cap) * 100 : undefined;
  });

  return (
    <div class="view">
      <div class="stats-row">
        <StatCard label="Sessions indexed" value={sessions()?.sessions.length ?? "…"} />
        <StatCard
          label="Total entries"
          value={sessions()?.sessions.reduce((a, s) => a + s.entry_count, 0) ?? "…"}
        />
        <StatCard
          label="FinOps spend"
          value={`$${spendUSD().toFixed(4)}`}
          progress={spendProgress()}
          sub={capUSD() ? `Budget cap: $${capUSD()!.toFixed(2)}` : "No limit set"}
          tone={spendProgress() && spendProgress()! > 90 ? "warn" : undefined}
        />
        <StatCard
          label="Security events · 24h"
          value={security.loading ? "…" : recentSecurity()}
          tone={recentSecurity() > 0 ? "warn" : undefined}
        />
      </div>

      <ApprovalsCard />

      <div class="two-col">
        <section class="panel">
          <h2>System &amp; Operations</h2>
          <Show when={!health.loading} fallback={<div class="empty">Loading…</div>}>
            <dl class="kv">
              <dt>provider</dt>
              <dd>{health()?.provider}</dd>
              <dt>model</dt>
              <dd class="mono">{health()?.model}</dd>
              <dt>permission mode</dt>
              <dd><span class="chip chip-mode">{health()?.permission_mode}</span></dd>
              <dt>sandbox</dt>
              <dd>{health()?.sandbox}</dd>
              <dt>context window</dt>
              <dd>{(health()?.context_window ?? 0).toLocaleString()} tok</dd>
              <dt>gateway service</dt>
              <dd>
                <span class="chip" data-on={ops()?.gateway?.state === "running" || ops()?.gateway_healthy}>
                  {ops()?.gateway?.state ?? (health()?.status === "ok" ? "ready" : "offline")}
                </span>
              </dd>
              <dt>workspace</dt>
              <dd class="mono wrap">{health()?.cwd}</dd>
            </dl>
            <Show when={(health()?.warnings?.length ?? 0) > 0}>
              <div class="warnings">
                <For each={health()?.warnings}>{(w) => <div class="warning">⚠ {w}</div>}</For>
              </div>
            </Show>
            <div class="row-gap" style="margin-top:14px">
              <button class="ghost small" onClick={() => healthActions.refetch()}>
                Refresh
              </button>
            </div>
          </Show>
        </section>

        <section class="panel">
          <h2>Live activity</h2>
          <Show
            when={feedItems().length > 0}
            fallback={<div class="empty">Waiting for events… they will appear here in real time.</div>}
          >
            <ul class="feed">
              <For each={feedItems()}>
                {(item) => (
                  <li data-type={item.event.type}>
                    <span class="feed-time">{clock(item.ts)}</span>
                    <span class="feed-kind">{item.event.type}</span>
                    <span class="feed-text">{summarizeEvent(item.event)}</span>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </section>
      </div>
    </div>
  );
}

// ---- Pending approvals -----------------------------------------------------

function ApprovalsCard() {
  const [pending, { refetch }] = createResource(approvalsVersion, () => api.approvals());
  const [busyId, setBusyId] = createSignal("");

  const answer = async (a: PendingApproval, approve: boolean) => {
    setBusyId(a.request_id);
    try {
      await api.answer(a.session_id, a.request_id, approve);
      pushToast("info", `${approve ? "Granted" : "Denied"} ${a.tool}`);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusyId("");
      refetch();
    }
  };

  return (
    <section class="panel" classList={{ "panel-alert": (pending()?.total ?? 0) > 0 }} style="margin-bottom:14px">
      <h2>Pending approvals</h2>
      <Show
        when={(pending()?.approvals.length ?? 0) > 0}
        fallback={<div class="empty">No gates waiting. Runs proceed without you.</div>}
      >
        <ul class="approval-list">
          <For each={pending()!.approvals}>
            {(a) => (
              <li>
                <div class="approval-head">
                  <span class="chip chip-tool mono">{a.tool}</span>
                  <span class="mono dim">{shortId(a.session_id)}</span>
                  <span class="when">{timeAgo(a.requested_at)}</span>
                </div>
                <Show when={a.reason}>
                  <div class="approval-reason">{a.reason}</div>
                </Show>
                <pre class="mono approval-args">{a.args_json}</pre>
                <div class="row-gap">
                  <button
                    class="approve"
                    disabled={busyId() === a.request_id}
                    onClick={() => answer(a, true)}
                  >
                    Approve
                  </button>
                  <button class="danger" disabled={busyId() === a.request_id} onClick={() => answer(a, false)}>
                    Deny
                  </button>
                  <span class="spacer" />
                  <button class="ghost small" onClick={() => navigate(`#/sessions/${a.session_id}`)}>
                    View session
                  </button>
                </div>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </section>
  );
}

function summarizeEvent(ev: import("./types").SystemEvent): string {
  switch (ev.type) {
    case "Agent":
      return ev.data.summary + (ev.data.detail ? ` · ${ev.data.detail}` : "");
    case "SessionCreated":
      return ev.data.session_id.slice(0, 12);
    case "SessionEntryAppended":
      return `${ev.data.kind} → ${ev.data.session_id.slice(0, 12)}`;
    case "ConfigChanged":
      return `${ev.data.label}: ${ev.data.detail}`;
    case "GatewayInbound":
      return `[${ev.data.surface}] ${ev.data.who}: ${ev.data.preview}`;
    case "ApprovalGranted":
    case "ApprovalDenied":
      return `${ev.data.tool} (${ev.data.id.slice(0, 8)})`;
    case "SecurityEvent":
      return `${ev.data.kind} — ${ev.data.label}`;
    case "ProviderError":
      return `${ev.data.provider}/${ev.data.model}: ${ev.data.error}`;
    case "RateLimit":
      return ev.data.provider;
    default:
      return "";
  }
}

// ---- Sessions list & Best-of-N Candidate Management ------------------------

function Sessions() {
  const [sessions, { refetch }] = createResource(sessionsVersion, () => api.sessions());
  const [bestofn, bestofnActions] = createResource(sessionsVersion, () => api.bestofn().catch(() => ({ runs: [], total: 0 })));
  const [q, setQ] = createSignal("");
  const [creating, setCreating] = createSignal(false);
  const [busyCandidate, setBusyCandidate] = createSignal("");

  const newSession = async () => {
    setCreating(true);
    try {
      const { session_id } = await api.createSession();
      navigate(`#/sessions/${session_id}`);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setCreating(false);
    }
  };

  const handleKeep = async (sessionId: string) => {
    setBusyCandidate(sessionId);
    try {
      await api.keepBestRun(sessionId);
      pushToast("info", "Candidate kept — worktree merged");
      bestofnActions.refetch();
      refetch();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusyCandidate("");
    }
  };

  const handleDiscard = async (sessionId: string) => {
    setBusyCandidate(sessionId);
    try {
      await api.discardBestRun(sessionId);
      pushToast("info", "Candidate run discarded");
      bestofnActions.refetch();
      refetch();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusyCandidate("");
    }
  };

  const filtered = createMemo(() => {
    const needle = q().toLowerCase();
    return (sessions()?.sessions ?? []).filter(
      (s: SessionListItem) => !needle || s.session_id.toLowerCase().includes(needle),
    );
  });

  return (
    <div class="view">
      {/* Best of N Candidate Runs Card */}
      <Show when={(bestofn()?.runs?.length ?? 0) > 0}>
        <section class="panel" style="margin-bottom:14px">
          <h2>Active Best-of-N candidate runs ({bestofn()!.runs.length})</h2>
          <div class="candidate-grid">
            <For each={bestofn()!.runs}>
              {(run) => (
                <div class="candidate-card">
                  <div class="candidate-head">
                    <span class="mono bold">{shortId(run.session_id)}</span>
                    <span class="chip chip-mode mono">{run.branch}</span>
                  </div>
                  <div class="dim mono small wrap" style="margin-bottom:8px">{run.repo}</div>
                  <div class="row-gap">
                    <button class="approve small" disabled={busyCandidate() === run.session_id} onClick={() => handleKeep(run.session_id)}>
                      Keep
                    </button>
                    <button class="danger small" disabled={busyCandidate() === run.session_id} onClick={() => handleDiscard(run.session_id)}>
                      Discard
                    </button>
                    <span class="spacer" />
                    <button class="ghost small" onClick={() => navigate(`#/sessions/${run.session_id}`)}>
                      Inspect
                    </button>
                  </div>
                </div>
              )}
            </For>
          </div>
        </section>
      </Show>

      <div class="toolbar">
        <input class="search-input" placeholder="Filter by session id…" value={q()} onInput={(e) => setQ(e.currentTarget.value)} />
        <span class="spacer" />
        <button disabled={creating()} onClick={newSession}>
          {creating() ? "Creating…" : "+ New session"}
        </button>
        <button class="ghost" onClick={() => { refetch(); bestofnActions.refetch(); }}>Refresh</button>
      </div>
      <Show when={!sessions.loading} fallback={<div class="empty">Loading sessions…</div>}>
        <Show
          when={filtered().length > 0}
          fallback={<div class="empty">No sessions yet. Run a prompt via TUI, desktop, or gateway and it appears here.</div>}
        >
          <table class="table">
            <thead>
              <tr><th>session</th><th>entries</th><th>first seen</th><th>last activity</th><th /></tr>
            </thead>
            <tbody>
              <For each={filtered()}>
                {(s) => (
                  <tr onClick={() => navigate(`#/sessions/${s.session_id}`)}>
                    <td class="mono">{shortId(s.session_id)}</td>
                    <td>{s.entry_count}</td>
                    <td title={s.first_ts}>{timeAgo(s.first_ts)}</td>
                    <td title={s.last_ts}>{timeAgo(s.last_ts)}</td>
                    <td><span class="chev">›</span></td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </Show>
      </Show>
    </div>
  );
}

// ---- Transcript & Session Forensics ----------------------------------------

const KIND_FILTERS = [
  { id: "", label: "All kinds" },
  { id: "message", label: "Messages" },
];

const ROLE_FILTERS = [
  { id: "", label: "All roles" },
  { id: "user", label: "User" },
  { id: "assistant", label: "Assistant" },
];

function Transcript(props: { sessionId: string }) {
  const [activeTab, setActiveTab] = createSignal<"transcript" | "diff" | "receipts" | "checkpoints">("transcript");
  const [kind, setKind] = createSignal("");
  const [role, setRole] = createSignal("");
  const [entries, setEntries] = createSignal<TranscriptEntry[]>([]);
  const [hasMore, setHasMore] = createSignal(false);
  const [totalCount, setTotalCount] = createSignal(0);
  const [offset, setOffset] = createSignal(0);
  const [loading, setLoading] = createSignal(true);
  const [live, setLive] = createSignal(false);
  const [running, setRunning] = createSignal(false);

  // Side tabs resources
  const [diffData] = createResource(activeTab, (t) => t === "diff" ? api.diff(props.sessionId).catch(() => ({ diff: "No worktree diff available." })) : Promise.resolve(null));
  const [receiptsData] = createResource(activeTab, (t) => t === "receipts" ? api.receipts(props.sessionId).catch(() => []) : Promise.resolve(null));
  const [checkpointsData, { refetch: refetchCheckpoints }] = createResource(activeTab, (t) => t === "checkpoints" ? api.checkpoints(props.sessionId).catch(() => ({ checkpoints: [] })) : Promise.resolve(null));

  const PAGE = 100;

  const load = async (reset: boolean) => {
    setLoading(true);
    try {
      const currentOffset = reset ? 0 : offset();
      if (reset) setOffset(0);
      const res = await api.transcript(props.sessionId, {
        limit: PAGE,
        offset: currentOffset,
        kind: kind() || undefined,
        role: role() || undefined,
        refresh: true,
      });
      setEntries(res.entries);
      setHasMore(res.has_more);
      setTotalCount(res.total);
      if (live()) {
        queueMicrotask(() => document.querySelector(".transcript")?.scrollTo({ top: 1e9 }));
      }
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setLoading(false);
    }
  };

  createEffect(() => {
    void props.sessionId;
    void kind();
    void role();
    load(true);
  });

  // Live tail: subscribe to the session's event stream.
  // Rust serializes AgentEvent as untagged: {"TurnStart":{"turn":0}}, {"RunFinished":...}
  createEffect(() => {
    void props.sessionId;
    if (!live()) return;
    const es = new EventSource(`/sessions/${encodeURIComponent(props.sessionId)}/events`);
    es.onmessage = (m) => {
      try {
        const ev = JSON.parse(m.data) as Record<string, unknown>;
        if ("TurnStart" in ev || "ToolCallStart" in ev) setRunning(true);
        if ("RunFinished" in ev) setRunning(false);
      } catch { /* ignore */ }
      clearTimeout((es as unknown as { t?: number }).t);
      (es as unknown as { t?: number }).t = setTimeout(() => load(true), 300) as unknown as number;
    };
    onCleanup(() => es.close());
  });

  const changePage = (delta: number) => {
    setOffset((prev) => Math.max(0, prev + delta * PAGE));
    load(false);
  };

  const cancel = async () => {
    try {
      await api.cancelRun(props.sessionId);
      pushToast("info", "Cancellation requested");
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const restoreCommit = async (seq: number) => {
    try {
      await api.restoreCheckpoint(props.sessionId, seq);
      pushToast("info", `Restored to checkpoint #${seq}`);
      refetchCheckpoints();
      load(true);
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  // ---- composer ----

  const [draft, setDraft] = createSignal("");
  const [nCandidates, setNCandidates] = createSignal(1);
  const [sending, setSending] = createSignal(false);

  const send = async () => {
    const text = draft().trim();
    if (!text || sending()) return;
    setSending(true);
    try {
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
      setTimeout(() => load(true), 400);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else if (`${err}`.includes("409")) pushToast("warn", "A run is already active on this session");
      else pushToast("alert", `${err}`);
    } finally {
      setSending(false);
    }
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  };

  return (
    <div class="view">
      <div class="toolbar">
        <button class="ghost" onClick={() => navigate("#/sessions")}>‹ Sessions</button>
        <span class="mono dim">{shortId(props.sessionId)}</span>
        <span class="spacer" />
        <Show when={running()}>
          <span class="chip chip-running">run active</span>
          <button class="danger small" onClick={cancel}>Cancel</button>
        </Show>
        <label class="toggle">
          <input type="checkbox" checked={live()} onChange={(e) => setLive(e.currentTarget.checked)} />
          Live tail
        </label>
      </div>

      {/* Tabs Bar */}
      <div class="tab-bar">
        <button class="tab-btn" classList={{ active: activeTab() === "transcript" }} onClick={() => setActiveTab("transcript")}>
          Transcript ({totalCount()})
        </button>
        <button class="tab-btn" classList={{ active: activeTab() === "diff" }} onClick={() => setActiveTab("diff")}>
          Worktree Diff
        </button>
        <button class="tab-btn" classList={{ active: activeTab() === "receipts" }} onClick={() => setActiveTab("receipts")}>
          Work Receipts
        </button>
        <button class="tab-btn" classList={{ active: activeTab() === "checkpoints" }} onClick={() => setActiveTab("checkpoints")}>
          Checkpoints
        </button>
      </div>

      <Switch>
        {/* Transcript Tab */}
        <Match when={activeTab() === "transcript"}>
          <div class="toolbar" style="margin-top:-6px; margin-bottom:10px">
            <select value={kind()} onChange={(e) => setKind(e.currentTarget.value)}>
              <For each={KIND_FILTERS}>{(f) => <option value={f.id}>{f.label}</option>}</For>
            </select>
            <select value={role()} onChange={(e) => setRole(e.currentTarget.value)}>
              <For each={ROLE_FILTERS}>{(f) => <option value={f.id}>{f.label}</option>}</For>
            </select>
          </div>

          <Show when={!loading()} fallback={<div class="empty">Loading transcript…</div>}>
            <Show
              when={entries().length > 0}
              fallback={<div class="empty">No entries match this filter.</div>}
            >
              <div class="transcript" classList={{ tailing: live() }}>
                <For each={entries()}>
                  {(e) => (
                    <article class="entry" data-role={e.role ?? "system"} data-error={e.is_error}>
                      <header>
                        <span class="who">{e.role ?? e.kind}</span>
                        <Show when={e.tool_name}>
                          <span class="chip chip-tool mono">{e.tool_name}</span>
                        </Show>
                        <Show when={e.is_error}>
                          <span class="chip chip-error">error</span>
                        </Show>
                        <span class="when" title={e.ts}>{clock(e.ts)}</span>
                      </header>
                      <pre class="mono">{e.content}</pre>
                    </article>
                  )}
                </For>
              </div>
              <div class="pager">
                <button class="ghost small" disabled={offset() === 0} onClick={() => changePage(-1)}>‹ Newer</button>
                <span class="dim">page {Math.floor(offset() / PAGE) + 1}</span>
                <button class="ghost small" disabled={!hasMore()} onClick={() => changePage(1)}>Older ›</button>
              </div>
            </Show>
          </Show>

          <div class="composer">
            <select
              class="n-stepper"
              title="1 = single run · 2–4 = best-of-N fan-out"
              value={nCandidates()}
              onChange={(e) => setNCandidates(Number(e.currentTarget.value))}
              disabled={running()}
            >
              <option value={1}>×1</option>
              <option value={2}>×2</option>
              <option value={3}>×3</option>
              <option value={4}>×4</option>
            </select>
            <textarea
              rows={2}
              placeholder={running() ? "Queue steering for the active run…" : nCandidates() >= 2 ? `Fan across ${nCandidates()} isolated worktrees…` : "Send a prompt to this session…"}
              value={draft()}
              onInput={(e) => setDraft(e.currentTarget.value)}
              onKeyDown={onKey}
              disabled={sending()}
            />
            <button onClick={() => void send()} disabled={sending() || !draft().trim()}>
              {sending() ? "…" : running() && nCandidates() === 1 ? "Steer" : "Send"}
            </button>
          </div>
        </Match>

        {/* Diff Tab */}
        <Match when={activeTab() === "diff"}>
          <Show when={!diffData.loading} fallback={<div class="empty">Computing worktree diff…</div>}>
            <div class="diff-box">
              <Show when={diffData()?.diff} fallback="No uncommitted changes in session worktree.">
                {diffData()!.diff}
              </Show>
            </div>
          </Show>
        </Match>

        {/* Work Receipts Tab */}
        <Match when={activeTab() === "receipts"}>
          <Show when={!receiptsData.loading} fallback={<div class="empty">Loading work receipts…</div>}>
            <Show when={(receiptsData()?.length ?? 0) > 0} fallback={<div class="empty">No work receipts logged for this session yet.</div>}>
              <table class="table">
                <thead>
                  <tr><th>step</th><th>provider</th><th>model</th><th>in tokens</th><th>out tokens</th><th>cost (usd)</th><th>latency</th></tr>
                </thead>
                <tbody>
                  <For each={receiptsData() ?? []}>
                    {(r: WorkReceipt) => (
                      <tr>
                        <td>{r.step ?? "—"}</td>
                        <td>{r.provider ?? "—"}</td>
                        <td class="mono">{r.model ?? "—"}</td>
                        <td>{r.input_tokens?.toLocaleString() ?? 0}</td>
                        <td>{r.output_tokens?.toLocaleString() ?? 0}</td>
                        <td class="mono">${(r.cost_usd ?? 0).toFixed(5)}</td>
                        <td>{r.latency_ms ? `${r.latency_ms}ms` : "—"}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </Show>
          </Show>
        </Match>

        {/* Checkpoints Tab */}
        <Match when={activeTab() === "checkpoints"}>
          <Show when={!checkpointsData.loading} fallback={<div class="empty">Loading checkpoints…</div>}>
            <Show when={(checkpointsData()?.checkpoints?.length ?? 0) > 0} fallback={<div class="empty">No checkpoints recorded for this session yet.</div>}>
              <table class="table">
                <thead>
                  <tr><th>seq</th><th>timestamp</th><th>commit</th><th>message</th><th /></tr>
                </thead>
                <tbody>
                  <For each={checkpointsData()?.checkpoints ?? []}>
                    {(cp: SessionCheckpoint) => (
                      <tr>
                        <td>#{cp.seq}</td>
                        <td title={cp.ts}>{timeAgo(cp.ts)}</td>
                        <td class="mono dim">{cp.commit_hash?.slice(0, 7) ?? "—"}</td>
                        <td>{cp.message || "Checkpoint"}</td>
                        <td>
                          <button class="ghost small" onClick={() => restoreCommit(cp.seq)}>Restore</button>
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </Show>
          </Show>
        </Match>
      </Switch>
    </div>
  );
}

// ---- Integrations & Extensibility (MCP, Hooks, Skills, Tasks) ---------------

function IntegrationsView() {
  const [subTab, setSubTab] = createSignal<"mcp" | "hooks" | "skills" | "tasks">("mcp");

  // MCP State
  const [mcpData, { refetch: refetchMcp }] = createResource(() => api.mcpServers().catch(() => ({ servers: {} })));
  const [newMcpName, setNewMcpName] = createSignal("");
  const [newMcpCmd, setNewMcpCmd] = createSignal("");
  const [newMcpArgs, setNewMcpArgs] = createSignal("");

  // Hooks State
  const [hooksData, { refetch: refetchHooks }] = createResource(() => api.hooks().catch(() => ({ hooks: [] })));

  // Skills & Proposals State
  const [skillsData, { refetch: refetchSkills }] = createResource(() => api.skills().catch(() => ({ skills: [] })));
  const [proposalsData, { refetch: refetchProposals }] = createResource(() => api.skillProposals().catch(() => ({ proposals: [] })));

  // Tasks State
  const [tasksData, { refetch: refetchTasks }] = createResource(() => api.tasks().catch(() => ({ tasks: [] })));

  // MCP Save
  const addMcpServer = async () => {
    const name = newMcpName().trim();
    const cmd = newMcpCmd().trim();
    if (!name || !cmd) return;
    const current = mcpData()?.servers ?? {};
    const updated = {
      ...current,
      [name]: {
        command: cmd,
        args: newMcpArgs().trim() ? newMcpArgs().trim().split(" ") : [],
      },
    };
    try {
      await api.putMcpServers(updated);
      pushToast("info", `Added MCP server '${name}'`);
      setNewMcpName("");
      setNewMcpCmd("");
      setNewMcpArgs("");
      refetchMcp();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const deleteMcpServer = async (name: string) => {
    const current: Record<string, McpServerConfig> = { ...(mcpData()?.servers ?? {}) };
    delete current[name];
    try {
      await api.putMcpServers(current);
      pushToast("info", `Removed MCP server '${name}'`);
      refetchMcp();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  // Proposals
  const promoteSkill = async (id: string) => {
    try {
      await api.promoteProposal(id);
      pushToast("info", "Proposal promoted to active skill");
      refetchProposals();
      refetchSkills();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const rejectSkill = async (id: string) => {
    try {
      await api.rejectProposal(id);
      pushToast("info", "Proposal rejected");
      refetchProposals();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  // Tasks
  const runTask = async (id: string) => {
    try {
      await api.runTaskNow(id);
      pushToast("info", "Task executed");
      refetchTasks();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  return (
    <div class="view">
      <div class="tab-bar">
        <button class="tab-btn" classList={{ active: subTab() === "mcp" }} onClick={() => setSubTab("mcp")}>
          MCP Servers ({Object.keys(mcpData()?.servers ?? {}).length})
        </button>
        <button class="tab-btn" classList={{ active: subTab() === "hooks" }} onClick={() => setSubTab("hooks")}>
          Lifecycle Hooks ({hooksData()?.hooks?.length ?? 0})
        </button>
        <button class="tab-btn" classList={{ active: subTab() === "skills" }} onClick={() => setSubTab("skills")}>
          Skills &amp; Proposals ({skillsData()?.skills?.length ?? 0})
        </button>
        <button class="tab-btn" classList={{ active: subTab() === "tasks" }} onClick={() => setSubTab("tasks")}>
          Scheduled Tasks ({tasksData()?.tasks?.length ?? 0})
        </button>
      </div>

      <Switch>
        {/* MCP Tab */}
        <Match when={subTab() === "mcp"}>
          <div class="two-col">
            <section class="panel">
              <h2>Registered MCP Servers</h2>
              <Show when={!mcpData.loading} fallback={<div class="empty">Loading MCP servers…</div>}>
                <Show when={Object.keys(mcpData()?.servers ?? {}).length > 0} fallback={<div class="empty">No MCP servers registered in .vakcoder/config.toml</div>}>
                  <table class="table">
                    <thead><tr><th>name</th><th>command</th><th /></tr></thead>
                    <tbody>
                      <For each={Object.entries(mcpData()?.servers ?? {}) as [string, McpServerConfig][]}>
                        {([name, s]) => (
                          <tr>
                            <td class="mono bold">{name}</td>
                            <td class="mono dim">{s.command} {s.args?.join(" ")}</td>
                            <td><button class="danger small" onClick={() => deleteMcpServer(name)}>Remove</button></td>
                          </tr>
                        )}
                      </For>
                    </tbody>
                  </table>
                </Show>
              </Show>
            </section>

            <section class="panel">
              <h2>Add MCP Server</h2>
              <div class="form-row">
                <label>server name</label>
                <input placeholder="e.g. github, filesystem" value={newMcpName()} onInput={(e) => setNewMcpName(e.currentTarget.value)} />
              </div>
              <div class="form-row">
                <label>command</label>
                <input class="mono" placeholder="npx, python3, uvx..." value={newMcpCmd()} onInput={(e) => setNewMcpCmd(e.currentTarget.value)} />
              </div>
              <div class="form-row">
                <label>args</label>
                <input class="mono" placeholder="-y @modelcontextprotocol/server-..." value={newMcpArgs()} onInput={(e) => setNewMcpArgs(e.currentTarget.value)} />
              </div>
              <div class="row-gap" style="margin-top:12px">
                <button disabled={!newMcpName().trim() || !newMcpCmd().trim()} onClick={addMcpServer}>
                  Add Server
                </button>
              </div>
            </section>
          </div>
        </Match>

        {/* Hooks Tab */}
        <Match when={subTab() === "hooks"}>
          <section class="panel">
            <h2>Configured Lifecycle Hooks</h2>
            <Show when={!hooksData.loading} fallback={<div class="empty">Loading hooks…</div>}>
              <Show when={(hooksData()?.hooks?.length ?? 0) > 0} fallback={<div class="empty">No lifecycle hooks defined in config.toml.</div>}>
                <table class="table">
                  <thead><tr><th>event</th><th>matcher</th><th>command</th><th>timeout</th><th>status</th></tr></thead>
                  <tbody>
                    <For each={hooksData()?.hooks ?? []}>
                      {(h: HookConfig) => (
                        <tr>
                          <td><span class="chip chip-mode mono">{h.event}</span></td>
                          <td class="mono dim">{h.matcher ?? "—"}</td>
                          <td class="mono">{h.command}</td>
                          <td>{h.timeout_ms}ms</td>
                          <td><span class="chip chip-ok">enabled</span></td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </Show>
            </Show>
          </section>
        </Match>

        {/* Skills & Proposals Tab */}
        <Match when={subTab() === "skills"}>
          <div class="two-col">
            <section class="panel">
              <h2>Active Skills ({skillsData()?.skills?.length ?? 0})</h2>
              <Show when={!skillsData.loading} fallback={<div class="empty">Loading skills…</div>}>
                <Show when={(skillsData()?.skills?.length ?? 0) > 0} fallback={<div class="empty">No skills discovered in skills/ directories.</div>}>
                  <table class="table">
                    <thead><tr><th>name</th><th>description</th></tr></thead>
                    <tbody>
                      <For each={skillsData()?.skills ?? []}>
                        {(s: SkillItem) => (
                          <tr>
                            <td class="mono bold">{s.name}</td>
                            <td class="dim">{s.description || "Skill extension"}</td>
                          </tr>
                        )}
                      </For>
                    </tbody>
                  </table>
                </Show>
              </Show>
            </section>

            <section class="panel">
              <h2>AI Skill Proposals ({proposalsData()?.proposals?.length ?? 0})</h2>
              <Show when={!proposalsData.loading} fallback={<div class="empty">Loading proposals…</div>}>
                <Show when={(proposalsData()?.proposals?.length ?? 0) > 0} fallback={<div class="empty">No pending skill proposals.</div>}>
                  <ul class="hit-list">
                    <For each={proposalsData()?.proposals ?? []}>
                      {(p: SkillProposal) => (
                        <li class="inbox-item">
                          <div class="hit-meta">
                            <span class="mono bold">{p.name}</span>
                          </div>
                          <div class="hit-snippet">{p.description}</div>
                          <div class="row-gap" style="margin-top:8px">
                            <button class="approve small" onClick={() => promoteSkill(p.id)}>Promote</button>
                            <button class="danger small" onClick={() => rejectSkill(p.id)}>Reject</button>
                          </div>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
              </Show>
            </section>
          </div>
        </Match>

        {/* Tasks Tab */}
        <Match when={subTab() === "tasks"}>
          <section class="panel">
            <h2>Scheduled Tasks &amp; Automations</h2>
            <Show when={!tasksData.loading} fallback={<div class="empty">Loading tasks…</div>}>
              <Show when={(tasksData()?.tasks?.length ?? 0) > 0} fallback={<div class="empty">No scheduled tasks or cron jobs configured.</div>}>
                <table class="table">
                  <thead><tr><th>name</th><th>type</th><th>schedule / interval</th><th>model pin</th><th>last run</th><th>status</th><th /></tr></thead>
                  <tbody>
                    <For each={tasksData()?.tasks ?? []}>
                      {(t: TaskItem) => (
                        <tr>
                          <td class="bold">{t.name}</td>
                          <td><span class={`chip ${t.script ? "chip-tool" : "chip-mode"}`}>{t.script ? "script" : "prompt"}</span></td>
                          <td class="mono">{t.schedule ?? `${t.interval_secs ?? 3600}s`}</td>
                          <td class="mono dim">{t.model_pin ?? "default"}</td>
                          <td title={t.last_run_at ?? ""}>{t.last_run_at ? timeAgo(t.last_run_at) : "never"}</td>
                          <td><span class={`chip ${t.enabled ? "chip-ok" : "chip-warn"}`}>{t.enabled ? "enabled" : "disabled"}</span></td>
                          <td>
                            <button class="ghost small" onClick={() => runTask(t.id)}>Run now</button>
                          </td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </Show>
            </Show>
          </section>
        </Match>
      </Switch>
    </div>
  );
}

// ---- Memory & Recall -------------------------------------------------------

function MemoryView() {
  const [memoryData, { refetch }] = createResource(() => api.memory().catch(() => ({ notes: [] })));
  const [scope, setScope] = createSignal<"profile" | "project">("project");
  const [tag, setTag] = createSignal("");
  const [noteText, setNoteText] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  const addNote = async () => {
    if (!noteText().trim() || busy()) return;
    setBusy(true);
    try {
      await api.addMemory(scope(), noteText().trim(), tag().trim() || undefined);
      pushToast("info", "Memory note recorded");
      setNoteText("");
      setTag("");
      refetch();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const forget = async (id: string) => {
    try {
      await api.forgetMemory(id);
      pushToast("info", "Memory note forgotten");
      refetch();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  return (
    <div class="view">
      <div class="two-col">
        <section class="panel">
          <h2>Tiered Memory Notes ({memoryData()?.notes?.length ?? 0})</h2>
          <Show when={!memoryData.loading} fallback={<div class="empty">Loading memory…</div>}>
            <Show when={(memoryData()?.notes?.length ?? 0) > 0} fallback={<div class="empty">No memory notes recorded.</div>}>
              <ul class="hit-list">
                <For each={memoryData()?.notes ?? []}>
                  {(m: MemoryItem) => (
                    <li class="inbox-item">
                      <div class="hit-meta">
                        <span class={`chip ${m.scope === "profile" ? "chip-mode" : "chip-tool"}`}>{m.scope}</span>
                        <Show when={m.tag}><strong class="mono">{m.tag}</strong></Show>
                        <span class="when">{timeAgo(m.ts)}</span>
                      </div>
                      <div class="hit-snippet">{m.text}</div>
                      <div class="row-gap" style="margin-top:8px">
                        <button class="danger small" onClick={() => forget(m.id)}>Forget</button>
                      </div>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </Show>
        </section>

        <section class="panel">
          <h2>Record Knowledge Note</h2>
          <div class="form-row">
            <label>scope</label>
            <select value={scope()} onChange={(e) => setScope(e.currentTarget.value as "profile" | "project")}>
              <option value="project">Workspace Note</option>
              <option value="profile">USER.md Profile</option>
            </select>
          </div>
          <div class="form-row">
            <label>tag</label>
            <input placeholder="Optional tag slug" value={tag()} onInput={(e) => setTag(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>text</label>
            <textarea rows={4} placeholder="Note content to persist..." value={noteText()} onInput={(e) => setNoteText(e.currentTarget.value)} />
          </div>
          <div class="row-gap" style="margin-top:12px">
            <button disabled={busy() || !noteText().trim()} onClick={addNote}>
              {busy() ? "Recording…" : "Save Note"}
            </button>
          </div>
        </section>
      </div>
    </div>
  );
}

// ---- Search ----------------------------------------------------------------

function highlight(snippet: string): string {
  return snippet.replaceAll("<b>", "<mark>").replaceAll("</b>", "</mark>");
}

function SearchView() {
  const [q, setQ] = createSignal("");
  const [role, setRole] = createSignal("");
  const [hits, setHits] = createSignal<SearchHit[] | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");

  let timer: ReturnType<typeof setTimeout> | undefined;
  const runSearch = (query: string) => {
    clearTimeout(timer);
    if (!query.trim()) {
      setHits(null);
      return;
    }
    setBusy(true);
    timer = setTimeout(async () => {
      try {
        const res = await api.search(query.trim(), { role: role() || undefined, limit: 50 });
        setHits(res.hits);
        setError("");
      } catch (err) {
        if (err instanceof AuthRequired) setAuthed(false);
        else setError(`${err}`);
      } finally {
        setBusy(false);
      }
    }, 250);
  };

  return (
    <div class="view">
      <div class="search-hero">
        <input
          class="search-big"
          placeholder="Search every session… (FTS5 BM25 syntax supported)"
          value={q()}
          onInput={(e) => {
            setQ(e.currentTarget.value);
            runSearch(e.currentTarget.value);
          }}
        />
        <select value={role()} onChange={(e) => { setRole(e.currentTarget.value); runSearch(q()); }}>
          <For each={ROLE_FILTERS}>{(f) => <option value={f.id}>{f.label}</option>}</For>
        </select>
      </div>
      <Show when={error()}>
        <div class="login-error">{error()}</div>
      </Show>
      <Switch>
        <Match when={busy() && !hits()}>
          <div class="empty">Searching…</div>
        </Match>
        <Match when={hits() === null}>
          <div class="empty">Type to search across all indexed transcripts. Phrases in quotes, prefix with star.</div>
        </Match>
        <Match when={hits()?.length === 0}>
          <div class="empty">No matches.</div>
        </Match>
        <Match when={hits()}>
          <ul class="hit-list">
            <For each={hits() ?? []}>
              {(h) => (
                <li onClick={() => navigate(`#/sessions/${h.session_id}`)}>
                  <div class="hit-meta">
                    <span class="mono">{shortId(h.session_id)}</span>
                    <span class="chip chip-kind">{h.kind}</span>
                    <Show when={h.role}><span class="chip chip-role">{h.role}</span></Show>
                    <Show when={h.tool_name}><span class="chip chip-tool mono">{h.tool_name}</span></Show>
                    <span class="when">{timeAgo(h.ts)}</span>
                  </div>
                  <div class="hit-snippet" innerHTML={highlight(h.snippet)} />
                </li>
              )}
            </For>
          </ul>
        </Match>
      </Switch>
    </div>
  );
}

// ---- Security --------------------------------------------------------------

const SEC_KINDS = ["", "auth_failure", "rate_limit", "chat_allowlist", "permission_denial", "config_change", "provider_key_change", "full_access_grant", "full_access_revoke"];

function Security() {
  const [kind, setKind] = createSignal("");
  const [events, { refetch }] = createResource(() => api.security(500, kind() || undefined));

  return (
    <div class="view">
      <div class="toolbar">
        <div class="chips">
          <For each={SEC_KINDS}>
            {(k) => (
              <button
                class="chip-btn"
                classList={{ active: kind() === k }}
                onClick={() => setKind(k)}
              >
                {k === "" ? "All" : k.replaceAll("_", " ")}
              </button>
            )}
          </For>
        </div>
        <span class="spacer" />
        <button class="ghost" onClick={() => refetch()}>Refresh</button>
      </div>
      <Show when={!events.loading} fallback={<div class="empty">Loading audit log…</div>}>
        <Show
          when={(events()?.events.length ?? 0) > 0}
          fallback={<div class="empty">No security events recorded. Quiet is good.</div>}
        >
          <table class="table">
            <thead>
              <tr><th>when</th><th>kind</th><th>label</th><th>detail</th><th>source</th></tr>
            </thead>
            <tbody>
              <For each={events()?.events}>
                {(e: SecurityEvent) => (
                  <tr data-kind={e.kind}>
                    <td title={e.ts}>{timeAgo(e.ts)}</td>
                    <td><span class="chip" data-kind={e.kind}>{e.kind.replaceAll("_", " ")}</span></td>
                    <td>{e.label}</td>
                    <td class="mono dim wrap">{e.detail}</td>
                    <td class="mono dim">{e.ip ?? "—"}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </Show>
      </Show>
    </div>
  );
}

// ---- Inbox -----------------------------------------------------------------

const INBOX_KIND_TONE: Record<string, string> = {
  task_summary: "",
  approval_pending: "warn",
  approval_denied: "alert",
  budget_alert: "alert",
  digest: "",
  heartbeat: "",
  proposal_opened: "info",
};

function Inbox() {
  const [unreadOnly, setUnreadOnly] = createSignal(false);
  const [inbox, { refetch }] = createResource(() => api.inbox(unreadOnly()));
  const [acking, setAcking] = createSignal("");

  const ack = async (id: string) => {
    setAcking(id);
    try {
      await api.inboxAck(id);
      refetch();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setAcking("");
    }
  };

  return (
    <div class="view">
      <div class="toolbar">
        <label class="toggle">
          <input
            type="checkbox"
            checked={unreadOnly()}
            onChange={(e) => setUnreadOnly(e.currentTarget.checked)}
          />
          Unread only
        </label>
        <span class="spacer" />
        <button class="ghost" onClick={() => refetch()}>Refresh</button>
      </div>
      <Show when={!inbox.loading} fallback={<div class="empty">Loading inbox…</div>}>
        <Show
          when={(inbox()?.entries.length ?? 0) > 0}
          fallback={<div class="empty">Inbox zero.</div>}
        >
          <ul class="hit-list">
            <For each={inbox()!.entries}>
              {(e) => (
                <li class="inbox-item">
                  <div class="hit-meta">
                    <span class={`chip ${INBOX_KIND_TONE[e.kind] ? `chip-tone-${INBOX_KIND_TONE[e.kind]}` : ""}`}>
                      {e.kind.replaceAll("_", " ")}
                    </span>
                    <strong>{e.title}</strong>
                    <span class="when">{timeAgo(e.ts)}</span>
                  </div>
                  <div class="hit-snippet">{e.body}</div>
                  <div class="row-gap" style="margin-top:8px">
                    <Show when={e.session_id}>
                      <button class="ghost small" onClick={() => navigate(`#/sessions/${e.session_id}`)}>
                        View session
                      </button>
                    </Show>
                    <span class="spacer" />
                    <button class="ghost small" disabled={acking() === e.id} onClick={() => ack(e.id)}>
                      Acknowledge
                    </button>
                  </div>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>
    </div>
  );
}

// ---- Settings & Governance -------------------------------------------------

const MODES = ["ReadOnly", "WorkspaceWrite", "FullAccess"];

function Settings() {
  const [config, { refetch: refetchConfig }] = createResource(() => api.config());
  const [providersData, { refetch: refetchProviders }] = createResource(() => api.providers().catch(() => null));
  const [gateway] = createResource(() => api.gatewayStatus());
  const [rebuilding, setRebuilding] = createSignal(false);
  const [doctorReport, setDoctorReport] = createSignal<string | null>(null);
  const [runningDoctor, setRunningDoctor] = createSignal(false);

  // Editable provider & live model discovery
  const [selectedProvider, setSelectedProvider] = createSignal("anthropic");
  const [selectedModel, setSelectedModel] = createSignal("");
  const [discoveredModels, setDiscoveredModels] = createSignal<string[]>([]);
  const [loadingModels, setLoadingModels] = createSignal(false);
  const [providerKeyInput, setProviderKeyInput] = createSignal("");
  const [savingKey, setSavingKey] = createSignal(false);

  // Seed once on initial config load
  let initialized = false;
  createEffect(() => {
    const c = config();
    if (c && !initialized) {
      initialized = true;
      setSelectedProvider(c.provider || "anthropic");
      setSelectedModel(c.model || "");
    }
  });

  // Fetch live discovered models whenever provider changes
  createEffect(async () => {
    const prov = selectedProvider();
    if (!prov) return;
    setLoadingModels(true);
    try {
      const res = await api.models(prov);
      const models = res.models ?? [];
      setDiscoveredModels(models);
      if (models.length > 0 && !models.includes(selectedModel())) {
        setSelectedModel(models[0]);
      }
    } catch {
      setDiscoveredModels([]);
    } finally {
      setLoadingModels(false);
    }
  });

  const saveIdentity = async () => {
    try {
      await api.patchConfig({ provider: selectedProvider(), model: selectedModel() });
      pushToast("info", `Model updated: ${selectedProvider()} / ${selectedModel()}`);
      refetchConfig();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  const saveKey = async () => {
    if (!providerKeyInput().trim() || savingKey()) return;
    setSavingKey(true);
    try {
      await api.setProviderKey(selectedProvider(), providerKeyInput().trim());
      pushToast("info", `API key set for ${selectedProvider()}`);
      setProviderKeyInput("");
      refetchProviders();
      // Re-trigger discovery
      const res = await api.models(selectedProvider());
      setDiscoveredModels(res.models ?? []);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setSavingKey(false);
    }
  };

  const deleteKey = async () => {
    try {
      await api.deleteProviderKey(selectedProvider());
      pushToast("info", `API key revoked for ${selectedProvider()}`);
      refetchProviders();
      setDiscoveredModels([]);
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const switchMode = async (mode: string) => {
    try {
      await api.setMode(mode);
      pushToast("info", `Permission mode → ${mode}`);
      refetchConfig();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
      refetchConfig();
    }
  };

  const rebuild = async () => {
    setRebuilding(true);
    try {
      const stats = await api.rebuild();
      if (stats.ok) pushToast("info", `Rebuilt: ${stats.files_scanned} files, ${stats.entries_indexed} entries`);
      else pushToast("alert", `Rebuild failed: ${stats.error}`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setRebuilding(false);
    }
  };

  const runDoctor = async () => {
    setRunningDoctor(true);
    try {
      const doc = await api.doctor();
      setDoctorReport(doc.report);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setRunningDoctor(false);
    }
  };

  const signOut = async () => {
    await api.logout();
    disconnectEvents();
    setAuthed(false);
    navigate("#/overview");
  };

  return (
    <div class="view">
      <div class="two-col">
        {/* Model & Provider Live Discovery */}
        <section class="panel">
          <h2>Model Identity &amp; Live Discovery</h2>
          <Show when={!config.loading} fallback={<div class="empty">Loading…</div>}>
            <div class="form-row">
              <label>provider</label>
              <select value={selectedProvider()} onChange={(e) => setSelectedProvider(e.currentTarget.value)}>
                <option value="anthropic">Anthropic (Claude)</option>
                <option value="openai">OpenAI (Responses)</option>
                <option value="google">Google (Gemini)</option>
                <option value="ollama">Ollama (Local)</option>
              </select>
            </div>

            <div class="form-row">
              <label>model</label>
              <Show
                when={discoveredModels().length > 0}
                fallback={
                  <input
                    class="mono"
                    placeholder={loadingModels() ? "Discovering models from key…" : "e.g. claude-3-5-sonnet-20241022"}
                    value={selectedModel()}
                    onInput={(e) => setSelectedModel(e.currentTarget.value)}
                  />
                }
              >
                <select value={selectedModel()} onChange={(e) => setSelectedModel(e.currentTarget.value)}>
                  <For each={discoveredModels()}>{(m) => <option value={m}>{m}</option>}</For>
                </select>
              </Show>
            </div>

            <div class="row-gap" style="margin-top:10px">
              <button onClick={saveIdentity}>Save model</button>
              <button class="ghost small" onClick={() => api.models(selectedProvider()).then(r => setDiscoveredModels(r.models))}>
                {loadingModels() ? "Discovering…" : "Rediscover models"}
              </button>
            </div>

            {/* Provider Key Management */}
            <h2 style="margin-top:20px">API Key Management ({selectedProvider()})</h2>
            <div class="form-row">
              <label>key</label>
              <input type="password" placeholder="sk-..." value={providerKeyInput()} onInput={(e) => setProviderKeyInput(e.currentTarget.value)} />
            </div>
            <div class="row-gap">
              <button disabled={savingKey() || !providerKeyInput().trim()} onClick={saveKey}>
                {savingKey() ? "Saving…" : "Set Key"}
              </button>
              <button class="danger small" onClick={deleteKey}>Revoke Key</button>
            </div>
          </Show>
        </section>

        {/* Permissions & Governance */}
        <section class="panel">
          <h2>Permission Mode</h2>
          <div class="mode-grid">
            <For each={MODES}>
              {(m) => (
                <button
                  class="mode-btn"
                  classList={{ active: config()?.permission_mode === m }}
                  onClick={() => switchMode(m)}
                  disabled={config()?.permission_mode === m}
                >
                  <span class="mode-name">{m}</span>
                  <span class="mode-desc">
                    {m === "ReadOnly" && "Read-only tools; writes denied"}
                    {m === "WorkspaceWrite" && "Writes confined to workspace"}
                    {m === "FullAccess" && "Unsandboxed — explicit trust"}
                  </span>
                </button>
              )}
            </For>
          </div>

          <h2 style="margin-top:20px">Diagnostics &amp; System Health</h2>
          <div class="row-gap">
            <button class="ghost" disabled={runningDoctor()} onClick={runDoctor}>
              {runningDoctor() ? "Diagnosing…" : "Run Doctor Diagnostics"}
            </button>
            <button class="ghost" disabled={rebuilding()} onClick={rebuild}>
              {rebuilding() ? "Rebuilding…" : "Rebuild Search Index"}
            </button>
          </div>
          <Show when={doctorReport()}>
            <pre class="mono" style="margin-top:10px; max-height:180px; overflow:auto; background:var(--bg); padding:8px; border-radius:6px">
              {doctorReport()}
            </pre>
          </Show>

          <h2 style="margin-top:20px">Gateway &amp; Security</h2>
          <Show when={!gateway.loading}>
            <dl class="kv">
              <dt>gateway</dt>
              <dd><span class="chip" data-on={gateway()?.enabled}>{gateway()?.enabled ? "enabled" : "disabled"}</span></dd>
              <dt>bindings</dt>
              <dd>{gateway()?.bindings.length ? gateway()!.bindings.join(", ") : "none"}</dd>
            </dl>
          </Show>
          <div class="row-gap" style="margin-top:16px">
            <button class="danger" onClick={signOut}>Sign out</button>
          </div>
        </section>
      </div>
    </div>
  );
}

// ---- Shell -----------------------------------------------------------------

const NAV = [
  { hash: "#/overview", label: "Overview", icon: ICONS.overview },
  { hash: "#/sessions", label: "Sessions", icon: ICONS.sessions },
  { hash: "#/integrations", label: "Integrations", icon: ICONS.integrations },
  { hash: "#/memory", label: "Memory", icon: ICONS.memory },
  { hash: "#/search", label: "Search", icon: ICONS.search },
  { hash: "#/inbox", label: "Inbox", icon: ICONS.inbox, badge: () => unread().toString() || "" },
  { hash: "#/security", label: "Security", icon: ICONS.security },
  { hash: "#/settings", label: "Settings", icon: ICONS.settings },
];

export default function App() {
  const probeAuth = () => {
    api.config()
      .then(() => {
        setAuthed(true);
        connectEvents();
      })
      .catch(() => setAuthed(false));
  };

  createEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const linkToken = params.get("token");
    if (!linkToken) {
      probeAuth();
      return;
    }
    const url = new URL(window.location.href);
    url.searchParams.delete("token");
    window.history.replaceState(null, "", url.toString());
    api.login(linkToken)
      .then(() => {
        setAuthed(true);
        connectEvents();
      })
      .catch(() => probeAuth());
  });

  createEffect(() => {
    if (authed() !== true) return;
    let alive = true;
    const tick = async () => {
      try {
        const { count } = await api.unreadCount();
        if (alive) setUnread(count);
      } catch { /* transient */ }
    };
    void tick();
    const t = setInterval(tick, 30_000);
    onCleanup(() => {
      alive = false;
      clearInterval(t);
    });
  });

  const currentRoute = () => {
    const r = route();
    if (r.startsWith("#/sessions/")) return "transcript";
    return NAV.find((n) => r.startsWith(n.hash))?.hash ?? "#/overview";
  };

  return (
    <Switch>
      <Match when={authed() === null}>
        <div class="boot"><div class="spin" /></div>
      </Match>
      <Match when={authed() === false}>
        <Login />
      </Match>
      <Match when={authed() === true}>
        <div class="shell">
          <aside class="sidebar">
            <div class="brand"><span class="brand-mark">◆</span> vakcoder</div>
            <nav>
              <For each={NAV}>
                {(item) => (
                  <a href={item.hash} classList={{ active: currentRoute() === item.hash }}>
                    <Icon d={item.icon} />
                    {item.label}
                    <Show when={"badge" in item && item.badge?.() && Number(item.badge!()) > 0}>
                      <span class="nav-badge">{item.badge!()}</span>
                    </Show>
                  </a>
                )}
              </For>
            </nav>
            <div class="sidebar-foot">
              <span class={`dot dot-${conn()}`} />
              <span class="conn-label">{conn()}</span>
            </div>
          </aside>
          <main class="main">
            <Switch>
              <Match when={currentRoute() === "#/overview"}><Overview /></Match>
              <Match when={currentRoute() === "#/sessions"}><Sessions /></Match>
              <Match when={currentRoute() === "transcript"}>
                <Transcript sessionId={route().slice("#/sessions/".length)} />
              </Match>
              <Match when={currentRoute() === "#/integrations"}><IntegrationsView /></Match>
              <Match when={currentRoute() === "#/memory"}><MemoryView /></Match>
              <Match when={currentRoute() === "#/search"}><SearchView /></Match>
              <Match when={currentRoute() === "#/inbox"}><Inbox /></Match>
              <Match when={currentRoute() === "#/security"}><Security /></Match>
              <Match when={currentRoute() === "#/settings"}><Settings /></Match>
            </Switch>
          </main>
          <div class="toasts">
            <For each={toasts()}>
              {(t) => <div class={`toast toast-${t.kind}`}>{t.text}</div>}
            </For>
          </div>
        </div>
      </Match>
    </Switch>
  );
}
