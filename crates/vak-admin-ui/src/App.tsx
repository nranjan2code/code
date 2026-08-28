import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { api, AuthRequired } from "./api";
import { clock, shortId, timeAgo } from "./time";
import {
  approvalsVersion, authed, conn, connectEvents, disconnectEvents, feed, navigate, pushToast,
  route, sessionsVersion, setAuthed, statsVersion, toasts,
} from "./store";
import type {
  AllowlistEntry, BestOfNRun, ChatSurfaceStatus, ConfigInfo, CorePoolEntry, DiscoveredModelsResponse, FinOpsStatus, HookConfig,
  GatewayBinding, GatewayStatus, InboxEntry, McpServerConfig, MemoryItem, OpsStatus, PendingApproval,
  PermissionMode, ProviderSummary,
  SearchHit, SecurityEvent, SessionCheckpoint, SessionDiff, SessionListItem,
  SkillItem, SkillProposal, TaskItem, TranscriptEntry, WorkReceipt,
} from "./types";

// Chat surfaces with a bridge (crates/vak-server's InboundChannel impls) and
// a matching Core::bot_token_env entry. A routing key's surface prefix is
// validated against this list — the bare colon check it replaced accepted
// a pasted bot token (itself "digits:secret"-shaped) as a plausible key.
const KNOWN_SURFACES = ["telegram", "discord", "slack"] as const;

// Theme state: initialized from localStorage and synchronized to document root dataset
const [theme, setTheme] = createSignal<"warm" | "dark" | "contrast">(
  (localStorage.getItem("vak_admin_theme") as "warm" | "dark" | "contrast") || "warm"
);

createEffect(() => {
  const t = theme();
  localStorage.setItem("vak_admin_theme", t);
  document.documentElement.dataset.theme = t === "warm" ? "" : t;
});

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
  gateway: "M4 4h16v12H4z M8 20h8 M12 16v4 M8 8h.01 M12 8h4 M8 12h8",
  memory: "M4 19.5A2.5 2.5 0 0 1 6.5 17H20 M4 4.5A2.5 2.5 0 0 1 6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15z",
  search: "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35",
  inbox: "M22 12h-6l-2 3h-4l-2-3H2M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
  security: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z",
  settings: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
};

// ---- filesystem paths ------------------------------------------------------

/// Shorten a path by dropping WHOLE middle segments, never by breaking one.
/// Two workspaces differ in their tail (`…/Projects/vakcoder`), almost never
/// in the `/Users/<name>` prefix every one of them shares, so the head is
/// what gets spent and the tail is kept to the last possible character.
///
/// The budget is in characters rather than segments on purpose: a
/// segment-counted result can still overflow the column and get clipped by
/// CSS from the right, which throws away exactly the end that distinguishes
/// two workspaces. Fitting the budget here means the CSS ellipsis is only
/// ever a backstop for a single segment longer than the whole column.
export function truncatePath(path: string, budget = 38): string {
  if (path.length <= budget) return path;
  const absolute = path.startsWith("/");
  const segments = path.split("/").filter(Boolean);
  if (segments.length <= 2) return path;

  const head = `${absolute ? "/" : ""}${segments[0]}/…`;
  // Grow the tail one segment at a time while it still fits, always keeping
  // at least the final segment even when nothing fits.
  let tail = segments[segments.length - 1];
  for (let i = segments.length - 2; i >= 1; i--) {
    const candidate = `${segments[i]}/${tail}`;
    if (head.length + 1 + candidate.length > budget) break;
    tail = candidate;
  }
  return `${head}/${tail}`;
}

/// A filesystem path in a width-constrained cell. Never wraps: middle
/// segments are elided first, and anything still too wide for the column is
/// clipped with an ellipsis by CSS. The untruncated path is always on the
/// `title` and selectable, so nothing is actually lost.
function PathCell(props: { path: string; budget?: number }) {
  // The CSS cap is derived from the same budget the text was fitted to, so
  // the two can never disagree and clip a tail that JS had already made room
  // for. `ch` is the right unit here because `.path` is monospaced.
  const budget = () => props.budget ?? 38;
  return (
    <span class="path" title={props.path} style={{ "max-width": `${budget() + 1}ch` }}>
      {truncatePath(props.path, budget())}
    </span>
  );
}

// ---- permission rules ------------------------------------------------------

type RuleDecision = "allow" | "ask" | "deny";

interface ParsedRule {
  /** The rule exactly as it appears in config, for display. */
  raw: string;
  /** Tool the rule binds to, lowercased (`bash`, `mcp`, `write`, …). */
  tool: string;
  /** Argument glob, or null for a blanket rule covering the whole tool. */
  pattern: string | null;
  decision: RuleDecision;
}

/// Mirrors `vak_permission::rules::Rule::parse`. A leading `+`/`?`/`-` picks
/// allow/ask/deny; a bare rule allows. `Tool(pattern)` scopes to matching
/// arguments, `Tool` covers every call. Anything that does not parse is
/// surfaced rather than silently dropped — an unparseable rule in config is
/// exactly the thing an operator needs to see.
function parseRule(raw: string, listDecision: RuleDecision): ParsedRule | null {
  const input = raw.trim();
  if (!input) return null;
  const prefix = input[0];
  const decision: RuleDecision =
    prefix === "+" ? "allow" : prefix === "-" ? "deny" : prefix === "?" ? "ask" : listDecision;
  const rest = "+-?".includes(prefix) ? input.slice(1) : input;
  const open = rest.indexOf("(");
  const close = rest.lastIndexOf(")");
  const [tool, pattern] =
    open >= 0 && close > open
      ? [rest.slice(0, open).trim(), rest.slice(open + 1, close).trim()]
      : [rest.trim(), null];
  if (!tool) return null;
  return {
    raw: input,
    tool: tool.toLowerCase(),
    pattern: pattern === "*" || pattern === "" ? null : pattern,
    decision,
  };
}

/// True when the server did not report its rule lists at all. An older
/// binary simply omits the field, and "the server did not say" must never be
/// rendered as "there are no rules" — that is the difference between an
/// unknown scope and an unrestricted one.
function rulesReported(config: ConfigInfo | undefined): boolean {
  return !!config && config.permissions !== undefined;
}

function parseRuleLists(rules: import("./types").PermissionRules | undefined): ParsedRule[] {
  if (!rules) return [];
  return (
    [
      ["allow", rules.allow],
      ["ask", rules.ask],
      ["deny", rules.deny],
    ] as const
  ).flatMap(([decision, list]) =>
    (list ?? []).map((r) => parseRule(r, decision)).filter((r): r is ParsedRule => r !== null),
  );
}

/// Glob match for the subset the rule syntax uses on a single path segment:
/// `*` spans anything, `?` one character. Enough to answer "does this MCP
/// pattern name this server", which is all it is used for.
function globMatch(pattern: string, value: string): boolean {
  const rx = new RegExp(
    `^${pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".")}$`,
  );
  return rx.test(value);
}

/// Rules that govern calls into one MCP server. The engine matches
/// `mcp(server/tool)` patterns against the literal `server/tool` string, so a
/// rule reaches this server when it is blanket (`mcp`) or when the pattern's
/// server half names it.
function rulesForMcpServer(rules: ParsedRule[], server: string): ParsedRule[] {
  return rules.filter((r) => {
    if (r.tool !== "mcp") return false;
    if (r.pattern === null) return true;
    const serverPart = r.pattern.split("/")[0];
    return globMatch(serverPart, server);
  });
}

// Reuses the chip tones the gateway tables already established, so a red
// chip means the same thing on every screen. `danger` is the vocabulary --
// there is no `alert` tone, and naming one would have rendered every deny
// rule in muted grey.
const DECISION_TONE: Record<RuleDecision, string> = {
  allow: "success",
  ask: "warning",
  deny: "danger",
};

function RuleChip(props: { rule: ParsedRule }) {
  return (
    <span class={`chip chip-tone-${DECISION_TONE[props.rule.decision]} mono`} title={`${props.rule.decision}: ${props.rule.raw}`}>
      {props.rule.raw}
    </span>
  );
}

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
        <div class="login-logo"><img src="/admin/vak-icon.png" alt="" /></div>
        <h1>vak admin</h1>
        <p class="hint">Paste the token printed by <code>vak serve</code></p>
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
              <dd><PathCell path={health()?.cwd ?? ""} budget={46} /></dd>
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
                  <div class="dim" style="margin-bottom:8px"><PathCell path={run.repo} /></div>
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

// ---- Extensions (MCP servers, skills, hooks, scheduled tasks) ---------------

/// Everything that widens what the agent can do, and what each thing is
/// permitted to do once loaded. Four sub-routes, one per extension point,
/// so a destination is nameable from the nav rather than found by scrolling
/// — the same shape the Gateway section uses.
const EXTENSION_TABS = [
  { hash: "#/integrations", label: "MCP servers" },
  { hash: "#/integrations/skills", label: "Skills" },
  { hash: "#/integrations/hooks", label: "Hooks" },
  { hash: "#/integrations/tasks", label: "Scheduled tasks" },
] as const;

function extensionsTab(): string {
  const r = route();
  return EXTENSION_TABS.slice(1).find((t) => r.startsWith(t.hash))?.hash ?? "#/integrations";
}

interface ExtensionsCtx {
  mcp: () => Record<string, McpServerConfig>;
  mcpLoading: () => boolean;
  refetchMcp: () => void;
  hooks: () => HookConfig[];
  hooksLoading: () => boolean;
  refetchHooks: () => void;
  skills: () => SkillItem[];
  skillsLoading: () => boolean;
  proposals: () => SkillProposal[];
  proposalsLoading: () => boolean;
  refetchSkills: () => void;
  tasks: () => TaskItem[];
  tasksLoading: () => boolean;
  refetchTasks: () => void;
  /** Parsed allow/ask/deny rules from the running config. */
  rules: () => ParsedRule[];
  /** Effective permission mode, which decides everything no rule covers. */
  mode: () => string;
  /** Whether the server reported its rule lists at all — see rulesReported. */
  rulesKnown: () => boolean;
}

/// Stands in for the rule chips when the server never sent its rule lists.
/// Says so, rather than letting an empty list read as "unrestricted".
function RulesUnknown() {
  return (
    <span class="chip" title="This server build does not report its permission rules.">
      scope not reported
    </span>
  );
}

/// What happens to a tool call that no rule matches. Mirrors the mode arms
/// of `vak_permission::PermissionEngine::evaluate`: full-access allows,
/// read-only denies anything that is not a read tool, and workspace-write
/// asks. `mcp` and `task` are neither read nor write tools, so this is the
/// whole answer for them.
function modeDefaultDecision(mode: string): RuleDecision {
  if (mode === "FullAccess") return "allow";
  if (mode === "ReadOnly") return "deny";
  return "ask";
}

function ModeDefaultChip(props: { mode: string }) {
  const decision = () => modeDefaultDecision(props.mode);
  return (
    <span
      class={`chip chip-tone-${DECISION_TONE[decision()]}`}
      title={`No rule covers this call, so ${props.mode} mode decides: ${decision()}`}
    >
      {decision()} · mode default
    </span>
  );
}

// ---- Extensions › MCP servers ----------------------------------------------

