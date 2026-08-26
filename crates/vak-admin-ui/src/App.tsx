import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { api, AuthRequired } from "./api";
import { clock, shortId, timeAgo } from "./time";
import {
  approvalsVersion, authed, conn, connectEvents, disconnectEvents, feed, navigate, pushToast,
  route, setAuthed, toasts,
} from "./store";
import type { PendingApproval, SearchHit, SecurityEvent, SessionListItem, TranscriptEntry } from "./types";

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

function Settings() {
  const [config, { refetch }] = createResource(() => api.config());
  const [gateway] = createResource(() => api.gatewayStatus());
  const [rebuilding, setRebuilding] = createSignal(false);

  // Editable copies, seeded once the fetch lands.
  const [provider, setProvider] = createSignal("");
  const [model, setModel] = createSignal("");
  createEffect(() => {
    const c = config();
    if (c) {
      setProvider(c.provider);
      setModel(c.model);
    }
  });
  const dirty = createMemo(() => {
    const c = config();
    return !!c && (provider() !== c.provider || model() !== c.model);
  });

  const saveIdentity = async () => {
    try {
      await api.patchConfig({ provider: provider().trim(), model: model().trim() });
      pushToast("info", "Provider/model updated");
      refetch();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  const switchMode = async (mode: string) => {
    try {
      await api.setMode(mode);
      pushToast("info", `Permission mode → ${mode}`);
      refetch();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
      refetch(); // snap back to actual
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

  const signOut = async () => {
    await api.logout();
    disconnectEvents();
    setAuthed(false);
    navigate("#/overview");
  };

  return (
    <div class="view">
      <div class="two-col">
        <section class="panel">
          <h2>Model identity</h2>
          <Show when={!config.loading} fallback={<div class="empty">Loading…</div>}>
            <div class="form-row">
              <label>provider</label>
              <input value={provider()} onInput={(e) => setProvider(e.currentTarget.value)} />
            </div>
            <div class="form-row">
              <label>model</label>
              <input class="mono" value={model()} onInput={(e) => setModel(e.currentTarget.value)} />
            </div>
            <div class="row-gap">
              <button disabled={!dirty() || !provider().trim() || !model().trim()} onClick={saveIdentity}>
                Save changes
              </button>
              <Show when={dirty()}>
                <button class="ghost" onClick={() => { const c = config(); if (c) { setProvider(c.provider); setModel(c.model); } }}>
                  Reset
                </button>
              </Show>
            </div>
            <dl class="kv" style="margin-top:18px">
              <dt>max turns</dt><dd>{config()?.max_turns}</dd>
              <dt>theme</dt><dd>{config()?.theme}</dd>
            </dl>
          </Show>
        </section>

        <section class="panel">
          <h2>Permission mode</h2>
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
                    {m === "ReadOnly" && "Read-only tools; nothing is written"}
                    {m === "WorkspaceWrite" && "Writes confined to the workspace"}
                    {m === "FullAccess" && "Unsandboxed — explicit human trust"}
                  </span>
                </button>
              )}
            </For>
          </div>

          <h2 style="margin-top:22px">Index &amp; Gateway</h2>
          <Show when={!gateway.loading} fallback={<div class="empty">Loading…</div>}>
            <dl class="kv">
              <dt>gateway</dt>
              <dd>
                <span class="chip" data-on={gateway()?.enabled}>{gateway()?.enabled ? "enabled" : "disabled"}</span>
              </dd>
              <dt>bindings</dt>
              <dd>{gateway()?.bindings.length ? gateway()!.bindings.join(", ") : "none"}</dd>
              <dt>chat allowlist</dt>
              <dd class="wrap">{gateway()?.chat_allowlist.length ? gateway()!.chat_allowlist.join(", ") : "all chats permitted"}</dd>
            </dl>
          </Show>
          <div class="row-gap">
            <button disabled={rebuilding()} onClick={rebuild}>
              {rebuilding() ? "Rebuilding…" : "Rebuild search index"}
            </button>
            <span class="spacer" />
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
  { hash: "#/search", label: "Search", icon: ICONS.search },
  { hash: "#/inbox", label: "Inbox", icon: ICONS.inbox, badge: () => unread().toString() || "" },
  { hash: "#/security", label: "Security", icon: ICONS.security },
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
