import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { api, AuthRequired } from "./api";
import { clock, shortId, timeAgo } from "./time";
import {
  approvalsVersion, authed, conn, connectEvents, disconnectEvents, feed, navigate, pushToast,
  route, setAuthed, toasts,
} from "./store";
import type {
  ConfigInfo, DigestReport, DiscoveredSkill, DoctorReport, FinopsStatus, HookConfig,
  McpServerDef, NoteBlock, OpsDiagnostics, OpsStatusShape, PendingApproval, ProvidersResponse,
  SearchHit, SecurityEvent, SessionListItem, SkillProposal, TaskDef, TranscriptEntry,
} from "./types";

// Unread inbox badge: polled lightly while signed in; refreshed by the
// Inbox view's own fetches.
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
  search: "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35",
  inbox: "M22 12h-6l-2 3h-4l-2-3H2M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
  security: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z",
  tasks: "M9 11l3 3L22 4M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11",
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

function StatCard(props: { label: string; value: string | number; sub?: string; tone?: string }) {
  return (
    <div class="stat-card" data-tone={props.tone ?? "default"}>
      <div class="stat-value">{props.value}</div>
      <div class="stat-label">{props.label}</div>
      <Show when={props.sub}>
        <div class="stat-sub">{props.sub}</div>
      </Show>
    </div>
  );
}