function McpServersView(props: { ctx: ExtensionsCtx }) {
  const [expanded, setExpanded] = createSignal("");
  const [newName, setNewName] = createSignal("");
  const [newCmd, setNewCmd] = createSignal("");
  const [newArgs, setNewArgs] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  const servers = createMemo(() => Object.entries(props.ctx.mcp()) as [string, McpServerConfig][]);

  const addServer = async () => {
    const name = newName().trim();
    const cmd = newCmd().trim();
    if (!name || !cmd || busy()) return;
    setBusy(true);
    try {
      await api.putMcpServers({
        ...props.ctx.mcp(),
        [name]: { command: cmd, args: newArgs().trim() ? newArgs().trim().split(/\s+/) : [] },
      });
      pushToast("info", `Added MCP server ‘${name}’`);
      setNewName("");
      setNewCmd("");
      setNewArgs("");
      props.ctx.refetchMcp();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const removeServer = async (name: string) => {
    const next = { ...props.ctx.mcp() };
    delete next[name];
    try {
      await api.putMcpServers(next);
      pushToast("info", `Removed MCP server ‘${name}’`);
      props.ctx.refetchMcp();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  return (
    <>
      <div class="toolbar">
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refetchMcp()}>Refresh</button>
      </div>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>MCP servers</h2>
            <p class="dim">
              Each row is a process vak will start on demand to borrow its tools. What it may then
              be asked to do is decided by the <code>mcp(server/tool)</code> rules in{" "}
              <a href="#/settings">Permissions</a> and, where no rule reaches it, by the permission
              mode.
            </p>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.mcpLoading()}>
            <table class="table">
              <thead>
                <tr><th>server</th><th>command</th><th>network</th><th>secrets</th><th>governed by</th><th /></tr>
              </thead>
              <tbody><SkeletonRows cols={6} /></tbody>
            </table>
          </Match>

          <Match when={servers().length === 0}>
            <div class="empty empty-teach">
              <strong>No MCP servers are registered.</strong>
              <p>
                An MCP server is an external process that hands the agent extra tools — a GitHub
                client, a database, a browser. Register one below and it starts the first time a run
                actually calls it; until then it costs nothing.
              </p>
            </div>
          </Match>

          <Match when={servers().length > 0}>
            <table class="table">
              <thead>
                <tr><th>server</th><th>command</th><th>network</th><th>secrets</th><th>governed by</th><th /></tr>
              </thead>
              <tbody>
                <For each={servers()}>
                  {([name, server]) => {
                    const matching = createMemo(() => rulesForMcpServer(props.ctx.rules(), name));
                    const envKeys = () => Object.keys(server.env ?? {});
                    const open = () => expanded() === name;
                    return (
                      <>
                        <tr classList={{ "row-open": open() }} onClick={() => setExpanded(open() ? "" : name)}>
                          <td class="mono bold">{name}</td>
                          <td class="dim col-command">
                            <span class="path" title={`${server.command} ${(server.args ?? []).join(" ")}`}>
                              {server.command} {(server.args ?? []).join(" ")}
                            </span>
                          </td>
                          <td>
                            <span class={`chip chip-tone-${server.network ? "warning" : "success"}`}>
                              {server.network ? "outbound allowed" : "blocked"}
                            </span>
                          </td>
                          <td class="dim">
                            {envKeys().length > 0 ? `${envKeys().length} env var${envKeys().length === 1 ? "" : "s"}` : "—"}
                          </td>
                          <td>
                            <Switch fallback={<ModeDefaultChip mode={props.ctx.mode()} />}>
                              <Match when={!props.ctx.rulesKnown()}><RulesUnknown /></Match>
                              <Match when={matching().length > 0}>
                                <div class="chip-stack">
                                  <For each={matching()}>{(r) => <RuleChip rule={r} />}</For>
                                </div>
                              </Match>
                            </Switch>
                          </td>
                          <td><span class="chev">{open() ? "⌄" : "›"}</span></td>
                        </tr>
                        <Show when={open()}>
                          <tr class="row-detail">
                            <td colspan={6}>
                              <div class="detail-grid">
                                <div>
                                  <span class="eyebrow">Command line</span>
                                  <pre class="mono detail-pre">{server.command} {(server.args ?? []).join(" ")}</pre>
                                </div>
                                <div>
                                  <span class="eyebrow">Environment passed in</span>
                                  <Show
                                    when={envKeys().length > 0}
                                    fallback={<p class="dim">Nothing beyond vak's own environment.</p>}
                                  >
                                    <div class="chip-stack">
                                      <For each={envKeys()}>{(k) => <span class="chip mono">{k}</span>}</For>
                                    </div>
                                    <p class="dim">
                                      Names only. Values live in <code>.vak/config.toml</code> and are
                                      never rendered here.
                                    </p>
                                  </Show>
                                </div>
                                <div>
                                  <span class="eyebrow">Permission scope</span>
                                  <Show when={!props.ctx.rulesKnown()}>
                                    <p class="dim">
                                      This server build does not report its permission rules, so what
                                      governs calls to <code>{name}</code> cannot be shown here. Read{" "}
                                      <code>allow</code>/<code>ask</code>/<code>deny</code> in{" "}
                                      <code>config.toml</code> directly, or update vak.
                                    </p>
                                  </Show>
                                  <Show when={props.ctx.rulesKnown()}>
                                  <Show
                                    when={matching().length > 0}
                                    fallback={
                                      <p class="dim">
                                        No rule names this server, so every call to it takes the mode
                                        default: <strong>{modeDefaultDecision(props.ctx.mode())}</strong> under{" "}
                                        {props.ctx.mode()}. Add an <code>mcp({name}/*)</code> rule to
                                        config to narrow or widen that.
                                      </p>
                                    }
                                  >
                                    <div class="chip-stack">
                                      <For each={matching()}>{(r) => <RuleChip rule={r} />}</For>
                                    </div>
                                    <p class="dim">
                                      Among rules that match one call, deny outranks ask outranks
                                      allow — order in config never decides it. A call these patterns
                                      miss falls through to the mode default:{" "}
                                      <strong>{modeDefaultDecision(props.ctx.mode())}</strong> under {props.ctx.mode()}.
                                    </p>
                                  </Show>
                                  </Show>
                                </div>
                              </div>
                              <div class="row-gap" style="margin-top:12px">
                                <button class="danger small" onClick={() => removeServer(name)}>
                                  Remove server
                                </button>
                              </div>
                            </td>
                          </tr>
                        </Show>
                      </>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>

        <details class="advanced">
          <summary>Register an MCP server</summary>
          <p class="dim">
            Written straight to <code>mcp.servers</code> in the workspace config. Outbound network
            and injected environment are privileged fields and are only editable in the file.
          </p>
          <div class="form-row">
            <label>name</label>
            <input placeholder="github, filesystem, postgres…" value={newName()} onInput={(e) => setNewName(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>command</label>
            <input class="mono" placeholder="npx, uvx, python3…" value={newCmd()} onInput={(e) => setNewCmd(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>args</label>
            <input class="mono" placeholder="-y @modelcontextprotocol/server-github" value={newArgs()} onInput={(e) => setNewArgs(e.currentTarget.value)} />
          </div>
          <div class="row-gap" style="margin-top:12px">
            <button disabled={busy() || !newName().trim() || !newCmd().trim()} onClick={() => void addServer()}>
              {busy() ? "Registering…" : "Register server"}
            </button>
          </div>
        </details>
      </section>
    </>
  );
}

// ---- Extensions › Skills ---------------------------------------------------

function SkillsView(props: { ctx: ExtensionsCtx }) {
  const [busyId, setBusyId] = createSignal("");

  const act = async (id: string, promote: boolean) => {
    setBusyId(id);
    try {
      if (promote) await api.promoteProposal(id);
      else await api.rejectProposal(id);
      pushToast("info", promote ? "Promoted to an active skill" : "Proposal rejected");
      props.ctx.refetchSkills();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusyId("");
    }
  };

  return (
    <>
      <div class="toolbar">
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refetchSkills()}>Refresh</button>
      </div>

      <Show when={props.ctx.proposals().length > 0}>
        <section class="panel panel-alert" style="margin-bottom:14px">
          <div class="panel-title-row">
            <div>
              <h2>{props.ctx.proposals().length} skill{props.ctx.proposals().length === 1 ? "" : "s"} proposed</h2>
              <p class="dim">
                Written by the agent from its own runs. Nothing loads until you promote it.
              </p>
            </div>
          </div>
          <ul class="hit-list">
            <For each={props.ctx.proposals()}>
              {(p) => (
                <li class="inbox-item">
                  <div class="hit-meta"><strong class="mono">{p.name}</strong></div>
                  <div class="hit-snippet">{p.description}</div>
                  <div class="row-gap" style="margin-top:8px">
                    <button class="approve small" disabled={busyId() === p.id} onClick={() => void act(p.id, true)}>
                      Promote
                    </button>
                    <button class="danger small" disabled={busyId() === p.id} onClick={() => void act(p.id, false)}>
                      Reject
                    </button>
                  </div>
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Loaded skills</h2>
            <p class="dim">
              A skill is instructions, not capability: it tells the agent how to approach a job, and
              every tool it then reaches for is gated the same as any other call. What it changes is
              which files the agent is told to read — so where a skill comes from is the thing worth
              watching.
            </p>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.skillsLoading()}>
            <table class="table">
              <thead><tr><th>skill</th><th>scope</th><th>what it does</th><th>source</th></tr></thead>
              <tbody><SkeletonRows cols={4} /></tbody>
            </table>
          </Match>

          <Match when={props.ctx.skills().length === 0}>
            <div class="empty empty-teach">
              <strong>No skills are loaded.</strong>
              <p>
                Skills are discovered from <code>.vak/skills/</code> in this workspace and from the
                user-wide skills directory. Each is a folder with a <code>SKILL.md</code> inside.
              </p>
            </div>
          </Match>

          <Match when={props.ctx.skills().length > 0}>
            <table class="table">
              <thead><tr><th>skill</th><th>scope</th><th>what it does</th><th>source</th></tr></thead>
              <tbody>
                <For each={props.ctx.skills()}>
                  {(s) => (
                    <tr class="row-static">
                      <td class="mono bold">{s.name}</td>
                      <td>
                        <span class={`chip ${s.scope === "workspace" ? "chip-tool" : "chip-mode"}`}>
                          {s.scope ?? "user"}
                        </span>
                      </td>
                      <td class="dim">{s.description || "No description in its frontmatter."}</td>
                      <td class="dim col-path">
                        <Show when={s.path} fallback={<span class="dim">—</span>}>
                          <PathCell path={s.path!} />
                        </Show>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>
      </section>
    </>
  );
}

// ---- Extensions › Hooks ----------------------------------------------------

const HOOK_EVENTS = ["pre_tool_use", "post_tool_use", "session_start", "stop"] as const;

function HooksView(props: { ctx: ExtensionsCtx }) {
  const [event, setEvent] = createSignal<string>("pre_tool_use");
  const [matcher, setMatcher] = createSignal("");
  const [command, setCommand] = createSignal("");
  const [timeout, setTimeoutMs] = createSignal("10000");
  const [busy, setBusy] = createSignal(false);

  const addHook = async () => {
    const cmd = command().trim();
    if (!cmd || busy()) return;
    setBusy(true);
    try {
      await api.putHooks([
        ...props.ctx.hooks(),
        {
          event: event(),
          matcher: matcher().trim() || null,
          command: cmd,
          timeout_ms: parseInt(timeout(), 10) || 10_000,
          enabled: true,
        },
      ]);
      pushToast("info", `Added a ${event()} hook`);
      setCommand("");
      setMatcher("");
      props.ctx.refetchHooks();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const removeHook = async (index: number) => {
    const next = [...props.ctx.hooks()];
    next.splice(index, 1);
    try {
      await api.putHooks(next);
      pushToast("info", "Hook removed");
      props.ctx.refetchHooks();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  return (
    <>
      <div class="toolbar">
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refetchHooks()}>Refresh</button>
      </div>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Lifecycle hooks</h2>
            <p class="dim">
              Hooks are the one extension that governs rather than being governed: a{" "}
              <code>pre_tool_use</code> hook can block a call outright, and every hook runs as the
              vak process itself — unsandboxed, outside the permission engine. Its matcher is the
              only thing narrowing when it fires.
            </p>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.hooksLoading()}>
            <table class="table">
              <thead><tr><th>event</th><th>fires on</th><th>command</th><th>timeout</th><th /></tr></thead>
              <tbody><SkeletonRows cols={5} /></tbody>
            </table>
          </Match>

          <Match when={props.ctx.hooks().length === 0}>
            <div class="empty empty-teach">
              <strong>No hooks are configured.</strong>
              <p>
                Nothing intercepts a run. Add one to audit tool calls, block a pattern before it
                executes, or seed each session with workspace context.
              </p>
            </div>
          </Match>

          <Match when={props.ctx.hooks().length > 0}>
            <table class="table">
              <thead><tr><th>event</th><th>fires on</th><th>command</th><th>timeout</th><th /></tr></thead>
              <tbody>
                <For each={props.ctx.hooks()}>
                  {(hook, index) => {
                    const scope = createMemo(() =>
                      hook.matcher ? parseRule(hook.matcher, "allow") : null,
                    );
                    return (
                      <tr class="row-static">
                        <td><span class="chip chip-mode mono">{hook.event}</span></td>
                        <td>
                          <Switch>
                            <Match when={!hook.matcher}>
                              <span class="chip chip-tone-warning">every call</span>
                            </Match>
                            {/* The matcher prints verbatim: it is the string
                                in config, and its glob half is
                                case-significant. The parse only decides
                                whether the engine will accept it at all. */}
                            <Match when={scope()}>
                              <span class="chip mono">{hook.matcher}</span>
                            </Match>
                            <Match when={!scope()}>
                              <span class="chip chip-tone-danger mono" title="This matcher does not parse as a rule; the hook will not load.">
                                {hook.matcher} · invalid
                              </span>
                            </Match>
                          </Switch>
                        </td>
                        <td class="dim col-command">
                          <span class="path" title={hook.command}>{hook.command}</span>
                        </td>
                        <td class="dim">{hook.timeout_ms}ms</td>
                        <td>
                          <button class="danger small" onClick={() => void removeHook(index())}>
                            Remove
                          </button>
                        </td>
                      </tr>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>

        <details class="advanced">
          <summary>Add a hook</summary>
          <p class="dim">
            The matcher uses the same syntax as a permission rule — <code>Bash(git *)</code>,{" "}
            <code>Write</code>, <code>mcp(github/*)</code>. Leave it empty and the hook fires on
            every call for its event.
          </p>
          <div class="form-row">
            <label>event</label>
            <select value={event()} onChange={(e) => setEvent(e.currentTarget.value)}>
              <For each={HOOK_EVENTS}>{(ev) => <option value={ev}>{ev}</option>}</For>
            </select>
          </div>
          <div class="form-row">
            <label>matcher</label>
            <input class="mono" placeholder="Bash(git *) — optional" value={matcher()} onInput={(e) => setMatcher(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>command</label>
            <input class="mono" placeholder="/path/to/script.sh" value={command()} onInput={(e) => setCommand(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>timeout (ms)</label>
            <input type="number" min="100" value={timeout()} onInput={(e) => setTimeoutMs(e.currentTarget.value)} />
          </div>
          <div class="row-gap" style="margin-top:12px">
            <button disabled={busy() || !command().trim()} onClick={() => void addHook()}>
              {busy() ? "Adding…" : "Add hook"}
            </button>
          </div>
        </details>
      </section>
    </>
  );
}

// ---- Extensions › Scheduled tasks ------------------------------------------

function TasksView(props: { ctx: ExtensionsCtx }) {
  const [name, setName] = createSignal("");
  const [prompt, setPrompt] = createSignal("");
  const [schedule, setSchedule] = createSignal("");
  const [modelPin, setModelPin] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [busyId, setBusyId] = createSignal("");

  const guard = async (id: string, work: () => Promise<void>, ok: string) => {
    setBusyId(id);
    try {
      await work();
      pushToast("info", ok);
      props.ctx.refetchTasks();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusyId("");
    }
  };

  const createTask = async () => {
    if (!name().trim() || !prompt().trim() || busy()) return;
    setBusy(true);
    try {
      await api.createTask({
        name: name().trim(),
        prompt: prompt().trim(),
        schedule: schedule().trim() || undefined,
        model_pin: modelPin().trim() || undefined,
      });
      pushToast("info", `Scheduled ‘${name().trim()}’`);
      setName("");
      setPrompt("");
      setSchedule("");
      setModelPin("");
      props.ctx.refetchTasks();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <div class="toolbar">
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refetchTasks()}>Refresh</button>
      </div>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Scheduled tasks</h2>
            <p class="dim">
              Runs vak starts on its own, with nobody watching. Each one runs under the same
              permission mode as any other turn — <strong>{props.ctx.mode()}</strong> — so a task
              that needs an approval simply waits for one.
            </p>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.tasksLoading()}>
            <table class="table">
              <thead><tr><th>task</th><th>kind</th><th>schedule</th><th>model</th><th>last run</th><th>state</th><th /></tr></thead>
              <tbody><SkeletonRows cols={7} /></tbody>
            </table>
          </Match>

          <Match when={props.ctx.tasks().length === 0}>
            <div class="empty empty-teach">
              <strong>Nothing is scheduled.</strong>
              <p>
                A scheduled task is a prompt vak runs on a cron expression or a fixed interval — a
                nightly digest, a recurring health check — and files the result in your Inbox.
              </p>
            </div>
          </Match>

          <Match when={props.ctx.tasks().length > 0}>
            <table class="table">
              <thead><tr><th>task</th><th>kind</th><th>schedule</th><th>model</th><th>last run</th><th>state</th><th /></tr></thead>
              <tbody>
                <For each={props.ctx.tasks()}>
                  {(t) => (
                    <tr class="row-static">
                      <td class="bold">{t.name}</td>
                      <td><span class={`chip ${t.script ? "chip-tool" : "chip-mode"}`}>{t.script ? "script" : "prompt"}</span></td>
                      <td class="mono dim">{t.schedule ?? `every ${t.interval_secs ?? 3600}s`}</td>
                      <td class="mono dim">{t.model_pin ?? "workspace default"}</td>
                      <td class="dim" title={t.last_run_at ?? ""}>{t.last_run_at ? timeAgo(t.last_run_at) : "never"}</td>
                      <td>
                        <span class={t.enabled ? "chip chip-tone-success" : "chip"}>
                          {t.enabled ? "enabled" : "paused"}
                        </span>
                      </td>
                      <td>
                        <div class="row-gap">
                          <button
                            class="ghost small"
                            disabled={busyId() === t.id}
                            onClick={() => void guard(t.id, () => api.runTaskNow(t.id), `Ran ‘${t.name}’`)}
                          >
                            Run now
                          </button>
                          <button
                            class="ghost small"
                            disabled={busyId() === t.id}
                            onClick={() =>
                              void guard(
                                t.id,
                                () => api.patchTask(t.id, { enabled: !t.enabled }),
                                t.enabled ? "Task paused" : "Task enabled",
                              )
                            }
                          >
                            {t.enabled ? "Pause" : "Enable"}
                          </button>
                          <button
                            class="danger small"
                            disabled={busyId() === t.id}
                            onClick={() => void guard(t.id, () => api.deleteTask(t.id), "Task deleted")}
                          >
                            Delete
                          </button>
                        </div>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>

        <details class="advanced">
          <summary>Schedule a task</summary>
          <div class="form-row">
            <label>name</label>
            <input placeholder="nightly-digest, dependency-audit…" value={name()} onInput={(e) => setName(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>prompt</label>
            <textarea rows={3} placeholder="What should vak do each time this runs?" value={prompt()} onInput={(e) => setPrompt(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>schedule</label>
            <input class="mono" placeholder="*/30 * * * * — empty means hourly" value={schedule()} onInput={(e) => setSchedule(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>model pin</label>
            <input class="mono" placeholder="Optional — otherwise the workspace default" value={modelPin()} onInput={(e) => setModelPin(e.currentTarget.value)} />
          </div>
          <div class="row-gap" style="margin-top:12px">
            <button disabled={busy() || !name().trim() || !prompt().trim()} onClick={() => void createTask()}>
              {busy() ? "Scheduling…" : "Schedule task"}
            </button>
          </div>
        </details>
      </section>
    </>
  );
}

// ---- Extensions section shell ----------------------------------------------

function ExtensionsSection() {
  const [config] = createResource(() => api.config());
  const [mcp, mcpActions] = createResource(() => api.mcpServers());
  const [hooks, hooksActions] = createResource(() => api.hooks());
  const [skills, skillsActions] = createResource(() => api.skills());
  const [proposals, proposalsActions] = createResource(() => api.skillProposals());
  const [tasks, tasksActions] = createResource(() => api.tasks());

  const ctx: ExtensionsCtx = {
    mcp: () => mcp()?.servers ?? {},
    mcpLoading: () => mcp.loading,
    refetchMcp: () => void mcpActions.refetch(),
    hooks: () => hooks()?.hooks ?? [],
    hooksLoading: () => hooks.loading,
    refetchHooks: () => void hooksActions.refetch(),
    skills: () => skills()?.skills ?? [],
    skillsLoading: () => skills.loading,
    proposals: () => proposals()?.proposals ?? [],
    proposalsLoading: () => proposals.loading,
    refetchSkills: () => {
      void skillsActions.refetch();
      void proposalsActions.refetch();
    },
    tasks: () => tasks()?.tasks ?? [],
    tasksLoading: () => tasks.loading,
    refetchTasks: () => void tasksActions.refetch(),
    rules: createMemo(() => parseRuleLists(config()?.permissions)),
    mode: () => config()?.permission_mode ?? "WorkspaceWrite",
    rulesKnown: () => rulesReported(config()),
  };

  const count = (n: number, loading: boolean) => (loading ? "" : ` ${n}`);

  // An errored fetch and a genuinely empty extension point look identical
  // once the resource settles, and "nothing is configured" is the more
  // alarming of the two to report wrongly. Name the failure instead.
  const failure = createMemo(() => {
    for (const [what, res] of [
      ["configuration", config],
      ["MCP servers", mcp],
      ["hooks", hooks],
      ["skills", skills],
      ["skill proposals", proposals],
      ["scheduled tasks", tasks],
    ] as const) {
      if (res.error) return { what, error: `${res.error}` };
    }
    return null;
  });

  createEffect(() => {
    if (failure() && (config.error instanceof AuthRequired || mcp.error instanceof AuthRequired)) {
      setAuthed(false);
    }
  });

  return (
    <div class="view">
      <Show when={failure()}>
        <section class="panel panel-alert callout" style="margin-bottom:14px">
          <div>
            <strong>Could not read {failure()!.what}</strong>
            <p class="dim">
              {failure()!.error} — what is shown below may be incomplete, so treat an empty list as
              unknown rather than as nothing configured.
            </p>
          </div>
        </section>
      </Show>

      <div class="tab-bar">
        <For each={EXTENSION_TABS}>
          {(t) => (
            <a class="tab-btn" href={t.hash} classList={{ active: extensionsTab() === t.hash }}>
              {t.label}
              <Switch>
                <Match when={t.hash === "#/integrations"}>
                  <span class="tab-count">{count(Object.keys(ctx.mcp()).length, ctx.mcpLoading())}</span>
                </Match>
                <Match when={t.hash === "#/integrations/skills"}>
                  <span class="tab-count">{count(ctx.skills().length, ctx.skillsLoading())}</span>
                  <Show when={ctx.proposals().length > 0}>
                    <span class="nav-badge">{ctx.proposals().length}</span>
                  </Show>
                </Match>
                <Match when={t.hash === "#/integrations/hooks"}>
                  <span class="tab-count">{count(ctx.hooks().length, ctx.hooksLoading())}</span>
                </Match>
                <Match when={t.hash === "#/integrations/tasks"}>
                  <span class="tab-count">{count(ctx.tasks().length, ctx.tasksLoading())}</span>
                </Match>
              </Switch>
            </a>
          )}
        </For>
      </div>

      <Switch>
        <Match when={extensionsTab() === "#/integrations"}><McpServersView ctx={ctx} /></Match>
        <Match when={extensionsTab() === "#/integrations/skills"}><SkillsView ctx={ctx} /></Match>
        <Match when={extensionsTab() === "#/integrations/hooks"}><HooksView ctx={ctx} /></Match>
        <Match when={extensionsTab() === "#/integrations/tasks"}><TasksView ctx={ctx} /></Match>
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

const SEC_KINDS = ["", "auth_failure", "rate_limit", "chat_allowlist", "chat_pending", "chat_approved", "chat_denied", "chat_revoked", "permission_denial", "config_change", "provider_key_change", "full_access_grant", "full_access_revoke"];

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

// The tone names here have to be the ones `.chip-tone-*` actually defines
// (warning / success / danger / info). `warn` and `alert` matched no rule, so
// a denied approval and a budget alert — the two entries most worth catching
// the eye — were rendering in the same muted grey as a heartbeat.
const INBOX_KIND_TONE: Record<string, string> = {
  task_summary: "",
  approval_pending: "warning",
  approval_denied: "danger",
  budget_alert: "danger",
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

const ALLOWLIST_CHIP_TONE: Record<string, string> = {
  allowed: "success",
  pending: "warning",
  denied: "danger",
};

// Per-surface badge so a mixed Telegram+Discord+Slack deployment reads at
// a glance (docs/design/34 Phase 3). Everything else in these panels is
// already surface-agnostic — it keys off `surface:chat` alone.
const SURFACE_LABEL: Record<string, string> = {
  telegram: "Telegram",
  discord: "Discord",
  slack: "Slack",
};

function SurfaceBadge(props: { channelKey: string }) {
  const surface = () => props.channelKey.split(":")[0] ?? "";
  return (
    <span class="chip chip-surface" data-surface={surface()} title={`${surface()} channel`}>
      {SURFACE_LABEL[surface()] ?? surface()}
    </span>
  );
}

const CUSTOM_WORKSPACE = "__custom__";

/// Workspace picker: known workspaces first (paths vak has actually run
/// in), with a custom-path fallback that says plainly it is unverified
/// until a Core starts there — docs/design/34 open question 4. A typo'd
/// path is what put a channel in the wrong workspace in the first place.
function WorkspacePicker(props: {
  value: string;
  onChange: (value: string) => void;
  known: string[];
  corePool: CorePoolEntry[];
}) {
  const options = createMemo(() => {
    const seen = props.known.filter(Boolean);
    // A value already saved on the entry stays selectable even when it is
    // not (or no longer) in the known list.
    if (props.value && !seen.includes(props.value)) return [props.value, ...seen];
    return seen;
  });
  const isCustom = () => !!props.value && !options().includes(props.value);
  const [custom, setCustom] = createSignal(isCustom());
  const warm = createMemo(() => props.corePool.some((e) => e.workspace === props.value.trim()));

  return (
    <div style="margin-bottom:8px">
      <label class="inherit-toggle" style="margin-top:2px">
        Workspace
        <span class="chip" data-on={warm()} style="margin-left:6px">
          {warm() ? "warm" : "cold — starts on next message"}
        </span>
      </label>
      <select
        class="mono"
        style="width:100%"
        value={custom() ? CUSTOM_WORKSPACE : props.value}
        onChange={(e) => {
          const next = e.currentTarget.value;
          if (next === CUSTOM_WORKSPACE) {
            setCustom(true);
            return;
          }
          setCustom(false);
          props.onChange(next);
        }}
      >
        <For each={options()}>{(w) => <option value={w}>{w}</option>}</For>
        <option value={CUSTOM_WORKSPACE}>Custom path…</option>
      </select>
      <Show when={custom()}>
        <input
          class="mono"
          value={props.value}
          onInput={(e) => props.onChange(e.currentTarget.value)}
          placeholder="/absolute/path/to/workspace"
          style="width:100%;margin-top:6px"
        />
        <div class="binding-meta">
          Unverified: nothing has run here yet, so this path is only checked when the pool
          actually starts a Core in it. A typo surfaces as a <code>vak doctor</code> failure.
        </div>
      </Show>
    </div>
  );
}

function GatewayBindingEditor(props: {
  binding: GatewayBinding;
  providers: ProviderSummary[];
  allowlistEntry: AllowlistEntry | undefined;
  knownWorkspaces: string[];
  corePool: CorePoolEntry[];
  refresh: () => void;
}) {
  const [editing, setEditing] = createSignal(false);
  const [inherit, setInherit] = createSignal(!props.binding.override);
  const [provider, setProvider] = createSignal(
    props.binding.override?.provider ?? props.binding.effective_route.provider,
  );
  const [model, setModel] = createSignal(
    props.binding.override?.model ?? props.binding.effective_route.model,
  );
  const [models, setModels] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);

  createEffect(async () => {
    if (inherit()) return;
    try {
      const found = (await api.models(provider())).models ?? [];
      setModels(found);
      if (found.length && !found.includes(model())) setModel(found[0]);
    } catch {
      setModels([]);
    }
  });

  const act = async (operation: () => Promise<void>, message: string) => {
    setBusy(true);
    try {
      await operation();
      pushToast("info", message);
      props.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <article class="binding-card" classList={{ "binding-stale": props.binding.stale }}>
      <div class="binding-head">
        <div>
          <strong class="mono">{props.binding.target}</strong>
          <div class="binding-meta">
            {props.binding.session_id ? `session ${shortId(props.binding.session_id)}` : "new session on next message"}
          </div>
        </div>
        <div class="row-gap" style="gap:6px">
          <SurfaceBadge channelKey={props.binding.target} />
          <Show when={props.allowlistEntry}>
            <span class={`chip chip-tone-${ALLOWLIST_CHIP_TONE[props.allowlistEntry!.status] ?? ""}`}>
              {props.allowlistEntry!.status}
            </span>
          </Show>
          <span class={`chip ${props.binding.stale ? "chip-tone-warning" : "chip-tone-success"}`}>
            {props.binding.stale ? "rotation required" : "current"}
          </span>
        </div>
      </div>

      <div class="route-compare">
        <div>
          <span class="eyebrow">Effective route</span>
          <strong>{props.binding.effective_route.provider}</strong>
          <code>{props.binding.effective_route.model}</code>
          <span class="binding-meta">{props.binding.effective_route.source}</span>
        </div>
        {/* The permission counterpart of "Effective route": what this
            channel actually runs as, and where that came from. A pin that
            the workspace's own mode capped is called out rather than shown
            as if the wider grant were live. */}
        <Show when={props.allowlistEntry?.effective_permission_mode}>
          <div>
            <span class="eyebrow">Effective permission</span>
            <strong>{props.allowlistEntry!.effective_permission_mode}</strong>
            <span class="binding-meta">
              {props.allowlistEntry!.permission_capped
                ? `channel pin ${props.allowlistEntry!.permission_mode} capped by workspace`
                : props.allowlistEntry!.permission_mode
                  ? "channel override"
                  : "inherited from workspace"}
            </span>
          </div>
        </Show>
        <div>
          <span class="eyebrow">Frozen session</span>
          <Show when={props.binding.session_contract} fallback={<span class="dim">Not created</span>}>
            <strong>{props.binding.session_contract!.provider}</strong>
            <code>{props.binding.session_contract!.model}</code>
            <span class="binding-meta">v{props.binding.session_contract!.app_version}</span>
          </Show>
        </div>
      </div>

      <Show when={props.binding.stale_reasons.length}>
        <div class="warning">Will rotate on next inbound message: {props.binding.stale_reasons.join(", ").replaceAll("_", " ")}</div>
      </Show>

      <label class="inherit-toggle">
        <input type="checkbox" checked={inherit()} onChange={(e) => setInherit(e.currentTarget.checked)} />
        Inherit workspace default
      </label>
      <Show when={!inherit()}>
        <div class="binding-controls">
          <select value={provider()} onChange={(e) => setProvider(e.currentTarget.value)}>
            <For each={props.providers}>{(p) => <option value={p.name}>{p.name}</option>}</For>
          </select>
          <Show when={models().length} fallback={
            <input class="mono" value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="model id" />
          }>
            <select class="mono" value={model()} onChange={(e) => setModel(e.currentTarget.value)}>
              <For each={models()}>{(m) => <option value={m}>{m}</option>}</For>
            </select>
          </Show>
        </div>
      </Show>
      <div class="row-gap">
        <button disabled={busy() || (!inherit() && !model().trim())} onClick={() => act(
          () => api.patchGatewayBinding(props.binding.target, inherit() ? {} : { provider: provider(), model: model() }),
          `Route updated for ${props.binding.target}`,
        )}>Save route</button>
        <button class="ghost" disabled={busy() || !props.binding.session_id} onClick={() => act(
          () => api.rotateGatewayBinding(props.binding.target),
          `Conversation rotated for ${props.binding.target}`,
        )}>Rotate now</button>
        <span class="spacer" />
        <Show when={props.allowlistEntry?.status === "allowed"}>
          <button class="ghost small" disabled={busy()} onClick={() => setEditing((v) => !v)}>
            {editing() ? "Close editor" : "Edit access"}
          </button>
          <button class="danger small" disabled={busy()} onClick={() => {
            if (window.confirm(`Revoke allowlist access for ${props.binding.target}? The next message from this chat will be rejected and start a fresh pending review.`)) {
              void act(() => api.revokeGatewayAllowlist(props.binding.target), `Access revoked: ${props.binding.target}`);
            }
          }}>Revoke access</button>
        </Show>
        <button class="danger small" disabled={busy()} onClick={() => {
          if (window.confirm(`Remove ${props.binding.target} and its route override? Session history is preserved.`)) {
            void act(() => api.deleteGatewayBinding(props.binding.target), `Binding removed: ${props.binding.target}`);
          }
        }}>Remove binding</button>
      </div>
      <Show when={editing() && props.allowlistEntry?.status === "allowed"}>
        <ChannelAccessEditor
          entry={props.allowlistEntry!}
          providers={props.providers}
          knownWorkspaces={props.knownWorkspaces}
          corePool={props.corePool}
          refresh={() => {
            setEditing(false);
            props.refresh();
          }}
        />
      </Show>
    </article>
  );
}

/// The three permission modes in wire (kebab-case) form, ordered least to
/// most permissive — the same order and the same `mode-btn` control the
/// Settings page's "Permission Mode" panel uses, so an operator sees one
/// vocabulary in both places.
const CHANNEL_MODES: { value: PermissionMode; label: string; desc: string }[] = [
  { value: "read-only", label: "ReadOnly", desc: "Read-only tools; writes denied" },
  { value: "workspace-write", label: "WorkspaceWrite", desc: "Writes confined to workspace" },
  { value: "full-access", label: "FullAccess", desc: "Unsandboxed — explicit trust" },
];

/// Optional per-channel permission pin, parallel to the "Pin a specific
/// provider / model" checkbox. Unchecked means inherit the workspace's own
/// configured mode, which is the default and today's behavior.
function ChannelPermissionPicker(props: {
  pinned: boolean;
  setPinned: (v: boolean) => void;
  mode: PermissionMode;
  setMode: (m: PermissionMode) => void;
  /// The workspace's own mode, when known — the ceiling a pin is capped to.
  ceiling?: PermissionMode | null;
}) {
  // A pin above the workspace's own mode is not an error, it is simply
  // reduced at dispatch; say so plainly rather than letting the operator
  // believe they granted access the workspace never permits.
  const capped = () => {
    const c = props.ceiling;
    if (!props.pinned || !c) return null;
    const rank = (m: PermissionMode) => CHANNEL_MODES.findIndex((x) => x.value === m);
    return rank(props.mode) > rank(c) ? c : null;
  };
  return (
    <>
      <label class="inherit-toggle">
        <input
          type="checkbox"
          checked={props.pinned}
          onChange={(e) => props.setPinned(e.currentTarget.checked)}
        />
        Pin a specific permission mode (otherwise inherits the workspace default)
      </label>
      <Show when={props.pinned}>
        <div class="mode-grid">
          <For each={CHANNEL_MODES}>
            {(m) => (
              <button
                class="mode-btn"
                classList={{ active: props.mode === m.value }}
                onClick={() => props.setMode(m.value)}
              >
                <span class="mode-name">{m.label}</span>
                <span class="mode-desc">{m.desc}</span>
              </button>
            )}
          </For>
        </div>
        <Show when={capped()}>
          {(c) => (
            <div class="binding-meta">
              This workspace is configured for <code>{c()}</code>. A channel can only match or
              reduce that, never exceed it — this pin will take effect as <code>{c()}</code>.
            </div>
          )}
        </Show>
      </Show>
    </>
  );
}

/// Re-point an already-allowed channel's workspace/route in place
/// (`PATCH .../allowlist/{key}`) instead of revoke-and-re-approve, which
/// would lose the added_at/added_by provenance and 403 the channel in
/// between. The same form the approve flow uses, pre-filled.
function ChannelAccessEditor(props: {
  entry: AllowlistEntry;
  providers: ProviderSummary[];
  knownWorkspaces: string[];
  corePool: CorePoolEntry[];
  refresh: () => void;
}) {
  const [workspace, setWorkspace] = createSignal(props.entry.workspace ?? "");
  const [pinRoute, setPinRoute] = createSignal(!!props.entry.route);
  const [provider, setProvider] = createSignal(
    props.entry.route?.provider ?? props.providers[0]?.name ?? "",
  );
  const [model, setModel] = createSignal(props.entry.route?.model ?? "");
  const [models, setModels] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [pinPerm, setPinPerm] = createSignal(!!props.entry.permission_mode);
  const [perm, setPerm] = createSignal<PermissionMode>(
    props.entry.permission_mode ?? props.entry.workspace_permission_mode ?? "workspace-write",
  );

  createEffect(async () => {
    if (!pinRoute() || !provider()) return;
    try {
      const found = (await api.models(provider())).models ?? [];
      setModels(found);
      if (found.length && !found.includes(model())) setModel(found[0]);
    } catch {
      setModels([]);
    }
  });

  const save = async () => {
    setBusy(true);
    try {
      await api.patchGatewayAllowlist(props.entry.key, {
        workspace: workspace().trim() || undefined,
        route: pinRoute() && provider() && model() ? { provider: provider(), model: model() } : {},
        // "" clears the pin back to inheriting the workspace default.
        permission_mode: pinPerm() ? perm() : "",
      });
      pushToast("info", `Updated ${props.entry.key} — the next message rotates to a fresh session`);
      props.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="pending-card" style="margin-top:10px">
      <div class="binding-meta" style="margin-bottom:6px">
        Approved {timeAgo(props.entry.added_at)} by {props.entry.added_by} — editing keeps that
        provenance. A changed workspace or route rotates to a new frozen session on the next
        message; the existing ledger is preserved.
      </div>
      <WorkspacePicker
        value={workspace()}
        onChange={setWorkspace}
        known={props.knownWorkspaces}
        corePool={props.corePool}
      />
      <label class="inherit-toggle">
        <input type="checkbox" checked={pinRoute()} onChange={(e) => setPinRoute(e.currentTarget.checked)} />
        Pin a specific provider / model (otherwise inherits the workspace default)
      </label>
      <Show when={pinRoute()}>
        <div class="binding-controls">
          <select value={provider()} onChange={(e) => setProvider(e.currentTarget.value)}>
            <For each={props.providers}>{(p) => <option value={p.name}>{p.name}</option>}</For>
          </select>
          <Show when={models().length} fallback={
            <input class="mono" value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="model id" />
          }>
            <select class="mono" value={model()} onChange={(e) => setModel(e.currentTarget.value)}>
              <For each={models()}>{(m) => <option value={m}>{m}</option>}</For>
            </select>
          </Show>
        </div>
      </Show>
      <ChannelPermissionPicker
        pinned={pinPerm()}
        setPinned={setPinPerm}
        mode={perm()}
        setMode={setPerm}
        ceiling={props.entry.workspace_permission_mode}
      />
      <div class="row-gap">
        <button disabled={busy()} onClick={save}>Save access</button>
      </div>
    </div>
  );
}

function PendingChannelCard(props: {
  entry: AllowlistEntry;
  providers: ProviderSummary[];
  defaultWorkspace: string;
  knownWorkspaces: string[];
  corePool: CorePoolEntry[];
  refresh: () => void;
}) {
  const [workspace, setWorkspace] = createSignal(props.defaultWorkspace);
  const [pinRoute, setPinRoute] = createSignal(false);
  const [provider, setProvider] = createSignal(props.providers[0]?.name ?? "");
  const [model, setModel] = createSignal("");
  const [models, setModels] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [pinPerm, setPinPerm] = createSignal(false);
  const [perm, setPerm] = createSignal<PermissionMode>("workspace-write");

  createEffect(async () => {
    if (!pinRoute() || !provider()) return;
    try {
      const found = (await api.models(provider())).models ?? [];
      setModels(found);
      if (found.length && !found.includes(model())) setModel(found[0]);
    } catch {
      setModels([]);
    }
  });

  const approve = async () => {
    setBusy(true);
    try {
      const entry = await api.approveGatewayAllowlist(props.entry.key, {
        workspace: workspace().trim() || undefined,
        route: pinRoute() && provider() && model() ? { provider: provider(), model: model() } : undefined,
        permission_mode: pinPerm() ? perm() : undefined,
      });
      // Surface a capped grant at the moment it happens: the server
      // reduces an over-broad pin to the workspace's own mode, and an
      // operator who is not told would believe the wider grant is live.
      if (entry.permission_capped) {
        pushToast(
          "alert",
          `${props.entry.key}: ${entry.permission_mode} exceeds that workspace's own mode — capped to ${entry.effective_permission_mode}`,
        );
      }
      pushToast("info", `Approved ${props.entry.key} → ${entry.workspace ?? "(workspace unset)"}`);
      props.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const deny = async () => {
    setBusy(true);
    try {
      await api.denyGatewayAllowlist(props.entry.key);
      pushToast("info", `Denied ${props.entry.key}`);
      props.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <article class="binding-card pending-card">
      <div class="binding-head">
        <div>
          <strong class="mono">{props.entry.key}</strong>
          <div class="binding-meta">first seen {timeAgo(props.entry.added_at)}</div>
        </div>
        <div class="row-gap" style="gap:6px">
          <SurfaceBadge channelKey={props.entry.key} />
          <span class="chip chip-tone-warning">pending</span>
        </div>
      </div>

      <Show when={props.entry.first_seen_text}>
        <div class="pending-first-seen">{props.entry.first_seen_text}</div>
      </Show>

      <WorkspacePicker
        value={workspace()}
        onChange={setWorkspace}
        known={props.knownWorkspaces}
        corePool={props.corePool}
      />

      <label class="inherit-toggle">
        <input type="checkbox" checked={pinRoute()} onChange={(e) => setPinRoute(e.currentTarget.checked)} />
        Pin a specific provider / model (otherwise inherits the workspace default)
      </label>
      <Show when={pinRoute()}>
        <div class="binding-controls">
          <select value={provider()} onChange={(e) => setProvider(e.currentTarget.value)}>
            <For each={props.providers}>{(p) => <option value={p.name}>{p.name}</option>}</For>
          </select>
          <Show when={models().length} fallback={
            <input class="mono" value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="model id" />
          }>
            <select class="mono" value={model()} onChange={(e) => setModel(e.currentTarget.value)}>
              <For each={models()}>{(m) => <option value={m}>{m}</option>}</For>
            </select>
          </Show>
        </div>
      </Show>

      <ChannelPermissionPicker
        pinned={pinPerm()}
        setPinned={setPinPerm}
        mode={perm()}
        setMode={setPerm}
      />

      <div class="row-gap">
        <button disabled={busy()} onClick={approve}>Approve</button>
        <button class="danger small" disabled={busy()} onClick={deny}>Deny</button>
      </div>
    </article>
  );
}

// One credential row: the token a bridge authenticates to its platform
// with (BotFather, Discord's Developer Portal, a Slack app's Bot User
// OAuth Token). Deliberately shares no screen real estate with a routing
// key (`surface:chat`) — the two were adjacent in one long page, and an
// operator pasted a token into the routing-key field.
function CredentialRow(props: { surface: string; state: ChatSurfaceStatus | undefined; refresh: () => void }) {
  const [editing, setEditing] = createSignal(false);
  const [draft, setDraft] = createSignal("");
  const [busy, setBusy] = createSignal<"save" | "remove" | null>(null);

  const configured = () => props.state?.configured ?? false;

  const save = async () => {
    const token = draft().trim();
    if (!token) return;
    setBusy("save");
    try {
      const res = await api.putBotToken(props.surface, token);
      setEditing(false);
      setDraft("");
      pushToast(
        "info",
        res.restarted
          ? `${props.surface} token saved and the bridge restarted`
          : `${props.surface} token saved — start the bridge by hand (vak ${props.surface} --server …) or from the tray`,
      );
      props.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(null);
    }
  };

  const remove = async () => {
    if (!window.confirm(`Remove the ${props.surface} bot token? The bridge stops authenticating and every channel on that surface goes quiet until a new token is set.`)) return;
    setBusy("remove");
    try {
      await api.removeBotToken(props.surface);
      pushToast("info", `${props.surface} token removed`);
      props.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div class="cred-row">
      <div class="cred-id">
        <SurfaceBadge channelKey={`${props.surface}:`} />
        <code class="binding-meta">{props.state?.env_var ?? "—"}</code>
      </div>
      <span class={`chip ${configured() ? "chip-tone-success" : ""}`}>
        {configured() ? "token set" : "no token"}
      </span>
      <span class="binding-meta cred-service">
        {props.state?.managed_service ? "managed service — restarts on save" : "started by hand"}
      </span>
      <span class="spacer" />
      <Show
        when={editing()}
        fallback={
          <div class="row-gap">
            <button class="ghost small" onClick={() => setEditing(true)}>
              {configured() ? "Replace token" : "Set token"}
            </button>
            <Show when={configured()}>
              <button class="danger small" disabled={busy() === "remove"} onClick={() => void remove()}>
                {busy() === "remove" ? "Removing…" : "Remove"}
              </button>
            </Show>
          </div>
        }
      >
        <div class="cred-edit">
          <input
            type="password"
            autocomplete="off"
            spellcheck={false}
            autofocus
            placeholder={`paste the ${props.surface} bot token`}
            value={draft()}
            onInput={(e) => setDraft(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void save();
              if (e.key === "Escape") setEditing(false);
            }}
          />
          <button disabled={busy() === "save" || !draft().trim()} onClick={() => void save()}>
            {busy() === "save" ? "Saving…" : "Save"}
          </button>
          <button class="ghost small" onClick={() => { setEditing(false); setDraft(""); }}>Cancel</button>
        </div>
      </Show>
    </div>
  );
}

/// Everything the four gateway screens read, fetched once at the section
/// level so a refresh on one screen is a refresh on all of them.
interface GatewayCtx {
  status: () => GatewayStatus | undefined;
  statusLoading: () => boolean;
  entries: () => AllowlistEntry[];
  pending: () => AllowlistEntry[];
  providers: () => ProviderSummary[];
  surfaces: () => ChatSurfaceStatus[];
  surfacesLoading: () => boolean;
  configured: () => ChatSurfaceStatus[];
  refresh: () => void;
}

function SkeletonRows(props: { rows?: number; cols: number }) {
  return (
    <For each={Array.from({ length: props.rows ?? 3 })}>
      {() => (
        <tr class="skel-row">
          <For each={Array.from({ length: props.cols })}>{() => <td><span class="skel" /></td>}</For>
        </tr>
      )}
    </For>
  );
}

// ---- Gateway › Channels ----------------------------------------------------

/// The settled, scan-first view: every approved channel, what it routes
/// to, what permission it runs with. Editing is a row expansion, so the
/// page reads as a list of channels first and a form only on demand.
function ChannelsView(props: { ctx: GatewayCtx }) {
  const [filter, setFilter] = createSignal("");
  const [expanded, setExpanded] = createSignal("");
  const [manualKey, setManualKey] = createSignal("");
  const [registering, setRegistering] = createSignal(false);

  const byKey = createMemo(() => {
    const map = new Map<string, AllowlistEntry>();
    for (const e of props.ctx.entries()) map.set(e.key, e);
    return map;
  });

  const rows = createMemo(() => {
    const needle = filter().trim().toLowerCase();
    return (props.ctx.status()?.bindings ?? []).filter(
      (b) => !needle || b.target.toLowerCase().includes(needle) || b.workspace.toLowerCase().includes(needle),
    );
  });

  const registerManually = async () => {
    const key = manualKey().trim();
    const surface = key.split(":", 1)[0];
    if (!key.includes(":") || !(KNOWN_SURFACES as readonly string[]).includes(surface)) {
      pushToast(
        "alert",
        `A routing key looks like surface:chat — for example telegram:12345, with the surface one of ${KNOWN_SURFACES.join(", ")}. Bot tokens live under Credentials and are never entered here.`,
      );
      return;
    }
    setRegistering(true);
    try {
      await api.patchGatewayBinding(key, {});
      setManualKey("");
      pushToast("info", `Registered ${key} with the workspace default`);
      props.ctx.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setRegistering(false);
    }
  };

  return (
    <>
      <div class="toolbar">
        <input
          class="search-input"
          placeholder="Filter by channel or workspace…"
          value={filter()}
          onInput={(e) => setFilter(e.currentTarget.value)}
        />
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refresh()}>Refresh</button>
        <button onClick={() => navigate("#/gateway/connect")}>Connect a channel</button>
      </div>

      <Show when={props.ctx.pending().length > 0}>
        <section class="panel panel-alert callout">
          <div>
            <strong>
              {props.ctx.pending().length} chat{props.ctx.pending().length === 1 ? "" : "s"} waiting for review
            </strong>
            <p class="dim">
              Unrecognized chats are rejected on arrival and recorded — nothing is allowed in silently.
            </p>
          </div>
          <button class="ghost" onClick={() => navigate("#/gateway/connect")}>Review</button>
        </section>
      </Show>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Registered channels</h2>
            <Show when={(props.ctx.status()?.bindings.length ?? 0) > 0}>
              <p class="dim">
                Each row is one approved chat: where it runs, what it routes to, and what it is
                allowed to do. Open a row to change any of that or to revoke it.
              </p>
            </Show>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.statusLoading()}>
            <table class="table">
              <thead>
                <tr><th>channel</th><th>surface</th><th>workspace</th><th>route</th><th>permission</th><th>status</th><th /></tr>
              </thead>
              <tbody><SkeletonRows cols={7} /></tbody>
            </table>
          </Match>

          <Match when={(props.ctx.status()?.bindings.length ?? 0) === 0}>
            <div class="empty empty-teach">
              <strong>No channels are connected yet.</strong>
              <p>
                A channel is one chat — a Telegram group, a Discord channel, a Slack conversation —
                bound to a workspace and a model. Connecting one takes three steps: give the bridge
                its bot token, message the bot from the chat, then approve it here.
              </p>
              <button onClick={() => navigate("#/gateway/connect")}>Connect a channel</button>
            </div>
          </Match>

          <Match when={rows().length === 0}>
            <div class="empty">
              No channel matches “{filter()}”.{" "}
              <button class="ghost small" onClick={() => setFilter("")}>Clear filter</button>
            </div>
          </Match>

          <Match when={rows().length > 0}>
            <table class="table">
              <thead>
                <tr><th>channel</th><th>surface</th><th>workspace</th><th>route</th><th>permission</th><th>status</th><th /></tr>
              </thead>
              <tbody>
                <For each={rows()}>
                  {(binding) => {
                    const entry = () => byKey().get(binding.target);
                    const open = () => expanded() === binding.target;
                    return (
                      <>
                        <tr
                          classList={{ "row-open": open() }}
                          onClick={() => setExpanded(open() ? "" : binding.target)}
                        >
                          <td class="mono">{binding.target}</td>
                          <td><SurfaceBadge channelKey={binding.target} /></td>
                          <td class="dim col-path"><PathCell path={binding.workspace} /></td>
                          <td class="mono dim">
                            {binding.effective_route.provider} / {binding.effective_route.model}
                            <Show when={binding.override}>
                              <span class="chip chip-mode" style="margin-left:6px">pinned</span>
                            </Show>
                          </td>
                          <td>
                            <Show when={entry()?.effective_permission_mode} fallback={<span class="dim">—</span>}>
                              <span class="chip chip-kind">{entry()!.effective_permission_mode}</span>
                              <Show when={entry()!.permission_capped}>
                                <span class="chip chip-tone-warning" style="margin-left:6px">capped</span>
                              </Show>
                            </Show>
                          </td>
                          <td>
                            <Show when={entry()}>
                              <span class={`chip chip-tone-${ALLOWLIST_CHIP_TONE[entry()!.status] ?? ""}`}>
                                {entry()!.status}
                              </span>
                            </Show>
                            <Show when={binding.stale}>
                              <span class="chip chip-tone-warning" style="margin-left:6px">rotates next</span>
                            </Show>
                          </td>
                          <td><span class="chev">{open() ? "⌄" : "›"}</span></td>
                        </tr>
                        <Show when={open()}>
                          <tr class="row-detail">
                            <td colspan={7}>
                              <GatewayBindingEditor
                                binding={binding}
                                providers={props.ctx.providers()}
                                allowlistEntry={entry()}
                                knownWorkspaces={props.ctx.status()?.known_workspaces ?? []}
                                corePool={props.ctx.status()?.core_pool.entries ?? []}
                                refresh={props.ctx.refresh}
                              />
                            </td>
                          </tr>
                        </Show>
                      </>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>

        <details class="advanced">
          <summary>Register a routing key by hand</summary>
          <p class="dim">
            Only for a chat whose id you already know — normally a channel registers itself the first
            time it messages the bot. This is the routing key <code>surface:chat</code>, never a bot
            token; tokens are set under Credentials.
          </p>
          <div class="add-binding">
            <input
              class="mono"
              value={manualKey()}
              placeholder="telegram:12345"
              onInput={(e) => setManualKey(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && void registerManually()}
            />
            <button disabled={registering() || !manualKey().trim()} onClick={() => void registerManually()}>
              {registering() ? "Registering…" : "Register"}
            </button>
          </div>
        </details>
      </section>
    </>
  );
}

// ---- Gateway › Connect -----------------------------------------------------

/// Onboarding, as the sequence it actually is. Each step reports its own
/// state from the live backend rather than asking the operator to keep
/// track: a token either is set or is not, a chat either has knocked or
/// has not.
function ConnectView(props: { ctx: GatewayCtx }) {
  const credentialDone = () => props.ctx.configured().length > 0;
  const knockDone = () => props.ctx.pending().length > 0 || (props.ctx.status()?.bindings.length ?? 0) > 0;
  const current = () => (!credentialDone() ? 1 : !props.ctx.pending().length ? 2 : 3);

  const stepState = (n: number) => {
    const done = n === 1 ? credentialDone() : n === 2 ? knockDone() : false;
    if (done && current() !== n) return "done";
    return current() === n ? "current" : "waiting";
  };

  return (
    <>
      <div class="toolbar">
        <div>
          <h2 class="view-title">Connect a channel</h2>
          <p class="dim">From a bot with no credential to a chat routed at a workspace.</p>
        </div>
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refresh()}>Refresh</button>
        <button class="ghost" onClick={() => navigate("#/gateway")}>All channels</button>
      </div>

      <ol class="steps">
        <li class="step" data-state={stepState(1)}>
          <span class="step-index">1</span>
          <div class="step-body">
            <h3>Give the bridge its bot token</h3>
            <p class="dim">
              The credential the bridge authenticates with — from BotFather (Telegram), the Discord
              Developer Portal, or a Slack app's Bot User OAuth Token. Stored in the shared user{" "}
              <code>.env</code> and never read back.
            </p>
            <Show when={!props.ctx.surfacesLoading()} fallback={<div class="cred-list"><span class="skel skel-block" /><span class="skel skel-block" /></div>}>
              <div class="cred-list">
                <For each={KNOWN_SURFACES}>
                  {(surface) => (
                    <CredentialRow
                      surface={surface}
                      state={props.ctx.surfaces().find((s) => s.surface === surface)}
                      refresh={props.ctx.refresh}
                    />
                  )}
                </For>
              </div>
            </Show>
          </div>
        </li>

        <li class="step" data-state={stepState(2)}>
          <span class="step-index">2</span>
          <div class="step-body">
            <h3>Message the bot from the chat you want to connect</h3>
            <p class="dim">
              The first message is rejected on purpose — vak records the chat as pending instead of
              letting it in. That rejection is what puts it in front of you below.
            </p>
            <Show
              when={props.ctx.pending().length > 0}
              fallback={
                <div class="waiting-line">
                  <span class={`dot dot-${conn()}`} />
                  <Show when={credentialDone()} fallback={<span class="dim">Waiting on step 1 — no bot token is set yet.</span>}>
                    <span class="dim">Listening. Send any message to the bot and it appears in step 3.</span>
                  </Show>
                </div>
              }
            >
              <div class="waiting-line">
                <span class="chip chip-tone-success">
                  {props.ctx.pending().length} chat{props.ctx.pending().length === 1 ? "" : "s"} knocked
                </span>
              </div>
            </Show>
          </div>
        </li>

        <li class="step" data-state={stepState(3)}>
          <span class="step-index">3</span>
          <div class="step-body">
            <h3>Review and approve</h3>
            <p class="dim">
              Approving binds the chat to a workspace and, optionally, pins a route and a permission
              mode. Anything left unpinned inherits the workspace default and can be changed later
              from Channels.
            </p>
            <Show
              when={props.ctx.pending().length > 0}
              fallback={
                <div class="empty">
                  Nothing is waiting for review.{" "}
                  <Show when={(props.ctx.status()?.bindings.length ?? 0) > 0}>
                    <button class="ghost small" onClick={() => navigate("#/gateway")}>See connected channels</button>
                  </Show>
                </div>
              }
            >
              <div class="binding-list">
                <For each={props.ctx.pending()}>
                  {(entry) => (
                    <PendingChannelCard
                      entry={entry}
                      providers={props.ctx.providers()}
                      defaultWorkspace={props.ctx.status()?.workspace ?? ""}
                      knownWorkspaces={props.ctx.status()?.known_workspaces ?? []}
                      corePool={props.ctx.status()?.core_pool.entries ?? []}
                      refresh={props.ctx.refresh}
                    />
                  )}
                </For>
              </div>
            </Show>
          </div>
        </li>
      </ol>
    </>
  );
}

// ---- Gateway › Credentials -------------------------------------------------

function CredentialsView(props: { ctx: GatewayCtx }) {
  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>Bot tokens</h2>
          <p class="dim">
            One credential per chat surface — what the bridge authenticates to Telegram, Discord, or
            Slack with. Kept in the shared user <code>.env</code>; the console can set or clear a
            token but never reads one back. Routing keys (<code>surface:chat</code>) are a different
            thing entirely and live under Channels.
          </p>
        </div>
        <button class="ghost small" onClick={() => props.ctx.refresh()}>Refresh</button>
      </div>
      <Show
        when={!props.ctx.surfacesLoading()}
        fallback={<div class="cred-list"><span class="skel skel-block" /><span class="skel skel-block" /><span class="skel skel-block" /></div>}
      >
        <div class="cred-list">
          <For each={KNOWN_SURFACES}>
            {(surface) => (
              <CredentialRow
                surface={surface}
                state={props.ctx.surfaces().find((s) => s.surface === surface)}
                refresh={props.ctx.refresh}
              />
            )}
          </For>
        </div>
      </Show>
    </section>
  );
}

// ---- Gateway › Routing & pool ----------------------------------------------

function GatewayHealthView(props: { ctx: GatewayCtx }) {
  const pool = () => props.ctx.status()?.core_pool;
  return (
    <>
      <section class="panel gateway-summary">
        <div class="panel-title-row">
          <div>
            <h2>Routing defaults</h2>
            <p class="dim">
              What a channel gets when it pins nothing of its own. Defaults propagate to inheriting
              channels; frozen sessions rotate rather than mutate.
            </p>
          </div>
          <button class="ghost small" onClick={() => props.ctx.refresh()}>Refresh</button>
        </div>
        <Show
          when={!props.ctx.statusLoading()}
          fallback={<div class="route-summary-grid"><span class="skel skel-block" /><span class="skel skel-block" /><span class="skel skel-block" /><span class="skel skel-block" /></div>}
        >
          <div class="route-summary-grid">
            <div><span class="eyebrow">Gateway</span><strong>{props.ctx.status()?.enabled ? "Enabled" : "Disabled"}</strong></div>
            <div><span class="eyebrow">Workspace</span><PathCell path={props.ctx.status()?.workspace ?? ""} budget={46} /></div>
            <div><span class="eyebrow">Admin default</span><strong>{props.ctx.status()?.default_route.provider}</strong><code>{props.ctx.status()?.default_route.model}</code></div>
            <div>
              <span class="eyebrow">Provenance</span>
              <code>{props.ctx.status()?.default_route.provider_source} + {props.ctx.status()?.default_route.model_source}</code>
              <span class="binding-meta">{props.ctx.status()?.default_route.revision}</span>
            </div>
          </div>
        </Show>
      </section>

      <section class="panel" style="margin-top:14px">
        <div class="panel-title-row">
          <div>
            <h2>Core pool</h2>
            <p class="dim">
              Workspaces holding a live Core (sandbox, permission mode, session ledger). A cold
              workspace starts its own Core on the next inbound message. Max {pool()?.max ?? "—"}{" "}
              pooled, idle eviction after {pool()?.idle_secs ?? "—"}s.
            </p>
          </div>
        </div>
        <Switch>
          <Match when={props.ctx.statusLoading()}>
            <div class="cred-list"><span class="skel skel-block" /><span class="skel skel-block" /></div>
          </Match>
          <Match when={(pool()?.entries.length ?? 0) === 0}>
            <div class="empty empty-teach">
              <strong>Every workspace is cold.</strong>
              <p>
                Nothing is wrong: a Core starts on the first inbound message to a channel and stays
                warm here until it has been idle for {pool()?.idle_secs ?? "—"}s.
              </p>
            </div>
          </Match>
          <Match when={(pool()?.entries.length ?? 0) > 0}>
            <table class="table">
              <thead>
                <tr><th>workspace</th><th>permission</th><th>idle</th><th /></tr>
              </thead>
              <tbody>
                <For each={pool()?.entries}>
                  {(entry) => (
                    <tr class="row-static">
                      <td class="col-path"><PathCell path={entry.workspace} /></td>
                      <td><span class="chip chip-kind">{entry.effective_permission_mode}</span></td>
                      <td class="dim">{entry.idle_secs}s</td>
                      <td>
                        <Show when={entry.is_default}><span class="chip chip-mode">default</span></Show>
                        <span class="chip chip-tone-success" style="margin-left:6px">warm</span>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>
      </section>
    </>
  );
}

// ---- Gateway section shell -------------------------------------------------

const GATEWAY_TABS = [
  { hash: "#/gateway", label: "Channels" },
  { hash: "#/gateway/connect", label: "Connect" },
  { hash: "#/gateway/credentials", label: "Credentials" },
  { hash: "#/gateway/routing", label: "Routing & pool" },
] as const;

function gatewayTab(): string {
  const r = route();
  return GATEWAY_TABS.slice(1).find((t) => r.startsWith(t.hash))?.hash ?? "#/gateway";
}

function GatewaySection() {
  const [status, { refetch: refetchStatus }] = createResource(() => api.gatewayStatus());
  const [allowlist, { refetch: refetchAllowlist }] = createResource(() => api.gatewayAllowlist());
  const [providers] = createResource(() => api.providers().catch(() => ({ providers: [] })));
  const [surfaces, { refetch: refetchSurfaces }] = createResource(() => api.chatSurfaces().catch(() => []));

  const refresh = () => {
    refetchStatus();
    refetchAllowlist();
    refetchSurfaces();
  };

  const entries = createMemo(() => allowlist()?.entries ?? []);
  const ctx: GatewayCtx = {
    status: () => status(),
    statusLoading: () => status.loading,
    entries,
    pending: createMemo(() => entries().filter((e) => e.status === "pending")),
    providers: () => providers()?.providers ?? [],
    surfaces: () => surfaces() ?? [],
    surfacesLoading: () => surfaces.loading,
    configured: createMemo(() => (surfaces() ?? []).filter((s) => s.configured)),
    refresh,
  };

  return (
    <div class="view">
      <div class="tab-bar">
        <For each={GATEWAY_TABS}>
          {(t) => (
            <a
              class="tab-btn"
              href={t.hash}
              classList={{ active: gatewayTab() === t.hash }}
            >
              {t.label}
              <Show when={t.hash === "#/gateway/connect" && ctx.pending().length > 0}>
                <span class="nav-badge">{ctx.pending().length}</span>
              </Show>
            </a>
          )}
        </For>
      </div>

      <Switch>
        <Match when={gatewayTab() === "#/gateway"}><ChannelsView ctx={ctx} /></Match>
        <Match when={gatewayTab() === "#/gateway/connect"}><ConnectView ctx={ctx} /></Match>
        <Match when={gatewayTab() === "#/gateway/credentials"}><CredentialsView ctx={ctx} /></Match>
        <Match when={gatewayTab() === "#/gateway/routing"}><GatewayHealthView ctx={ctx} /></Match>
      </Switch>
    </div>
  );
}

const MODES = ["ReadOnly", "WorkspaceWrite", "FullAccess"];

const PROVIDERS = [
  { id: "anthropic", label: "Anthropic (Claude)" },
  { id: "openai", label: "OpenAI (Responses)" },
  { id: "google", label: "Google (Gemini)" },
  { id: "ollama", label: "Ollama (local)" },
] as const;

const MODE_COPY: Record<string, string> = {
  ReadOnly: "Read tools only, confined to the workspace. Every write is denied outright.",
  WorkspaceWrite: "Writes inside the workspace go through; anything else asks first.",
  FullAccess: "Nothing is gated. Only for a workspace you have decided to trust completely.",
};

/// One page, six self-contained panels. Each answers a single question about
/// how this instance is configured; nothing here is a summary of a screen
/// that already exists elsewhere.
function Settings() {
  const [config, { refetch: refetchConfig }] = createResource(() => api.config());
  const [providersData, { refetch: refetchProviders }] = createResource(() => api.providers());
  const [rebuilding, setRebuilding] = createSignal(false);
  const [doctorReport, setDoctorReport] = createSignal<string | null>(null);
  const [runningDoctor, setRunningDoctor] = createSignal(false);

  const [selectedProvider, setSelectedProvider] = createSignal("anthropic");
  const [selectedModel, setSelectedModel] = createSignal("");
  const [discoveredModels, setDiscoveredModels] = createSignal<string[]>([]);
  const [loadingModels, setLoadingModels] = createSignal(false);
  const [modelError, setModelError] = createSignal("");
  const [providerKeyInput, setProviderKeyInput] = createSignal("");
  const [savingKey, setSavingKey] = createSignal(false);

  let initialized = false;
  createEffect(() => {
    const c = config();
    if (c && !initialized) {
      initialized = true;
      setSelectedProvider(c.provider || "anthropic");
      setSelectedModel(c.model || "");
    }
  });

  const discover = async (provider: string) => {
    setLoadingModels(true);
    setModelError("");
    try {
      const res = await api.models(provider);
      const models = res.models ?? [];
      setDiscoveredModels(models);
      if (models.length > 0 && !models.includes(selectedModel())) setSelectedModel(models[0]);
    } catch (err) {
      setDiscoveredModels([]);
      // Almost always "no key for this provider yet", which the free-text
      // model field below already lets the operator work around.
      setModelError(`${err}`);
    } finally {
      setLoadingModels(false);
    }
  };

  createEffect(() => {
    const provider = selectedProvider();
    if (provider) void discover(provider);
  });

  const keyConfigured = createMemo(
    () => providersData()?.providers?.find((p) => p.name === selectedProvider())?.configured ?? false,
  );

  const guard = async (work: () => Promise<void>, ok: string) => {
    try {
      await work();
      pushToast("info", ok);
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
      pushToast("info", `Key stored for ${selectedProvider()}`);
      setProviderKeyInput("");
      refetchProviders();
      await discover(selectedProvider());
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setSavingKey(false);
    }
  };

  const rebuild = async () => {
    setRebuilding(true);
    try {
      const stats = await api.rebuild();
      if (stats.ok) pushToast("info", `Reindexed ${stats.entries_indexed} entries from ${stats.files_scanned} files`);
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
      setDoctorReport((await api.doctor()).report);
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

  const rules = createMemo(() => parseRuleLists(config()?.permissions));
  const rulesFor = (decision: RuleDecision) => rules().filter((r) => r.decision === decision);

  return (
    <div class="view">
      <div class="two-col">
        <div class="stack">
          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>Model</h2>
                <p class="dim">What answers a turn when nothing pins a different route.</p>
              </div>
            </div>
            <Show when={!config.loading} fallback={<div class="cred-list"><span class="skel skel-block" /><span class="skel skel-block" /></div>}>
              <div class="form-row">
                <label>provider</label>
                <select value={selectedProvider()} onChange={(e) => setSelectedProvider(e.currentTarget.value)}>
                  <For each={PROVIDERS}>{(p) => <option value={p.id}>{p.label}</option>}</For>
                </select>
              </div>
              <div class="form-row">
                <label>model</label>
                <Show
                  when={discoveredModels().length > 0}
                  fallback={
                    <input
                      class="mono"
                      placeholder={loadingModels() ? "Asking the provider…" : "claude-sonnet-4-5-20250929"}
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
              <Show when={modelError() && discoveredModels().length === 0}>
                <p class="dim">
                  Could not list models for {selectedProvider()} ({modelError()}). Set its key below,
                  or type a model id by hand.
                </p>
              </Show>
              <div class="row-gap" style="margin-top:10px">
                <button
                  disabled={!selectedModel().trim()}
                  onClick={() =>
                    void guard(
                      () => api.patchConfig({ provider: selectedProvider(), model: selectedModel() }),
                      `Route is now ${selectedProvider()} / ${selectedModel()}`,
                    )
                  }
                >
                  Save route
                </button>
                <button class="ghost small" disabled={loadingModels()} onClick={() => void discover(selectedProvider())}>
                  {loadingModels() ? "Discovering…" : "Rediscover models"}
                </button>
              </div>
            </Show>
          </section>

          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>Provider key</h2>
                <p class="dim">
                  Stored for {selectedProvider()}. Written to the user <code>.env</code>; it is never
                  read back into this page.
                </p>
              </div>
              <span class={`chip chip-tone-${keyConfigured() ? "success" : "warning"}`}>
                {keyConfigured() ? "set" : "not set"}
              </span>
            </div>
            <div class="form-row">
              <label>key</label>
              <input
                type="password"
                autocomplete="off"
                placeholder="sk-…"
                value={providerKeyInput()}
                onInput={(e) => setProviderKeyInput(e.currentTarget.value)}
              />
            </div>
            <div class="row-gap" style="margin-top:10px">
              <button disabled={savingKey() || !providerKeyInput().trim()} onClick={() => void saveKey()}>
                {savingKey() ? "Storing…" : "Store key"}
              </button>
              <Show when={keyConfigured()}>
                <button
                  class="danger small"
                  onClick={() =>
                    void guard(async () => {
                      await api.deleteProviderKey(selectedProvider());
                      refetchProviders();
                      setDiscoveredModels([]);
                    }, `Key revoked for ${selectedProvider()}`)
                  }
                >
                  Revoke
                </button>
              </Show>
            </div>
          </section>

          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>Appearance</h2>
                <p class="dim">Applies to this browser only.</p>
              </div>
            </div>
            <div class="theme-grid">
              <For
                each={[
                  { id: "warm", label: "Warm dark", class: "" },
                  { id: "dark", label: "Midnight", class: "dark" },
                  { id: "contrast", label: "High contrast", class: "contrast" },
                ] as const}
              >
                {(t) => (
                  <button class="theme-choice" classList={{ active: theme() === t.id }} onClick={() => setTheme(t.id)}>
                    <span class={`theme-preview ${t.class}`}><i /><i /><i /></span>
                    <strong>{t.label}</strong>
                  </button>
                )}
              </For>
            </div>
          </section>
        </div>

        <div class="stack">
          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>Permissions</h2>
                <p class="dim">
                  The mode decides every call no rule covers. Rules are checked first, and among the
                  rules that match one call, deny outranks ask outranks allow — the order they appear
                  in never decides it.
                </p>
              </div>
            </div>
            <div class="mode-grid">
              <For each={MODES}>
                {(m) => (
                  <button
                    class="mode-btn"
                    classList={{ active: config()?.permission_mode === m }}
                    onClick={() => void guard(() => api.setMode(m), `Permission mode → ${m}`)}
                    disabled={config()?.permission_mode === m}
                  >
                    <span class="mode-name">{m}</span>
                    <span class="mode-desc">{MODE_COPY[m]}</span>
                  </button>
                )}
              </For>
            </div>

            <Show
              when={rulesReported(config())}
              fallback={
                <Show when={!config.loading}>
                  <div class="rule-lists">
                    <p class="dim">
                      This server build does not report its rule lists, so the rules layered on top
                      of the mode cannot be shown. Everything above still applies; read{" "}
                      <code>allow</code>/<code>ask</code>/<code>deny</code> in <code>config.toml</code>{" "}
                      directly, or update vak.
                    </p>
                  </div>
                </Show>
              }
            >
              <div class="rule-lists">
                <For each={["deny", "ask", "allow"] as const}>
                  {(decision) => (
                    <div>
                      <span class="eyebrow">{decision} rules</span>
                      <Show
                        when={rulesFor(decision).length > 0}
                        fallback={<p class="dim">None — nothing is {decision === "allow" ? "pre-approved" : decision === "deny" ? "blocked outright" : "forced to ask"} beyond what the mode already decides.</p>}
                      >
                        <div class="chip-stack">
                          <For each={rulesFor(decision)}>{(r) => <RuleChip rule={r} />}</For>
                        </div>
                      </Show>
                    </div>
                  )}
                </For>
              </div>
              <p class="dim">
                Rules live in <code>config.toml</code> and are read-only here — editing them from a
                browser session would let the console widen its own reach. See what each one grants a
                given extension under <a href="#/integrations">Extensions</a>.
              </p>
            </Show>
          </section>

          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>Maintenance</h2>
                <p class="dim">Neither is destructive; both can take a moment on a large store.</p>
              </div>
            </div>
            <div class="row-gap">
              <button class="ghost" disabled={runningDoctor()} onClick={() => void runDoctor()}>
                {runningDoctor() ? "Diagnosing…" : "Run diagnostics"}
              </button>
              <button class="ghost" disabled={rebuilding()} onClick={() => void rebuild()}>
                {rebuilding() ? "Reindexing…" : "Rebuild search index"}
              </button>
            </div>
            <Show when={doctorReport()}>
              <pre class="mono report-pre">{doctorReport()}</pre>
              <div class="row-gap">
                <button class="ghost small" onClick={() => setDoctorReport(null)}>Dismiss report</button>
              </div>
            </Show>
          </section>

          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>This session</h2>
                <p class="dim">
                  Signing out clears the console cookie here. Runs already in flight keep going.
                </p>
              </div>
            </div>
            <div class="row-gap">
              <button class="danger" onClick={() => void signOut()}>Sign out</button>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

// ---- Shell -----------------------------------------------------------------

interface NavItem {
  hash: string;
  label: string;
  icon: string;
  badge?: () => string;
  /// Sub-destinations, rendered in the sidebar while the section is open.
  children?: readonly { readonly hash: string; readonly label: string }[];
  /// Which child is current. A section with children owns this, because only
  /// it knows how its sub-routes resolve (a bare section hash is its own
  /// first tab, not "no tab").
  activeChild?: () => string;
}

const NAV: NavItem[] = [
  { hash: "#/overview", label: "Overview", icon: ICONS.overview },
  { hash: "#/sessions", label: "Sessions", icon: ICONS.sessions },
  // Extensions are four distinct governance questions — what external
  // processes can be started, what instructions are loaded, what intercepts
  // a run, what runs unattended — and they read as four screens for the
  // same reason the Gateway does.
  {
    hash: "#/integrations",
    label: "Extensions",
    icon: ICONS.integrations,
    children: EXTENSION_TABS,
    activeChild: extensionsTab,
  },
  // The gateway is four distinct jobs, not one page: watch the channels
  // you have, walk a new one in, handle credentials, check routing and
  // pool health. The sub-rows expand in place when the section is open so
  // the destination is nameable from the nav rather than found by
  // scrolling one long view.
  {
    hash: "#/gateway",
    label: "Gateway",
    icon: ICONS.gateway,
    children: GATEWAY_TABS,
    activeChild: gatewayTab,
  },
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
      } catch (err) {
        // A cookie left over from before a server restart (a fresh
        // token, or a full self uninstall --purge / reinstall) fails
        // every request from here on, but nothing about the page looks
        // broken -- it just quietly stops updating. Bounce back to the
        // login screen instead of polling a dead session forever.
        if (err instanceof AuthRequired) setAuthed(false);
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
            <div class="brand"><span class="brand-mark"><img src="/admin/vak-icon.png" alt="" /></span> vak</div>
            <nav>
              <For each={NAV}>
                {(item) => (
                  <>
                    <a href={item.hash} classList={{ active: currentRoute() === item.hash }}>
                      <Icon d={item.icon} />
                      {item.label}
                      <Show when={"badge" in item && item.badge?.() && Number(item.badge!()) > 0}>
                        <span class="nav-badge">{item.badge!()}</span>
                      </Show>
                    </a>
                    <Show when={item.children && currentRoute() === item.hash}>
                      <div class="nav-sub">
                        <For each={item.children ?? []}>
                          {(child) => (
                            <a
                              class="sub"
                              href={child.hash}
                              classList={{ active: item.activeChild?.() === child.hash }}
                            >
                              {child.label}
                            </a>
                          )}
                        </For>
                      </div>
                    </Show>
                  </>
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
              <Match when={currentRoute() === "#/integrations"}><ExtensionsSection /></Match>
              <Match when={currentRoute() === "#/gateway"}><GatewaySection /></Match>
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