function Overview() {
  const [health, healthActions] = createResource(() => api.health());
  const [sessions] = createResource(() => api.sessions());
  const [security] = createResource(() => api.security(500));

  const recentSecurity = createMemo(
    () =>
      (security()?.events ?? []).filter((e) => Date.now() - new Date(e.ts).getTime() < 86_400_000)
        .length,
  );

  const feedItems = createMemo(() => [...feed()].reverse());

  return (
    <div class="view">
      <div class="stats-row">
        <StatCard label="Sessions indexed" value={sessions()?.sessions.length ?? "…"} />
        <StatCard
          label="Total entries"
          value={sessions()?.sessions.reduce((a, s) => a + s.entry_count, 0) ?? "…"}
        />
        <StatCard
          label="Security events · 24h"
          value={security.loading ? "…" : recentSecurity()}
          tone={recentSecurity() > 0 ? "warn" : undefined}
        />
        <StatCard
          label="Gateway"
          value={health()?.status === "ok" ? "ready" : "…"}
          sub={health()?.provider}
        />
      </div>

      <ApprovalsCard />

      <div class="two-col">
        <section class="panel">
          <h2>System</h2>
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
              <dt>workspace</dt>
              <dd class="mono wrap">{health()?.cwd}</dd>
            </dl>
            <Show when={(health()?.warnings?.length ?? 0) > 0}>
              <div class="warnings">
                <For each={health()?.warnings}>{(w) => <div class="warning">⚠ {w}</div>}</For>
              </div>
            </Show>
            <button class="ghost small" onClick={() => healthActions.refetch()}>
              Refresh
            </button>
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
  // Refetches live: approvalsVersion bumps on ApprovalRequested/Granted/Denied.
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
    <section class="panel" classList={{ "panel-alert": (pending()?.total ?? 0) > 0 }}>
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

// ---- Sessions list ---------------------------------------------------------

function Sessions() {
  const [sessions, { refetch }] = createResource(() => api.sessions());
  const [q, setQ] = createSignal("");
  const [creating, setCreating] = createSignal(false);

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

  const filtered = createMemo(() => {
    const needle = q().toLowerCase();
    return (sessions()?.sessions ?? []).filter(
      (s: SessionListItem) => !needle || s.session_id.toLowerCase().includes(needle),
    );
  });

  return (
    <div class="view">
      <div class="toolbar">
        <input class="search-input" placeholder="Filter by session id…" value={q()} onInput={(e) => setQ(e.currentTarget.value)} />
        <span class="spacer" />
        <button disabled={creating()} onClick={newSession}>
          {creating() ? "Creating…" : "+ New session"}
        </button>
        <button class="ghost" onClick={() => refetch()}>Refresh</button>
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

// ---- Transcript ------------------------------------------------------------

const KIND_FILTERS = [
  { id: "", label: "All" },
  { id: "message", label: "Messages" },
];

const ROLE_FILTERS = [
  { id: "", label: "Everyone" },
  { id: "user", label: "User" },
  { id: "assistant", label: "Assistant" },
];

function Transcript(props: { sessionId: string }) {
  const [kind, setKind] = createSignal("");
  const [role, setRole] = createSignal("");
  const [entries, setEntries] = createSignal<TranscriptEntry[]>([]);
  const [hasMore, setHasMore] = createSignal(false);
  const [loading, setLoading] = createSignal(true);
  const [live, setLive] = createSignal(false);
  const [running, setRunning] = createSignal(false);

  const PAGE = 100;
  let offset = 0;

  const load = async (reset: boolean) => {
    setLoading(true);
    try {
      if (reset) offset = 0;
      const res = await api.transcript(props.sessionId, {
        limit: PAGE,
        offset,
        kind: kind() || undefined,
        role: role() || undefined,
        refresh: true,
      });
      // API returns ascending; newest last.
      setEntries(res.entries);
      setHasMore(res.has_more);
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

  // Live tail: subscribe to the session's own event stream; any activity
  // refreshes the (re-imported) transcript.
  createEffect(() => {
    void props.sessionId;
    if (!live()) return;
    const es = new EventSource(`/sessions/${encodeURIComponent(props.sessionId)}/events`);
    es.onmessage = (m) => {
      try {
        const ev = JSON.parse(m.data) as { type?: string };
        if (ev.type === "TurnStart" || ev.type === "ToolCallStart") setRunning(true);
        if (ev.type === "RunFinished") setRunning(false);
      } catch { /* ignore */ }
      clearTimeout((es as unknown as { t?: number }).t);
      (es as unknown as { t?: number }).t = setTimeout(() => load(true), 300) as unknown as number;
    };
    onCleanup(() => es.close());
  });

  const changePage = (delta: number) => {
    offset = Math.max(0, offset + delta * PAGE);
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
        // Mid-run: queue as steering so the loop consumes it between steps.
        await api.steer(props.sessionId, text);
        pushToast("info", "Steering queued");
      } else if (nCandidates() >= 2) {
        const res = await api.startBestofn(props.sessionId, text, nCandidates());
        pushToast("info", `Fanned out to ${res.runs.length} candidate runs — compare in Sessions`);
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

  const composerPlaceholder = () =>
    running()
      ? "Queue steering for the active run…"
      : nCandidates() >= 2
        ? `Fan this prompt across ${nCandidates()} isolated worktrees…`
        : "Send a prompt to this session…";

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
            <button class="ghost small" disabled={offset === 0} onClick={() => changePage(-1)}>‹ Newer</button>
            <span class="dim">page {Math.floor(offset / PAGE) + 1}</span>
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
          placeholder={composerPlaceholder()}
          value={draft()}
          onInput={(e) => setDraft(e.currentTarget.value)}
          onKeyDown={onKey}
          disabled={sending()}
        />
        <button onClick={() => void send()} disabled={sending() || !draft().trim()}>
          {sending() ? "…" : running() && nCandidates() === 1 ? "Steer" : "Send"}
        </button>
      </div>
    </div>
  );
}

// ---- Search ----------------------------------------------------------------

function highlight(snippet: string): string {
  // Server emits <b>…</b>; convert to <mark>.
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
          placeholder="Search every session… (FTS5 syntax supported)"
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
                  {/* snippet comes from our own server; markers are inserted by us */}
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

const SEC_KINDS = ["", "auth_failure", "rate_limit", "chat_allowlist", "config_change", "provider_key_change"];

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

// ---- Settings --------------------------------------------------------------

const MODES = ["ReadOnly", "WorkspaceWrite", "FullAccess"];

const SETTINGS_TABS = [
  { id: "agent", label: "Agent" },
  { id: "integrations", label: "Integrations" },
  { id: "learning", label: "Learning" },
  { id: "tasks", label: "Tasks" },
  { id: "services", label: "Services" },
  { id: "reliability", label: "Reliability" },
  { id: "advanced", label: "Advanced" },
] as const;

type SettingsTab = (typeof SETTINGS_TABS)[number]["id"];

function Settings() {
  const [tab, setTab] = createSignal<SettingsTab>("agent");

  return (
    <div class="view">
      <div class="settings-layout">
        <nav class="settings-tabs">
          <For each={SETTINGS_TABS}>
            {(t) => (
              <button
                classList={{ active: tab() === t.id }}
                onClick={() => setTab(t.id)}
              >
                {t.label}
              </button>
            )}
          </For>
        </nav>
        <div class="settings-body">
          {tab() === "agent" && <SettingsAgent />}
          {tab() === "integrations" && <SettingsIntegrations />}
          {tab() === "learning" && <SettingsLearning />}
          {tab() === "tasks" && <SettingsTasks />}
          {tab() === "services" && <SettingsServices />}
          {tab() === "reliability" && <SettingsReliability />}
          {tab() === "advanced" && <SettingsAdvanced />}
        </div>
      </div>
    </div>
  );
}

// ---- Settings: Agent --------------------------------------------------------

function SettingsAgent() {
  const [config, { refetch }] = createResource(() => api.config());
  const [providers, setProviders] = createSignal<ProvidersResponse | null>(null);
  const [catalog, setCatalog] = createSignal<string[]>([]);
  const [discovering, setDiscovering] = createSignal(false);
  const [customModel, setCustomModel] = createSignal(false);

  const [provider, setProvider] = createSignal("");
  const [model, setModel] = createSignal("");
  const [maxTurns, setMaxTurns] = createSignal(0);

  // Credential state
  const [keyInput, setKeyInput] = createSignal("");
  const [keySaving, setKeySaving] = createSignal(false);
  const [keyRemoving, setKeyRemoving] = createSignal(false);

  // Load providers on mount
  const [providersLoading, setProvidersLoading] = createSignal(true);
  createEffect(() => {
    setProvidersLoading(true);
    void api
      .listProviders()
      .then(setProviders)
      .catch(() => {})
      .finally(() => setProvidersLoading(false));
  });

  // Seed editable fields from config (only after providers have loaded)
  createEffect(() => {
    const c = config();
    if (c && !providersLoading()) {
      setProvider(c.provider);
      setModel(c.model);
      setMaxTurns(c.max_turns);
    }
  });

  // Discover models when provider changes
  createEffect(() => {
    const p = provider();
    if (!p) return;
    setDiscovering(true);
    setCatalog([]);
    setCustomModel(false);
    void api
      .discoverModels(p)
      .then((r) => setCatalog(r.models))
      .catch(() => setCatalog([]))
      .finally(() => setDiscovering(false));
  });

  const dirty = createMemo(() => {
    const c = config();
    return (
      !!c &&
      (provider() !== c.provider ||
        model() !== c.model ||
        maxTurns() !== c.max_turns)
    );
  });

  const save = async () => {
    try {
      await api.patchConfig({
        provider: provider().trim(),
        model: model().trim(),
        max_turns: maxTurns(),
      });
      pushToast("info", "Agent settings updated");
      refetch();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  const reset = () => {
    const c = config();
    if (c) {
      setProvider(c.provider);
      setModel(c.model);
      setMaxTurns(c.max_turns);
    }
  };

  const addKey = async () => {
    const p = provider();
    const k = keyInput().trim();
    if (!p || !k) return;
    setKeySaving(true);
    try {
      await api.putProviderKey(p, k);
      setKeyInput("");
      pushToast("info", "API key saved");
      const fresh = await api.listProviders();
      setProviders(fresh);
      refetch();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setKeySaving(false);
    }
  };

  const removeKey = async () => {
    const p = provider();
    if (!p) return;
    setKeyRemoving(true);
    try {
      const res = await api.removeProviderKey(p);
      if (res.shadowed_by_env) pushToast("warn", "Key removed, but a real env var still authenticates this provider");
      else pushToast("info", "API key removed");
      const fresh = await api.listProviders();
      setProviders(fresh);
      refetch();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setKeyRemoving(false);
    }
  };

  const currentProviderInfo = createMemo(() =>
    providers()?.providers.find((p) => p.name === provider()),
  );

  return (
    <div class="two-col">
      <section class="panel">
        <h2>Provider &amp; model</h2>
        <Show when={!config.loading && !providersLoading()} fallback={<div class="empty">Loading…</div>}>
          <div class="form-row">
            <label>provider</label>
            <select value={provider()} onChange={(e) => setProvider(e.currentTarget.value)}>
              <For each={providers()?.providers ?? []}>
                {(p) => (
                  <option value={p.name}>
                    {p.name}{p.configured ? " ✓" : ""}
                  </option>
                )}
              </For>
            </select>
          </div>

          <div class="form-row">
            <label>model</label>
            <Show
              when={!customModel()}
              fallback={
                <div class="model-custom-row">
                  <input
                    class="mono"
                    placeholder="model id"
                    value={model()}
                    onInput={(e) => setModel(e.currentTarget.value)}
                  />
                  <button class="ghost small" onClick={() => setCustomModel(false)}>
                    Choose from list
                  </button>
                </div>
              }
            >
              <select
                class="mono"
                value={model()}
                onChange={(e) => setModel(e.currentTarget.value)}
              >
                <For each={catalog()}>
                  {(m) => <option value={m}>{m}</option>}
                </For>
                <option value={model()}>
                  {model() || "—"}
                </option>
              </select>
            </Show>
            <div class="form-hint">
              <Show when={discovering()}>Discovering models…</Show>
              <Show when={!discovering() && catalog().length === 0 && provider()}>
                No models discovered. <button class="link-btn" onClick={() => setCustomModel(true)}>Enter manually</button>
              </Show>
              <Show when={!discovering() && catalog().length > 0 && !customModel()}>
                {catalog().length} models available.
                {" "}
                <button class="link-btn" onClick={() => setCustomModel(true)}>Enter custom id</button>
              </Show>
            </div>
          </div>

          <div class="form-row">
            <label>max turns</label>
            <input
              type="number"
              min={1}
              max={1000}
              value={maxTurns()}
              onInput={(e) => setMaxTurns(Number(e.currentTarget.value))}
            />
          </div>

          <div class="row-gap">
            <button disabled={!dirty() || !provider().trim() || !model().trim()} onClick={save}>
              Apply changes
            </button>
            <Show when={dirty()}>
              <button class="ghost" onClick={reset}>Reset</button>
            </Show>
          </div>
        </Show>
      </section>

      <section class="panel">
        <h2>Credentials</h2>
        <Show when={currentProviderInfo()} fallback={<div class="empty">Select a provider first.</div>}>
          <dl class="kv">
            <dt>provider</dt><dd>{currentProviderInfo()!.name}</dd>
            <dt>env var</dt><dd class="mono">{currentProviderInfo()!.env_var ?? "—"}</dd>
            <dt>configured</dt>
            <dd>
              <span class="chip" data-on={currentProviderInfo()!.configured}>
                {currentProviderInfo()!.configured ? "yes" : "no"}
              </span>
            </dd>
          </dl>
        </Show>

        <div class="form-row" style="margin-top:14px">
          <label>API key</label>
          <div class="key-input-row">
            <input
              type="password"
              placeholder={currentProviderInfo()?.configured ? "Replace key…" : "Add key…"}
              value={keyInput()}
              onInput={(e) => setKeyInput(e.currentTarget.value)}
              onKeyDown={(e) => { if (e.key === "Enter") void addKey(); }}
            />
            <button disabled={keySaving() || !keyInput().trim()} onClick={() => void addKey()}>
              {keySaving() ? "Saving…" : "Save key"}
            </button>
          </div>
        </div>

        <div class="row-gap" style="margin-top:10px">
          <button
            class="danger small"
            disabled={keyRemoving() || !currentProviderInfo()?.configured}
            onClick={() => void removeKey()}
          >
            {keyRemoving() ? "Removing…" : "Remove key"}
          </button>
        </div>

        <h2 style="margin-top:22px">Permission mode</h2>
        <div class="mode-grid">
          <For each={MODES}>
            {(m) => (
              <button
                class="mode-btn"
                classList={{ active: config()?.permission_mode === m }}
                onClick={async () => {
                  try {
                    await api.setMode(m);
                    pushToast("info", `Permission mode → ${m}`);
                    refetch();
                  } catch (err) {
                    if (err instanceof AuthRequired) setAuthed(false);
                    else pushToast("alert", `${err}`);
                    refetch();
                  }
                }}
                disabled={config()?.permission_mode === m}
              >
                <span class="mode-name">{m}</span>
                <span class="mode-desc">
                  {m === "ReadOnly" && "Read-only tools; nothing is written"}
                  {m === "WorkspaceWrite" && "Writes confined to the workspace"}
                  {m === "FullAccess" && "Unsandboxed — explicit human trust"}
                </span>
              </button>
            )}
          </For>
        </div>
      </section>
    </div>
  );
}

// ---- Settings: Integrations -------------------------------------------------

function SettingsIntegrations() {
  const [subTab, setSubTab] = createSignal<"mcp" | "skills" | "hooks">("mcp");

  return (
    <div>
      <div class="capability-tabs">
        <button classList={{ active: subTab() === "mcp" }} onClick={() => setSubTab("mcp")}>
          MCP servers
        </button>
        <button classList={{ active: subTab() === "skills" }} onClick={() => setSubTab("skills")}>
          Skills
        </button>
        <button classList={{ active: subTab() === "hooks" }} onClick={() => setSubTab("hooks")}>
          Hooks
        </button>
      </div>
      {subTab() === "mcp" && <McpPanel />}
      {subTab() === "skills" && <SkillsPanel />}
      {subTab() === "hooks" && <HooksPanel />}
    </div>
  );
}

function McpPanel() {
  const [servers, setServers] = createSignal<Record<string, McpServerDef>>({});
  const [dirty, setDirty] = createSignal(false);
  const [saving, setSaving] = createSignal(false);

  createEffect(() => {
    void api
      .getMcpServers()
      .then((r) => setServers(r.servers ?? {}))
      .catch(() => {});
  });

  const mark = () => setDirty(true);

  const addServer = () => {
    const existing = Object.keys(servers());
    let name = "new-server";
    let i = 2;
    while (existing.includes(name)) name = `new-server-${i++}`;
    setServers({ ...servers(), [name]: { command: "", args: [], env: {}, network: false } });
    mark();
  };

  const removeServer = (name: string) => {
    const copy = { ...servers() };
    delete copy[name];
    setServers(copy);
    mark();
  };

  const updateServer = (name: string, patch: Partial<McpServerDef>) => {
    setServers({ ...servers(), [name]: { ...servers()[name], ...patch } });
    mark();
  };

  const renameServer = (oldName: string, newName: string) => {
    if (!newName || newName === oldName) return;
    const copy = { ...servers() };
    copy[newName] = copy[oldName];
    delete copy[oldName];
    setServers(copy);
    mark();
  };

  const save = async () => {
    setSaving(true);
    try {
      await api.putMcpServers(servers());
      setDirty(false);
      pushToast("info", "MCP servers saved");
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div>
      <h3>Tool servers</h3>
      <For each={Object.entries(servers())}>
        {([name, srv]) => (
          <div class="setting-row">
            <div class="setting-row-head">
              <input
                class="inline-name"
                value={name}
                onChange={(e) => renameServer(name, e.currentTarget.value)}
              />
              <span class="chip" data-on={true}>Configured</span>
              <span class="spacer" />
              <button class="danger small" onClick={() => removeServer(name)}>Remove</button>
            </div>
            <div class="setting-row-body">
              <div class="form-row">
                <label>command</label>
                <input value={srv.command} onInput={(e) => updateServer(name, { command: e.currentTarget.value })} />
              </div>
              <div class="form-row">
                <label>arguments</label>
                <input
                  value={srv.args.join(" ")}
                  onInput={(e) => updateServer(name, { args: e.currentTarget.value.split(/\s+/).filter(Boolean) })}
                  placeholder="space-separated args"
                />
              </div>
              <label class="toggle" style="margin-top:8px">
                <input
                  type="checkbox"
                  checked={srv.network}
                  onChange={(e) => updateServer(name, { network: e.currentTarget.checked })}
                />
                Allow outbound network
              </label>
            </div>
          </div>
        )}
      </For>
      <div class="row-gap" style="margin-top:12px">
        <button class="ghost" onClick={addServer}>Add server</button>
        <span class="spacer" />
        <Show when={dirty()}>
          <span class="unsaved-indicator">Unsaved changes</span>
        </Show>
        <button disabled={!dirty() || saving()} onClick={() => void save()}>
          {saving() ? "Saving…" : "Save & apply"}
        </button>
      </div>
    </div>
  );
}

function SkillsPanel() {
  const [skills, setSkills] = createSignal<DiscoveredSkill[]>([]);
  const [proposals, setProposals] = createSignal<SkillProposal[]>([]);

  const refresh = async () => {
    try {
      const [s, p] = await Promise.all([api.listSkills(), api.listProposals()]);
      setSkills(s.skills ?? []);
      setProposals(p.proposals ?? []);
    } catch {
      // partial load is fine
    }
  };

  createEffect(() => { void refresh(); });

  const promote = async (id: string) => {
    try {
      await api.promoteProposal(id);
      pushToast("info", "Proposal promoted");
      void refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const reject = async (id: string) => {
    try {
      await api.rejectProposal(id);
      pushToast("info", "Proposal rejected");
      void refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  return (
    <div>
      <h3>Discovered skills ({skills().length})</h3>
      <Show
        when={skills().length > 0}
        fallback={<div class="empty">No skills discovered.</div>}
      >
        <For each={skills()}>
          {(s) => (
            <details class="skill-row">
              <summary>
                <span class="skill-name">{s.name}</span>
                <Show when={s.scope}><span class="chip">{s.scope}</span></Show>
                <span class="chip" data-on={true}>Available</span>
              </summary>
              <div class="skill-body">{s.description}</div>
            </details>
          )}
        </For>
      </Show>

      <h3 style="margin-top:16px">Pending proposals ({proposals().length})</h3>
      <Show
        when={proposals().length > 0}
        fallback={<div class="empty">Queue is empty.</div>}
      >
        <For each={proposals()}>
          {(p) => (
            <div class="setting-row">
              <strong>{p.name}</strong>
              <div class="dim">{p.description}</div>
              <div class="row-gap" style="margin-top:8px">
                <button class="approve small" onClick={() => void promote(p.id)}>Promote</button>
                <button class="danger small" onClick={() => void reject(p.id)}>Reject</button>
              </div>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
}

function HooksPanel() {
  const [hooks, setHooks] = createSignal<HookConfig[]>([]);
  const [dirty, setDirty] = createSignal(false);
  const [saving, setSaving] = createSignal(false);

  createEffect(() => {
    void api
      .getHooks()
      .then((r) => setHooks(r.hooks ?? []))
      .catch(() => {});
  });

  const mark = () => setDirty(true);

  const addHook = () => {
    setHooks([
      ...hooks(),
      { event: "pre_tool_use", matcher: "", command: "", timeout_ms: 10000, enabled: true },
    ]);
    mark();
  };

  const removeHook = (i: number) => {
    setHooks(hooks().filter((_, idx) => idx !== i));
    mark();
  };

  const updateHook = (i: number, patch: Partial<HookConfig>) => {
    setHooks(hooks().map((h, idx) => (idx === i ? { ...h, ...patch } : h)));
    mark();
  };

  const save = async () => {
    setSaving(true);
    try {
      await api.putHooks(hooks());
      setDirty(false);
      pushToast("info", "Hooks saved");
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setSaving(false);
    }
  };

  const EVENTS = ["session_start", "pre_tool_use", "post_tool_use", "stop"];

  return (
    <div>
      <p class="form-hint">Hooks run commands at controlled lifecycle points.</p>
      <For each={hooks()}>
        {(h, i) => (
          <div class="setting-row">
            <div class="setting-row-head">
              <label class="toggle">
                <input
                  type="checkbox"
                  checked={h.enabled ?? true}
                  onChange={(e) => updateHook(i(), { enabled: e.currentTarget.checked })}
                />
                {h.enabled !== false ? "Enabled" : "Disabled"}
              </label>
              <span class="spacer" />
              <button class="danger small" onClick={() => removeHook(i())}>Remove</button>
            </div>
            <div class="setting-row-body">
              <div class="form-row">
                <label>event</label>
                <select value={h.event} onChange={(e) => updateHook(i(), { event: e.currentTarget.value })}>
                  <For each={EVENTS}>{(ev) => <option value={ev}>{ev}</option>}</For>
                </select>
              </div>
              <div class="form-row">
                <label>tool matcher</label>
                <input
                  value={h.matcher ?? ""}
                  onInput={(e) => updateHook(i(), { matcher: e.currentTarget.value || null })}
                  placeholder="bash, write, or leave blank"
                />
              </div>
              <div class="form-row">
                <label>command</label>
                <input
                  value={h.command}
                  onInput={(e) => updateHook(i(), { command: e.currentTarget.value })}
                  placeholder="e.g. cargo fmt --all --check"
                />
              </div>
              <div class="form-row">
                <label>timeout (ms)</label>
                <input
                  type="number"
                  min={100}
                  max={120000}
                  value={h.timeout_ms ?? 10000}
                  onInput={(e) => updateHook(i(), { timeout_ms: Number(e.currentTarget.value) })}
                />
              </div>
            </div>
          </div>
        )}
      </For>
      <div class="row-gap" style="margin-top:12px">
        <button class="ghost" onClick={addHook}>Add hook</button>
        <span class="spacer" />
        <Show when={dirty()}>
          <span class="unsaved-indicator">Unsaved changes</span>
        </Show>
        <button disabled={!dirty() || saving()} onClick={() => void save()}>
          {saving() ? "Saving…" : "Save & apply"}
        </button>
      </div>
    </div>
  );
}

// ---- Settings: Learning -----------------------------------------------------

function SettingsLearning() {
  const [notes, setNotes] = createSignal<NoteBlock[]>([]);
  const [proposals, setProposals] = createSignal<SkillProposal[]>([]);
  const [tier, setTier] = createSignal<"workspace" | "profile">("workspace");
  const [editingId, setEditingId] = createSignal<string | null>(null);
  const [editText, setEditText] = createSignal("");
  const [adding, setAdding] = createSignal(false);
  const [addKind, setAddKind] = createSignal("preference");
  const [addTag, setAddTag] = createSignal("");
  const [addText, setAddText] = createSignal("");

  const refresh = async () => {
    try {
      const [n, p] = await Promise.all([api.listMemory(), api.listProposals()]);
      setNotes(n.notes ?? []);
      setProposals(p.proposals ?? []);
    } catch {
      // partial load
    }
  };

  createEffect(() => { void refresh(); });

  const tierNotes = createMemo(() =>
    notes().filter((n) => (n.scope ?? "workspace") === tier()),
  );

  const forgetNote = async (id: string) => {
    try {
      await api.forgetMemory(id, tier());
      pushToast("info", "Note forgotten");
      void refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const amendNote = async (id: string) => {
    try {
      await api.amendMemory(id, tier(), editText());
      setEditingId(null);
      pushToast("info", "Note amended");
      void refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const addNote = async () => {
    try {
      await api.appendMemory({
        kind: addKind(),
        tag: addTag(),
        text: addText(),
        scope: tier(),
      });
      setAdding(false);
      setAddText("");
      setAddTag("");
      pushToast("info", "Note added");
      void refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const promote = async (id: string) => {
    try {
      await api.promoteProposal(id);
      pushToast("info", "Promoted");
      void refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const reject = async (id: string) => {
    try {
      await api.rejectProposal(id);
      pushToast("info", "Rejected");
      void refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  return (
    <div>
      <Show when={proposals().length > 0}>
        <h3>Skill proposals ({proposals().length})</h3>
        <For each={proposals()}>
          {(p) => (
            <div class="setting-row">
              <strong>{p.name}</strong>
              <div class="dim">{p.description}</div>
              <div class="row-gap" style="margin-top:8px">
                <button class="approve small" onClick={() => void promote(p.id)}>Promote</button>
                <button class="danger small" onClick={() => void reject(p.id)}>Reject</button>
              </div>
            </div>
          )}
        </For>
      </Show>

      <h3 style="margin-top:16px">Memory notes</h3>
      <div class="capability-tabs" style="margin-bottom:12px">
        <button classList={{ active: tier() === "workspace" }} onClick={() => setTier("workspace")}>
          Workspace ({notes().filter((n) => (n.scope ?? "workspace") === "workspace").length})
        </button>
        <button classList={{ active: tier() === "profile" }} onClick={() => setTier("profile")}>
          Profile ({notes().filter((n) => n.scope === "profile").length})
        </button>
      </div>

      <Show when={tier() === "profile"}>
        <Show
          when={adding()}
          fallback={
            <button class="ghost" onClick={() => setAdding(true)} style="margin-bottom:12px">
              + Add profile note
            </button>
          }
        >
          <div class="setting-row" style="margin-bottom:12px">
            <div class="form-row">
              <label>kind</label>
              <input value={addKind()} onInput={(e) => setAddKind(e.currentTarget.value)} />
            </div>
            <div class="form-row">
              <label>tag</label>
              <input value={addTag()} onInput={(e) => setAddTag(e.currentTarget.value)} placeholder="optional" />
            </div>
            <div class="form-row">
              <label>text</label>
              <textarea rows={2} value={addText()} onInput={(e) => setAddText(e.currentTarget.value)} />
            </div>
            <div class="row-gap">
              <button disabled={!addText().trim() || !addKind().trim()} onClick={() => void addNote()}>Append</button>
              <button class="ghost" onClick={() => setAdding(false)}>Cancel</button>
            </div>
          </div>
        </Show>
      </Show>

      <For each={[...tierNotes()].reverse()}>
        {(n) => (
          <div class="setting-row memory-note">
            <div class="note-meta">
              <span class="mono dim">{n.id.slice(0, 8)}</span>
              <span class="chip">{n.kind}</span>
              <Show when={n.tag}><span class="chip">{n.tag}</span></Show>
              <span class="when">{timeAgo(n.ts)}</span>
            </div>
            <Show
              when={editingId() === n.id}
              fallback={
                <>
                  <div class="note-text">{n.text}</div>
                  <div class="row-gap" style="margin-top:6px">
                    <button class="ghost small" onClick={() => { setEditingId(n.id); setEditText(n.text); }}>
                      Amend
                    </button>
                    <button class="danger small" onClick={() => void forgetNote(n.id)}>
                      Forget
                    </button>
                  </div>
                </>
              }
            >
              <textarea rows={3} value={editText()} onInput={(e) => setEditText(e.currentTarget.value)} />
              <div class="row-gap" style="margin-top:6px">
                <button class="small" onClick={() => void amendNote(n.id)}>Save</button>
                <button class="ghost small" onClick={() => setEditingId(null)}>Cancel</button>
              </div>
            </Show>
          </div>
        )}
      </For>
      <Show when={tierNotes().length === 0}>
        <div class="empty">No notes in this tier.</div>
      </Show>
    </div>
  );
}

// ---- Settings: Tasks --------------------------------------------------------

function SettingsTasks() {
  const [tasks, setTasks] = createSignal<TaskDef[]>([]);
  const [creating, setCreating] = createSignal(false);
  const [draft, setDraft] = createSignal({ name: "", prompt: "", interval_secs: 3600, schedule: "", model_pin: "" });

  const refresh = async () => {
    try {
      const r = await api.listTasks();
      setTasks(r.tasks ?? []);
    } catch {
      // ignore
    }
  };

  createEffect(() => { void refresh(); });

  const createTask = async () => {
    const d = draft();
    if (!d.name.trim() || !d.prompt.trim()) return;
    try {
      await api.createTask({
        name: d.name.trim(),
        prompt: d.prompt.trim(),
        interval_secs: d.interval_secs,
        schedule: d.schedule.trim() || null,
        model_pin: d.model_pin.trim() || null,
      });
      setCreating(false);
      setDraft({ name: "", prompt: "", interval_secs: 3600, schedule: "", model_pin: "" });
      pushToast("info", "Task created");
      void refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const toggleTask = async (t: TaskDef) => {
    try {
      await api.patchTask(t.id, { enabled: !t.enabled });
      void refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const runNow = async (id: string) => {
    try {
      await api.runTaskNow(id);
      pushToast("info", "Task triggered");
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const deleteTask = async (id: string) => {
    try {
      await api.deleteTask(id);
      pushToast("info", "Task deleted");
      void refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  return (
    <div>
      <h3>Scheduled tasks ({tasks().length})</h3>
      <Show
        when={tasks().length > 0}
        fallback={<div class="empty">No tasks configured.</div>}
      >
        <For each={tasks()}>
          {(t) => (
            <div class="setting-row task-row">
              <div class="setting-row-head">
                <label class="toggle">
                  <input
                    type="checkbox"
                    checked={t.enabled}
                    onChange={() => void toggleTask(t)}
                  />
                  {t.enabled ? "On" : "Off"}
                </label>
                <strong>{t.name}</strong>
                <Show when={t.model_pin}>
                  <span class="chip mono">{t.model_pin}</span>
                </Show>
                <span class="spacer" />
                <button class="ghost small" onClick={() => void runNow(t.id)}>Run now</button>
                <button class="danger small" onClick={() => void deleteTask(t.id)}>Delete</button>
              </div>
              <div class="dim" style="margin-top:4px">{t.prompt}</div>
              <div class="note-meta" style="margin-top:4px">
                <span>interval: {t.interval_secs}s</span>
                <Show when={t.schedule}><span class="chip">{t.schedule}</span></Show>
                <Show when={t.next_run}>
                  <span>next: {timeAgo(t.next_run!)}</span>
                </Show>
              </div>
            </div>
          )}
        </For>
      </Show>

      <Show
        when={creating()}
        fallback={
          <button class="ghost" onClick={() => setCreating(true)} style="margin-top:12px">
            + New task
          </button>
        }
      >
        <div class="setting-row" style="margin-top:12px">
          <div class="form-row">
            <label>name</label>
            <input value={draft().name} onInput={(e) => setDraft({ ...draft(), name: e.currentTarget.value })} />
          </div>
          <div class="form-row">
            <label>prompt</label>
            <textarea rows={2} value={draft().prompt} onInput={(e) => setDraft({ ...draft(), prompt: e.currentTarget.value })} />
          </div>
          <div class="two-col" style="gap:10px">
            <div class="form-row">
              <label>interval (secs)</label>
              <input
                type="number"
                min={60}
                value={draft().interval_secs}
                onInput={(e) => setDraft({ ...draft(), interval_secs: Number(e.currentTarget.value) })}
              />
            </div>
            <div class="form-row">
              <label>schedule (cron)</label>
              <input
                value={draft().schedule}
                onInput={(e) => setDraft({ ...draft(), schedule: e.currentTarget.value })}
                placeholder="optional"
              />
            </div>
          </div>
          <div class="form-row">
            <label>model pin</label>
            <input
              value={draft().model_pin}
              onInput={(e) => setDraft({ ...draft(), model_pin: e.currentTarget.value })}
              placeholder="optional — use specific model"
            />
          </div>
          <div class="row-gap">
            <button disabled={!draft().name.trim() || !draft().prompt.trim()} onClick={() => void createTask()}>
              Create
            </button>
            <button class="ghost" onClick={() => setCreating(false)}>Cancel</button>
          </div>
        </div>
      </Show>
    </div>
  );
}

// ---- Settings: Services -----------------------------------------------------

function SettingsServices() {
  const [diag, setDiag] = createSignal<OpsDiagnostics | null>(null);
  const [finops, setFinops] = createSignal<FinopsStatus | null>(null);
  const [doctor, setDoctor] = createSignal<DoctorReport | null>(null);
  const [ops, setOps] = createSignal<OpsStatusShape | null>(null);
  const [digest, setDigest] = createSignal<DigestReport | null>(null);
  const [digestDays, setDigestDays] = createSignal(7);
  const [rebuilding, setRebuilding] = createSignal(false);

  const refresh = async () => {
    try {
      const [d, f, doc, o] = await Promise.all([
        api.opsDiagnostics(),
        api.finopsStatus(),
        api.doctor(),
        api.opsStatus(),
      ]);
      setDiag(d);
      setFinops(f);
      setDoctor(doc);
      setOps(o);
    } catch {
      // partial
    }
  };

  const refreshDigest = async () => {
    try {
      setDigest(await api.digest(digestDays()));
    } catch {
      // ignore
    }
  };

  createEffect(() => { void refresh(); });
  createEffect(() => { void refreshDigest(); });

  const svcAction = async (svc: string, action: string) => {
    try {
      await api.opsAction(svc, action);
      pushToast("info", `${svc}: ${action}`);
      void refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
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

  const fmt = (n: number) => n.toLocaleString(undefined, { maximumFractionDigits: 2 });
  const fmtK = (n: number) => {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
    if (n >= 1_000) return (n / 1_000).toFixed(1) + "k";
    return String(n);
  };

  return (
    <div>
      <h3>Runtime health</h3>
      <Show when={diag()} fallback={<div class="empty">Loading diagnostics…</div>}>
        <dl class="kv">
          <dt>status</dt>
          <dd><span class="chip" data-on={diag()!.health.status === "ok"}>{diag()!.health.status}</span></dd>
          <dt>provider</dt><dd>{diag()!.health.provider}</dd>
          <dt>model</dt><dd class="mono">{diag()!.health.model}</dd>
          <dt>sandbox</dt><dd>{diag()!.health.sandbox}</dd>
          <dt>permission</dt><dd>{diag()!.health.permission_mode}</dd>
        </dl>
      </Show>

      <h3 style="margin-top:16px">Background services</h3>
      <Show when={ops()}>
        <div class="svc-grid">
          <div class="svc-row">
            <span>Gateway</span>
            <span class="chip" data-on={ops()!.gateway.state === "running"}>{ops()!.gateway.state}</span>
            <button class="ghost small" onClick={() => void svcAction("gateway", "restart")}>Restart</button>
          </div>
          <div class="svc-row">
            <span>Telegram</span>
            <span class="chip" data-on={ops()!.telegram.state === "running"}>{ops()!.telegram.state}</span>
            <div class="row-gap">
              <button class="ghost small" onClick={() => void svcAction("telegram", "start")}>Start</button>
              <button class="ghost small" onClick={() => void svcAction("telegram", "stop")}>Stop</button>
            </div>
          </div>
        </div>
      </Show>

      <h3 style="margin-top:16px">Spend &amp; budget</h3>
      <Show when={finops()}>
        <div class="digest-tiles">
          <div class="digest-tile">
            <div class="digest-value">${fmt(finops()!.day_usd)}</div>
            <div class="digest-label">Today</div>
          </div>
          <Show when={finops()!.day_cap_usd != null}>
            <div class="digest-tile">
              <div class="digest-value">${fmt(finops()!.day_cap_usd!)}</div>
              <div class="digest-label">Day cap</div>
            </div>
          </Show>
          <div class="digest-tile">
            <div class="digest-value">{finops()!.unknown_rows}</div>
            <div class="digest-label">Unpriced</div>
          </div>
        </div>
        <Show when={finops()!.by_provider.length > 0}>
          <table class="table" style="margin-top:10px">
            <thead><tr><th>provider</th><th>usd</th><th>calls</th></tr></thead>
            <tbody>
              <For each={finops()!.by_provider}>
                {(p) => <tr><td>{p.name}</td><td>${fmt(p.usd)}</td><td>{p.calls}</td></tr>}
              </For>
            </tbody>
          </table>
        </Show>
      </Show>

      <h3 style="margin-top:16px">Usage digest</h3>
      <div class="row-gap" style="margin-bottom:10px">
        <select value={digestDays()} onChange={(e) => { setDigestDays(Number(e.currentTarget.value)); }}>
          <option value={1}>1 day</option>
          <option value={7}>7 days</option>
          <option value={14}>14 days</option>
          <option value={30}>30 days</option>
        </select>
        <button class="ghost small" onClick={() => void refreshDigest()}>Refresh</button>
      </div>
      <Show when={digest()}>
        <div class="digest-tiles">
          <div class="digest-tile">
            <div class="digest-value">${fmt(digest()!.total_usd)}</div>
            <div class="digest-label">Total spend</div>
          </div>
          <div class="digest-tile">
            <div class="digest-value">{fmtK(digest()!.input_tokens + digest()!.output_tokens)}</div>
            <div class="digest-label">Tokens</div>
          </div>
          <div class="digest-tile">
            <div class="digest-value">{digest()!.dispatches}</div>
            <div class="digest-label">Dispatches</div>
          </div>
          <div class="digest-tile">
            <div class="digest-value">{digest()!.distinct_sessions.length}</div>
            <div class="digest-label">Sessions</div>
          </div>
          <div class="digest-tile">
            <div class="digest-value">{digest()!.memory_notes_appended}</div>
            <div class="digest-label">Memory notes</div>
          </div>
          <div class="digest-tile">
            <div class="digest-value">{digest()!.unpriced_rows}</div>
            <div class="digest-label" data-warn={digest()!.unpriced_rows > 0}>Unpriced</div>
          </div>
        </div>
      </Show>

      <h3 style="margin-top:16px">Doctor</h3>
      <Show when={doctor()}>
        <div class="row-gap" style="margin-bottom:8px">
          <span class="chip" data-on={doctor()!.failures === 0}>
            {doctor()!.failures === 0 ? "All checks pass" : `${doctor()!.failures} failing`}
          </span>
        </div>
        <For each={doctor()!.checks}>
          {(c) => (
            <div class="setting-row" style="padding:6px 0">
              <span class="chip" data-on={c.ok}>{c.ok ? "✓" : "✗"}</span>
              <span>{c.label}</span>
              <span class="dim">{c.detail}</span>
            </div>
          )}
        </For>
      </Show>

      <div class="row-gap" style="margin-top:16px">
        <button disabled={rebuilding()} onClick={() => void rebuild()}>
          {rebuilding() ? "Rebuilding…" : "Rebuild search index"}
        </button>
      </div>
    </div>
  );
}

// ---- Settings: Reliability --------------------------------------------------

function SettingsReliability() {
  const [config] = createResource(() => api.config());

  return (
    <div>
      <Show when={!config.loading} fallback={<div class="empty">Loading…</div>}>
        <h3>Request recovery</h3>
        <dl class="kv">
          <dt>context window</dt>
          <dd>{(config()?.context_window ?? 0).toLocaleString()} tokens</dd>
          <dt>max output</dt>
          <dd>{(config()?.max_tokens ?? 0).toLocaleString()} tokens</dd>
        </dl>

        <h3 style="margin-top:16px">Route ladder</h3>
        <dl class="kv">
          <dt>objective</dt>
          <dd>{config()?.route?.objective ?? "—"}</dd>
          <dt>cross-model fallbacks</dt>
          <dd>
            {config()?.route?.fallback_models?.length ?? 0} models
            <Show when={(config()?.route?.fallback_models?.length ?? 0) > 0}>
              <span class="dim"> — {config()!.route.fallback_models.join(", ")}</span>
            </Show>
          </dd>
          <dt>ladder length cap</dt>
          <dd>{config()?.route?.max_fallbacks ?? "—"}</dd>
        </dl>

        <h3 style="margin-top:16px">Integrations</h3>
        <dl class="kv">
          <dt>MCP servers</dt><dd>{config()?.integrations?.mcp_servers?.length ?? 0}</dd>
          <dt>hooks</dt><dd>{config()?.integrations?.hooks ?? 0}</dd>
          <dt>skills</dt><dd>{config()?.integrations?.skills?.length ?? 0}</dd>
        </dl>

        <Show when={(config()?.warnings?.length ?? 0) > 0}>
          <h3 style="margin-top:16px">Warnings</h3>
          <For each={config()!.warnings}>
            {(w) => <div class="warning">{w}</div>}
          </For>
        </Show>
      </Show>
    </div>
  );
}

// ---- Settings: Advanced -----------------------------------------------------

function SettingsAdvanced() {
  const [config] = createResource(() => api.config());
  const [backupDir, setBackupDir] = createSignal("");
  const [includeSecrets, setIncludeSecrets] = createSignal(false);
  const [backupBusy, setBackupBusy] = createSignal(false);
  const [importDir, setImportDir] = createSignal("");
  const [conflict, setConflict] = createSignal<"skip" | "rename">("skip");
  const [importBusy, setImportBusy] = createSignal(false);

  const fmt = (n: number) => n.toLocaleString();

  const runExport = async () => {
    if (!backupDir().trim()) return;
    setBackupBusy(true);
    try {
      const res = await api.backupExport(backupDir().trim(), includeSecrets());
      pushToast("info", `Exported ${res.manifest.file_count} files (${fmt(res.manifest.total_bytes)} bytes)`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBackupBusy(false);
    }
  };

  const runImport = async () => {
    if (!importDir().trim()) return;
    setImportBusy(true);
    try {
      const res = await api.backupImport(importDir().trim(), conflict());
      pushToast("info", `Imported: ${res.copied} copied, ${res.skipped} skipped`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setImportBusy(false);
    }
  };

  const copyToClipboard = (text: string) => {
    void navigator.clipboard.writeText(text);
    pushToast("info", "Copied to clipboard");
  };

  return (
    <div>
      <Show when={!config.loading} fallback={<div class="empty">Loading…</div>}>
        <h3>Paths</h3>
        <dl class="kv">
          <dt>project config</dt>
          <dd class="mono wrap">{config()?.paths?.project_config ?? "—"}</dd>
          <dt>global config</dt>
          <dd class="mono wrap">
            {config()?.paths?.global_config ?? "—"}
            <Show when={config()?.paths?.global_config}>
              <button class="ghost small" onClick={() => copyToClipboard(config()!.paths.global_config!)}>
                Copy
              </button>
            </Show>
          </dd>
          <dt>session store</dt>
          <dd class="mono wrap">
            {config()?.paths?.sessions_home ?? "—"}
            <Show when={config()?.paths?.sessions_home}>
              <button class="ghost small" onClick={() => copyToClipboard(config()!.paths.sessions_home)}>
                Copy
              </button>
            </Show>
          </dd>
          <dt>workspace</dt>
          <dd class="mono wrap">{config()?.paths?.cwd ?? "—"}</dd>
        </dl>

        <h3 style="margin-top:16px">Data &amp; backup</h3>
        <div class="backup-section">
          <div class="form-row">
            <label>Export destination</label>
            <input
              value={backupDir()}
              onInput={(e) => setBackupDir(e.currentTarget.value)}
              placeholder="/path/to/backup"
            />
          </div>
          <label class="toggle" style="margin:8px 0">
            <input
              type="checkbox"
              checked={includeSecrets()}
              onChange={(e) => setIncludeSecrets(e.currentTarget.checked)}
            />
            Include secrets (API keys)
          </label>
          <button disabled={backupBusy() || !backupDir().trim()} onClick={() => void runExport()}>
            {backupBusy() ? "Exporting…" : "Export now"}
          </button>
        </div>

        <div class="backup-section" style="margin-top:16px">
          <div class="form-row">
            <label>Import source</label>
            <input
              value={importDir()}
              onInput={(e) => setImportDir(e.currentTarget.value)}
              placeholder="/path/to/backup"
            />
          </div>
          <div class="form-row">
            <label>Conflict policy</label>
            <select value={conflict()} onChange={(e) => setConflict(e.currentTarget.value as "skip" | "rename")}>
              <option value="skip">Skip existing</option>
              <option value="rename">Rename incoming</option>
            </select>
          </div>
          <button disabled={importBusy() || !importDir().trim()} onClick={() => void runImport()}>
            {importBusy() ? "Importing…" : "Import"}
          </button>
        </div>

        <Show when={(config()?.warnings?.length ?? 0) > 0}>
          <h3 style="margin-top:16px">Configuration warnings</h3>
          <For each={config()!.warnings}>
            {(w) => <div class="warning">{w}</div>}
          </For>
        </Show>
      </Show>
    </div>
  );
}

// ---- Shell -----------------------------------------------------------------

const NAV = [
  { hash: "#/overview", label: "Overview", icon: ICONS.overview },
  { hash: "#/sessions", label: "Sessions", icon: ICONS.sessions },
  { hash: "#/search", label: "Search", icon: ICONS.search },
  { hash: "#/inbox", label: "Inbox", icon: ICONS.inbox, badge: () => unread().toString() || "" },
  { hash: "#/security", label: "Security", icon: ICONS.security },
  { hash: "#/tasks", label: "Tasks", icon: ICONS.tasks },
  { hash: "#/settings", label: "Settings", icon: ICONS.settings },
];

export default function App() {
  // One-click login: the tray (and bookmarked links) may open the console
  // as /admin#token=… — the fragment never leaves the machine, so trade it
  // for the session cookie and scrub it from the address bar immediately.
  createEffect(() => {
    const m = /^#token=(.+)$/.exec(location.hash);
    if (!m) return;
    history.replaceState(null, "", location.pathname + "#/overview");
    api
      .login(decodeURIComponent(m[1]))
      .then(() => {
        setAuthed(true);
        connectEvents();
        navigate("#/overview");
      })
      .catch(() => setAuthed(false));
  });

  // Probe auth once: any authenticated endpoint answering 200 means we're in.
  createEffect(() => {
    api.config()
      .then(() => {
        setAuthed(true);
        connectEvents();
      })
      .catch(() => setAuthed(false));
  });

  // Light unread-count poll while signed in.
  createEffect(() => {
    if (authed() !== true) return;
    let alive = true;
    const tick = async () => {
      try {
        const { count } = await api.unreadCount();
        if (alive) setUnread(count);
      } catch {
        // transient; next tick retries
      }
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
                    <Show when={"badge" in item && item.badge?.()}>
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
              <Match when={currentRoute() === "#/search"}><SearchView /></Match>
              <Match when={currentRoute() === "#/inbox"}><Inbox /></Match>
              <Match when={currentRoute() === "#/security"}><Security /></Match>
              <Match when={currentRoute() === "#/tasks"}><SettingsTasks /></Match>
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
