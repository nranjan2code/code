import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { api, AuthRequired } from "./api";
import { OperationsCenter } from "./OperationsCenter";
import { clock, shortId, timeAgo } from "./time";
import {
  AccessPicker, BUILTIN_TOOLS, MATCHER_TOOLS, MatcherBuilder, ScheduleBuilder,
  describeDuration, describeMatcher, describeSchedule, shortDuration, syncSelect,
} from "./controls";
import type { AccessOption } from "./controls";
import {
  approvalsVersion, authed, conn, connectEvents, disconnectEvents, feed, navigate, pushToast,
  route, sessionsVersion, setAuthed, statsVersion, toasts,
} from "./store";
import type {
  AllowlistEntry, BestOfNRun, Bot, ChannelPolicy, ChatSurfaceStatus, ConfigInfo, CorePoolEntry, DiscoveredModelsResponse,
  FinOpsStatus, FinOpsDailyPoint, FinOpsRollupEntry, HookConfig,
  GatewayBinding, GatewayStatus, InboxEntry, McpServerConfig, MemoryItem, OpsStatus, PendingApproval,
  PermissionMode, ProviderSummary,
  SearchHit, SecurityEvent, SessionCheckpoint, SessionDiff, SessionListItem,
  ActiveSubagent, SkillItem, SkillProposal, TaskItem, TranscriptEntry, VoiceConfig, WorkReceipt,
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
  operations: "M4 6h16M4 12h16M4 18h16 M8 6v12 M16 6v12",
  sessions: "M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM23 21v-2a4 4 0 0 0-3-3.87",
  integrations: "M16.5 9.4 7.55 4.24a1.78 1.78 0 0 0-2.5 1.55v12.42a1.78 1.78 0 0 0 2.5 1.55L16.5 14.6a1.78 1.78 0 0 0 0-3.2z M21 12h-3 M3 12h1",
  gateway: "M4 4h16v12H4z M8 20h8 M12 16v4 M8 8h.01 M12 8h4 M8 12h8",
  memory: "M4 19.5A2.5 2.5 0 0 1 6.5 17H20 M4 4.5A2.5 2.5 0 0 1 6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15z",
  feeds: "M4 11a9 9 0 0 1 9 9M4 4a16 16 0 0 1 16 16M5 21a1 1 0 1 0 0-2 1 1 0 0 0 0 2z",
  search: "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35",
  inbox: "M22 12h-6l-2 3h-4l-2-3H2M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
  security: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z",
  finops: "M12 1v22 M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6",
  settings: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
};

function confirmDestructive(message: string): boolean {
  return window.confirm(`${message}\n\nThis cannot be undone from the admin console.`);
}

function PageHeader(props: { title: string; description: string; actions?: import("solid-js").JSX.Element }) {
  return (
    <header class="page-header">
      <div>
        <h1>{props.title}</h1>
        <p>{props.description}</p>
      </div>
      <Show when={props.actions}>
        <div class="page-actions">{props.actions}</div>
      </Show>
    </header>
  );
}

function LoadError(props: { message?: string; onRetry?: () => void }) {
  return (
    <div class="empty error-state" role="alert">
      <strong>Couldn’t load this section</strong>
      <p>{props.message ?? "The server did not return usable data. Your saved configuration is unchanged."}</p>
      <Show when={props.onRetry}>
        <button class="ghost small" onClick={() => props.onRetry?.()}>Try again</button>
      </Show>
    </div>
  );
}

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
/// A decision keyword said as an outcome. The keyword itself is still the
/// config vocabulary and stays on hover wherever this is used.
const DECISION_WORDS: Record<RuleDecision, string> = {
  allow: "allowed",
  ask: "asks you first",
  deny: "blocked",
};

const DECISION_TONE: Record<RuleDecision, string> = {
  allow: "success",
  ask: "warning",
  deny: "danger",
};

/// A parsed rule, said in a sentence. The raw form stays on hover and in the
/// title — an operator who knows the syntax loses nothing, and one who
/// doesn't no longer has to learn it to read the page.
function describeRule(rule: ParsedRule): string {
  const noun =
    rule.tool === "mcp"
      ? rule.pattern
        ? `the connected app ${rule.pattern.split("/")[0]}`
        : "connected apps"
      : (BUILTIN_TOOLS.find((t) => t.value === rule.tool)?.label ?? rule.tool);
  if (rule.tool !== "mcp" && rule.pattern) return `${noun} matching ${rule.pattern}`;
  return noun;
}

function RuleChip(props: { rule: ParsedRule }) {
  return (
    <span
      class={`chip chip-phrase chip-tone-${DECISION_TONE[props.rule.decision]}`}
      title={`${props.rule.decision}: ${props.rule.raw}`}
    >
      {describeRule(props.rule)}
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
      setError(err instanceof AuthRequired || `${err}`.includes("401") ? "That token doesn\u2019t match. Check the one vak serve printed." : `${err}`);
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
  const [sessions, sessionsActions] = createResource(statsVersion, () => api.sessions());
  const [security] = createResource(statsVersion, () => api.security(500));
  const [finops] = createResource(statsVersion, () => api.finops().catch(() => null));
  const [ops] = createResource(statsVersion, () => api.opsStatus().catch(() => null));
  const [config] = createResource(statsVersion, () => api.config().catch(() => null));
  const [gateway] = createResource(statsVersion, () => api.gatewayStatus().catch(() => null));
  const [approvals] = createResource(approvalsVersion, () => api.approvals());
  const [bestofn] = createResource(statsVersion, () => api.bestofn().catch(() => ({ runs: [], total: 0 })));

  const recentSecurity = createMemo(
    () =>
      (security()?.events ?? []).filter((e) => Date.now() - new Date(e.ts).getTime() < 86_400_000)
        .length,
  );

  const feedItems = createMemo(() => [...feed()].reverse());

  // Today's spend against the day cap — the run cap is a per-turn ceiling,
  // not a running total, so it has no meaningful "progress" to show here.
  const spendUSD = createMemo(() => finops()?.day_usd ?? 0);
  const capUSD = createMemo(() => finops()?.day_cap_usd ?? null);
  const spendProgress = createMemo(() => {
    const cap = capUSD();
    return cap && cap > 0 ? (spendUSD() / cap) * 100 : undefined;
  });

  const recentSessions = createMemo(() =>
    [...(sessions()?.sessions ?? [])].sort((a, b) => new Date(b.last_ts).getTime() - new Date(a.last_ts).getTime()).slice(0, 5),
  );
  const recentWarnings = createMemo(() => health()?.warnings ?? []);
  const attentionCount = createMemo(() =>
    (approvals()?.total ?? 0) + (bestofn()?.total ?? 0) + recentWarnings().length + recentSecurity(),
  );
  const activityCounts = createMemo(() => {
    const counts = new Map<string, number>();
    for (const item of feed()) counts.set(item.event.type, (counts.get(item.event.type) ?? 0) + 1);
    return [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 5);
  });

  const newSession = async () => {
    try {
      const { session_id } = await api.createSession();
      navigate(`#/sessions/${session_id}`);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  const refresh = () => {
    healthActions.refetch();
    sessionsActions.refetch();
    pushToast("info", "Overview refreshed");
  };

  return (
    <div class="view">
      <PageHeader
        title="Overview"
        description="What is running, what needs you, and what changed."
        actions={<>
          <button class="ghost" onClick={refresh}>Refresh</button>
          <button onClick={newSession}>+ New session</button>
        </>}
      />
      <section class="overview-hero">
        <div>
          <div class="hero-kicker"><span class={`dot dot-${conn()}`} /> {conn() === "live" ? "Live telemetry" : `Telemetry ${conn()}`}</div>
          <h2>{attentionCount() === 0 ? "Everything is clear." : `${attentionCount()} thing${attentionCount() === 1 ? "" : "s"} need a look.`}</h2>
          <p>{attentionCount() === 0 ? "Nothing is waiting on you — no approvals, warnings, draft runs, or security events." : "Start with the list below, then read the panels underneath to see what changed."}</p>
        </div>
        <div class="hero-route">
          <span class="hero-route-label">Answering with</span>
          <strong>{providerLabel(config()?.provider ?? health()?.provider ?? "") || "Loading…"}</strong>
          <span class="mono">{config()?.model ?? health()?.model ?? ""}</span>
          <button class="text-action" onClick={() => navigate("#/settings")}>Change this →</button>
        </div>
      </section>
      <div class="stats-row">
        <StatCard label="Sessions" value={sessions()?.total ?? "…"} />
        <StatCard
          label="Messages recorded"
          value={sessions()?.sessions.reduce((a, s) => a + s.entry_count, 0) ?? "…"}
          sub={
            sessions() && sessions()!.total > sessions()!.sessions.length
              ? `Across the ${sessions()!.sessions.length} most recent of ${sessions()!.total} sessions`
              : undefined
          }
        />
        <StatCard
          label="Spent today"
          value={`$${spendUSD().toFixed(4)}`}
          progress={spendProgress()}
          sub={capUSD() ? `of your $${capUSD()!.toFixed(2)} daily budget` : "No daily budget set"}
          tone={spendProgress() && spendProgress()! > 90 ? "warn" : undefined}
        />
        <StatCard
          label="Security events today"
          value={security.loading ? "…" : recentSecurity()}
          tone={recentSecurity() > 0 ? "warn" : undefined}
        />
      </div>

      <section class="attention-panel panel">
        <div class="panel-title-row">
          <div><h2>Needs your attention</h2><p>Anything that can hold work up, or change what vak is allowed to do.</p></div>
          <span class={`chip ${attentionCount() > 0 ? "chip-warn" : "chip-ok"}`}>{attentionCount()} open</span>
        </div>
        <div class="attention-grid">
          <button class="attention-item" onClick={() => navigate("#/inbox")}>
            <span class="attention-icon warning">!</span><span><strong>{approvals()?.total ?? "…"} waiting for your approval</strong><small>Vak paused and asked before doing something — review it in the Inbox</small></span><span class="chev">›</span>
          </button>
          <button class="attention-item" onClick={() => navigate("#/sessions")}>
            <span class="attention-icon info">◆</span><span><strong>{bestofn()?.total ?? "…"} draft attempts</strong><small>Vak tried the same task several ways — pick the one to keep</small></span><span class="chev">›</span>
          </button>
          <button class="attention-item" onClick={() => navigate("#/security")}>
            <span class="attention-icon danger">⌁</span><span><strong>{recentSecurity()} security events</strong><small>Everything worth recording from the last 24 hours</small></span><span class="chev">›</span>
          </button>
        </div>
      </section>

      <ApprovalsCard />

      <div class="two-col">
        <section class="panel">
          <h2>This machine</h2>
          <Show when={!health.loading} fallback={<div class="empty">Loading…</div>}>
            <dl class="kv">
              <dt>Provider</dt>
              <dd>{providerLabel(health()?.provider ?? "")}</dd>
              <dt>Model</dt>
              <dd class="mono">{health()?.model}</dd>
              <dt>What it may do</dt>
              <dd><span class="chip chip-phrase chip-mode">{modeLabel(health()?.permission_mode)}</span></dd>
              <dt>Sandbox</dt>
              <dd>{health()?.sandbox}</dd>
              <dt>Memory per turn</dt>
              <dd>{(health()?.context_window ?? 0).toLocaleString()} tokens</dd>
              <dt>Chat gateway</dt>
              <dd>
                <span class="chip" data-on={ops()?.gateway?.state === "running" || ops()?.gateway_healthy}>
                  {ops()?.gateway?.state ?? (health()?.status === "ok" ? "ready" : "offline")}
                </span>
              </dd>
              <dt>Project folder</dt>
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
          <div class="panel-title-row"><div><h2>What is running</h2><p>Live status of each part of the system.</p></div><button class="ghost small" onClick={() => navigate("#/gateway")}>Open gateway</button></div>
          <div class="posture-list">
            <div><span class={`status-mark ${(health()?.posture ?? (health()?.status === "ok" ? "healthy" : "degraded")) === "healthy" ? "good" : "bad"}`} /> <span>Vak itself</span><strong>{health()?.posture ?? health()?.status ?? "Loading…"}</strong></div>
            <div><span class={`status-mark ${ops()?.gateway_healthy ? "good" : "bad"}`} /> <span>Chat gateway</span><strong>{ops()?.gateway?.state ?? "Unknown"}</strong></div>
            <div><span class={`status-mark ${gateway()?.enabled ? "good" : "neutral"}`} /> <span>Connected chats</span><strong>{gateway()?.bindings.length ?? "…"}</strong></div>
            <div><span class="status-mark neutral" /> <span>What it may do</span><strong>{config()?.permission_mode ?? health()?.permission_mode ? modeLabel(config()?.permission_mode ?? health()?.permission_mode) : "…"}</strong></div>
          </div>
          <Show when={recentWarnings().length > 0}>
            <div class="posture-warning">{recentWarnings()[0]} <button class="text-action" onClick={() => navigate("#/settings")}>Review settings →</button></div>
          </Show>
        </section>

        <section class="panel recent-panel">
          <div class="panel-title-row"><div><h2>Recent sessions</h2><p>The latest conversations, across every project.</p></div><button class="ghost small" onClick={() => navigate("#/sessions")}>View all</button></div>
          <Show when={recentSessions().length > 0} fallback={<div class="empty">No sessions yet.</div>}>
            <div class="recent-sessions">
              <For each={recentSessions()}>{(session) => <button class="recent-session" onClick={() => navigate(`#/sessions/${session.session_id}`)}>
                <span class="session-pulse" /><span class="mono">{shortId(session.session_id)}</span><span class="session-entries">{session.entry_count} messages</span><span class="when">{timeAgo(session.last_ts)}</span>
              </button>}</For>
            </div>
          </Show>
        </section>

        <section class="panel">
          <div class="panel-title-row"><div><h2>What has been happening</h2><p>The mix of events since this page connected.</p></div><span class="chip chip-tone-info">{feed().length} events</span></div>
          <Show when={activityCounts().length > 0} fallback={<div class="empty">Waiting for events…</div>}>
            <div class="activity-report"><For each={activityCounts()}>{([kind, count]) => <div class="activity-row"><span title={kind}>{EVENT_LABELS[kind] ?? kind}</span><div class="activity-track"><i style={{ width: `${Math.max(8, (count / Math.max(1, feed().length)) * 100)}%` }} /></div><strong>{count}</strong></div>}</For></div>
          </Show>
          <button class="text-action report-link" onClick={() => navigate("#/security")}>See every recorded event →</button>
        </section>

        <section class="panel live-panel">
          <div class="panel-title-row"><div><h2>Happening now</h2><p>New events appear here the moment they occur.</p></div><span class="chip chip-tone-success">streaming</span></div>
          <Show
            when={feedItems().length > 0}
            fallback={<div class="empty">Waiting for events… they will appear here in real time.</div>}
          >
            <ul class="feed">
              <For each={feedItems()}>
                {(item) => (
                  <li data-type={item.event.type}>
                    <span class="feed-time">{clock(item.ts)}</span>
                    <span class="feed-kind" title={item.event.type}>{EVENT_LABELS[item.event.type] ?? item.event.type}</span>
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
      pushToast("info", `${approve ? "Allowed" : "Refused"} — ${a.tool}`);
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
      <div class="panel-title-row">
        <div>
          <h2>Waiting for your approval</h2>
          <p class="dim">Vak stopped before doing each of these and is waiting on your answer.</p>
        </div>
      </div>
      <Show
        when={(pending()?.approvals.length ?? 0) > 0}
        fallback={<div class="empty">Nothing is waiting. Vak is getting on with it.</div>}
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

/// `SystemEvent`'s serde tag is a Rust variant name. Print what happened
/// instead, and keep the tag on hover for anyone matching it against a log.
const EVENT_LABELS: Record<string, string> = {
  Agent: "Agent step",
  SessionCreated: "Session started",
  SessionEntryAppended: "Message recorded",
  ConfigChanged: "Setting changed",
  GatewayInbound: "Message from a chat",
  ApprovalRequested: "Approval requested",
  ApprovalGranted: "Approval granted",
  ApprovalDenied: "Approval refused",
  SecurityEvent: "Security event",
  ProviderError: "Provider error",
  RateLimit: "Rate limited",
  Heartbeat: "Still connected",
  Lagged: "Events skipped",
};

function summarizeEvent(ev: import("./types").SystemEvent): string {
  switch (ev.type) {
    case "Agent":
      return ev.data.summary + (ev.data.detail ? ` · ${ev.data.detail}` : "");
    case "SessionCreated":
      return ev.data.session_id.slice(0, 12);
    case "SessionEntryAppended":
      return `${ev.data.kind} in ${ev.data.session_id.slice(0, 12)}`;
    case "ConfigChanged":
      return `${ev.data.label}: ${ev.data.detail}`;
    case "GatewayInbound":
      return `${SURFACE_LABEL[ev.data.surface] ?? ev.data.surface} · ${ev.data.who}: ${ev.data.preview}`;
    case "ApprovalGranted":
    case "ApprovalDenied":
      return `${ev.data.tool} (${ev.data.id.slice(0, 8)})`;
    case "SecurityEvent":
      return `${secKindLabel(ev.data.kind)} — ${ev.data.label}`;
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
  // Needed only for `workspace_project_hash` — which rows this console
  // instance can archive/delete/export, versus read-only across projects.
  const [config] = createResource(() => api.config().catch(() => null));
  const [q, setQ] = createSignal("");
  const [showArchived, setShowArchived] = createSignal(false);
  const [creating, setCreating] = createSignal(false);
  const [busyCandidate, setBusyCandidate] = createSignal("");
  const [busySession, setBusySession] = createSignal("");
  const [bulkBusy, setBulkBusy] = createSignal(false);

  const isLocal = (s: SessionListItem) => s.project_hash === config()?.workspace_project_hash;

  const toggleArchive = async (s: SessionListItem) => {
    setBusySession(s.session_id);
    try {
      await api.archiveSession(s.session_id, !s.archived);
      pushToast("info", s.archived ? "Unarchived" : "Archived");
      refetch();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusySession("");
    }
  };

  const removeSession = async (s: SessionListItem) => {
    if (!confirmDestructive(`Delete session ${shortId(s.session_id)}? The transcript file stays on disk.`)) return;
    setBusySession(s.session_id);
    try {
      await api.deleteSession(s.session_id);
      pushToast("info", "Session deleted");
      refetch();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusySession("");
    }
  };

  const archivedCount = createMemo(() => (sessions()?.sessions ?? []).filter((s) => s.archived).length);

  const deleteAllArchived = async () => {
    if (!confirmDestructive(`Delete all ${archivedCount()} archived sessions? The transcript files stay on disk.`)) return;
    setBulkBusy(true);
    try {
      const res = await api.deleteAllArchived();
      pushToast("info", `Deleted ${res.deleted} archived session${res.deleted === 1 ? "" : "s"}`);
      refetch();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBulkBusy(false);
    }
  };

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
      pushToast("info", "Kept. Those changes are now in your project.");
      bestofnActions.refetch();
      refetch();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusyCandidate("");
    }
  };

  const handleDiscard = async (sessionId: string) => {
    if (!confirmDestructive("Throw this draft away? Its changes are lost.")) return;
    setBusyCandidate(sessionId);
    try {
      await api.discardBestRun(sessionId);
      pushToast("info", "Draft discarded");
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
      (s: SessionListItem) =>
        (!needle || s.session_id.toLowerCase().includes(needle)) && (showArchived() || !s.archived),
    );
  });

  return (
    <div class="view">
      <PageHeader title="Sessions" description="Every conversation vak has had — read one, pick up where it left off, or compare the drafts of a task it tried several ways." />
      {/* Best of N Candidate Runs Card */}
      <Show when={(bestofn()?.runs?.length ?? 0) > 0}>
        <section class="panel" style="margin-bottom:14px">
          <div class="panel-title-row">
            <div>
              <h2>Drafts waiting to be picked ({bestofn()!.runs.length})</h2>
              <p class="dim">
                Vak tried the same task several ways, each in its own copy of the project. Keep the
                one you want and the rest are thrown away.
              </p>
            </div>
          </div>
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
        <input class="search-input" placeholder="Find a session…" value={q()} onInput={(e) => setQ(e.currentTarget.value)} />
        <label class="toggle">
          <input type="checkbox" checked={showArchived()} onChange={(e) => setShowArchived(e.currentTarget.checked)} />
          Show archived ({archivedCount()})
        </label>
        <span class="spacer" />
        <Show when={showArchived() && archivedCount() > 0}>
          <button class="danger small" disabled={bulkBusy()} onClick={() => void deleteAllArchived()}>
            {bulkBusy() ? "Deleting…" : `Delete all archived (${archivedCount()})`}
          </button>
        </Show>
        <button disabled={creating()} onClick={newSession}>
          {creating() ? "Creating…" : "+ New session"}
        </button>
        <button class="ghost" onClick={() => { refetch(); bestofnActions.refetch(); }}>Refresh</button>
      </div>
      <Show when={!sessions.loading} fallback={<div class="empty">Loading sessions…</div>}>
        <Show
          when={!sessions.error}
          fallback={<LoadError message={`${sessions.error}`} onRetry={() => refetch()} />}
        >
        <Show when={sessions() && sessions()!.total > sessions()!.sessions.length}>
          <p class="dim" style="margin:-6px 0 10px">
Showing the {sessions()!.sessions.length} most recent of {sessions()!.total} sessions — search above to
            find an older one.
          </p>
        </Show>
        <Show
          when={filtered().length > 0}
          fallback={<div class="empty">No sessions yet. Start one here, in the desktop app, or by messaging a connected chat — it will show up in this list.</div>}
        >
          <table class="table">
            <thead>
              <tr><th>session</th><th>messages</th><th>started</th><th>last active</th><th>project</th><th /></tr>
            </thead>
            <tbody>
              <For each={filtered()}>
                {(s) => {
                  const local = createMemo(() => isLocal(s));
                  const busy = () => busySession() === s.session_id;
                  return (
                    <tr tabindex="0" classList={{ dim: s.archived }} onClick={() => navigate(`#/sessions/${s.session_id}`)} onKeyDown={(e) => e.key === "Enter" && navigate(`#/sessions/${s.session_id}`)}>
                      <td class="mono">
                        {shortId(s.session_id)}
                        <Show when={s.archived}><span class="chip" style="margin-left:6px">archived</span></Show>
                      </td>
                      <td>{s.entry_count}</td>
                      <td title={s.first_ts}>{timeAgo(s.first_ts)}</td>
                      <td title={s.last_ts}>{timeAgo(s.last_ts)}</td>
                      <td>
                        <Show when={local()} fallback={<span class="dim" title="This session belongs to a different project than the one this console is attached to, so it can only be read from here.">another project</span>}>
                          <span class="chip chip-tone-success">this project</span>
                        </Show>
                      </td>
                      <td onClick={(e) => e.stopPropagation()}>
                        <Show
                          when={local()}
                          fallback={
                            <span class="dim" title="Only sessions in this console's own project can be archived, deleted, or exported here.">
                              —
                            </span>
                          }
                        >
                          <div class="row-gap">
                            <button class="ghost small" disabled={busy()} onClick={() => void toggleArchive(s)}>
                              {busy() ? "…" : s.archived ? "Unarchive" : "Archive"}
                            </button>
                            <a class="ghost small" href={`/sessions/${encodeURIComponent(s.session_id)}/transcript.md`} target="_blank" rel="noreferrer">
                              Export .md
                            </a>
                            <Show when={s.archived}>
                              <button class="danger small" disabled={busy()} onClick={() => void removeSession(s)}>
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
        </Show>
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
  const [subagentData, { refetch: refetchSubagents }] = createResource(
    () => props.sessionId,
    (id) => api.subagents(id),
  );
  const [subagentBusy, setSubagentBusy] = createSignal("");

  createEffect(() => {
    void props.sessionId;
    const timer = window.setInterval(() => refetchSubagents(), 3000);
    onCleanup(() => window.clearInterval(timer));
  });

  const stopChild = async (child: ActiveSubagent) => {
    if (!confirmDestructive(`Stop subagent “${child.label}”? Its ledger will remain available.`)) return;
    setSubagentBusy(child.id);
    try {
      await api.stopSubagent(props.sessionId, child.id);
      pushToast("info", "Subagent stop requested");
      refetchSubagents();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setSubagentBusy("");
    }
  };

  const steerChild = async (child: ActiveSubagent) => {
    const text = window.prompt(`Steer ${child.label}`, "");
    if (!text?.trim()) return;
    setSubagentBusy(child.id);
    try {
      await api.steerSubagent(props.sessionId, child.id, text.trim());
      pushToast("info", "Steering queued");
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setSubagentBusy("");
    }
  };

  // `diff` and `receipts` are served off the live in-memory session handle,
  // not the store index the transcript list itself reads from — a session
  // opened here from the sessions list (rather than created in this tab)
  // has no such handle yet. Attach it first, the same way the desktop
  // client does on every task switch; attach is a no-op when the handle is
  // already live. `checkpoints` reads the on-disk snapshot directory
  // directly and needs no handle.
  const [diffData] = createResource(activeTab, (t) =>
    t === "diff"
      ? api
          .attach(props.sessionId)
          .then(() => api.diff(props.sessionId))
          .catch((err) => ({ diff: `Could not work out what changed: ${err}` }))
      : Promise.resolve(null),
  );
  const [receiptsData] = createResource(activeTab, (t) =>
    t === "receipts"
      ? api
          .attach(props.sessionId)
          .then(() => api.receipts(props.sessionId))
          .catch((err) => {
            pushToast("alert", `Could not load work receipts: ${err}`);
            return [];
          })
      : Promise.resolve(null),
  );
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
      await api.attach(props.sessionId);
      await api.cancelRun(props.sessionId);
      pushToast("info", "Cancellation requested");
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const restoreCommit = async (seq: number) => {
    if (!confirmDestructive(`Put the files back to how they were at save point #${seq}? Later changes in this session are lost.`)) return;
    try {
      // Attaching first lets the server resolve the correct cwd for a
      // best-of-N child (its worktree, not the workspace root) even when
      // this tab never ran anything on it.
      await api.attach(props.sessionId);
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
      <PageHeader title="Session" description="Read the conversation, see what changed on disk, and steer or stop the run." />
      <div class="toolbar">
        <button class="ghost" onClick={() => navigate("#/sessions")}>‹ Sessions</button>
        <span class="mono dim">{shortId(props.sessionId)}</span>
        <span class="spacer" />
        <Show when={running()}>
          <span class="chip chip-running">running</span>
          <button class="danger small" onClick={cancel}>Cancel</button>
        </Show>
        <label class="toggle">
          <input type="checkbox" checked={live()} onChange={(e) => setLive(e.currentTarget.checked)} />
          Follow along
        </label>
      </div>

      {/* Tabs Bar */}
      <div class="tab-bar">
        <button class="tab-btn" classList={{ active: activeTab() === "transcript" }} onClick={() => setActiveTab("transcript")}>
          Transcript ({totalCount()})
        </button>
        <button class="tab-btn" classList={{ active: activeTab() === "diff" }} onClick={() => setActiveTab("diff")}>
          Changed files
        </button>
        <button class="tab-btn" classList={{ active: activeTab() === "receipts" }} onClick={() => setActiveTab("receipts")}>
          Cost &amp; usage
        </button>
        <button class="tab-btn" classList={{ active: activeTab() === "checkpoints" }} onClick={() => setActiveTab("checkpoints")}>
          Checkpoints
        </button>
      </div>

      <Switch>
        {/* Transcript Tab */}
        <Match when={activeTab() === "transcript"}>
          <Show when={(subagentData()?.subagents?.length ?? 0) > 0}>
            <section class="panel" style="margin-bottom:12px">
              <div class="panel-title-row">
                <div><h2>Live subagents</h2><p class="dim">Child sessions share this workspace’s memory policy; their ledgers remain after they finish.</p></div>
                <button class="ghost small" onClick={() => refetchSubagents()}>Refresh</button>
              </div>
              <table class="table">
                <thead><tr><th>label</th><th>child session</th><th>elapsed</th><th /></tr></thead>
                <tbody>
                  <For each={subagentData()?.subagents ?? []}>
                    {(child) => <tr>
                      <td>{child.label}</td>
                      <td class="mono dim">{shortId(child.id)}</td>
                      <td>{child.elapsed_secs}s</td>
                      <td>
                        <div class="row-gap">
                          <button class="ghost small" disabled={subagentBusy() === child.id} onClick={() => void steerChild(child)}>Steer</button>
                          <button class="danger small" disabled={subagentBusy() === child.id} onClick={() => void stopChild(child)}>Stop</button>
                        </div>
                      </td>
                    </tr>}
                  </For>
                </tbody>
              </table>
            </section>
          </Show>
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
              fallback={<div class="empty">Nothing matches this filter.</div>}
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
                <span class="dim">Page {Math.floor(offset() / PAGE) + 1}</span>
                <button class="ghost small" disabled={!hasMore()} onClick={() => changePage(1)}>Older ›</button>
              </div>
            </Show>
          </Show>

          <div class="composer">
            <select
              class="n-stepper"
              title="How many attempts to make. 1 answers once; 2\u20134 try the same task separately, in their own copies of the project, so you can pick the best."
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
              placeholder={running() ? "Add a note to the run in progress…" : nCandidates() >= 2 ? `Try this ${nCandidates()} separate ways…` : "Ask vak to do something…"}
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
          <Show when={!diffData.loading} fallback={<div class="empty">Working out what changed…</div>}>
            <div class="diff-box">
              <Show when={diffData()?.diff} fallback="Nothing has been changed on disk in this session.">
                {diffData()!.diff}
              </Show>
            </div>
          </Show>
        </Match>

        {/* Work Receipts Tab */}
        <Match when={activeTab() === "receipts"}>
          <Show when={!receiptsData.loading} fallback={<div class="empty">Loading…</div>}>
            <Show when={(receiptsData()?.length ?? 0) > 0} fallback={<div class="empty">Nothing has been billed to this session yet.</div>}>
              <table class="table">
                <thead>
                  <tr><th>step</th><th>provider</th><th>model</th><th>sent</th><th>received</th><th>cost</th><th>took</th></tr>
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
            <Show when={(checkpointsData()?.checkpoints?.length ?? 0) > 0} fallback={<div class="empty">No save points yet. Vak records one each time it finishes a piece of work.</div>}>
              <table class="table">
                <thead>
                  <tr><th>#</th><th>when</th><th>commit</th><th>what changed</th><th /></tr>
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
  { hash: "#/integrations", label: "Connected apps" },
  { hash: "#/integrations/plugins", label: "Plugins" },
  { hash: "#/integrations/skills", label: "Skills" },
  { hash: "#/integrations/hooks", label: "Automations" },
  { hash: "#/integrations/tasks", label: "Scheduled tasks" },
] as const;

function extensionsTab(): string {
  const r = route();
  return EXTENSION_TABS.slice(1).find((t) => r.startsWith(t.hash))?.hash ?? "#/integrations";
}

interface ExtensionsCtx {
  plugins: () => import("./types").PluginItem[];
  pluginsLoading: () => boolean;
  refetchPlugins: () => void;
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
    <span class="chip chip-phrase" title="This version of the server does not report its permission rules.">
      not reported
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
      title={`No rule covers this, so the "${modeLabel(props.mode)}" setting decides: ${decision()}`}
    >
      {DECISION_WORDS[decision()]} by default
    </span>
  );
}

// ---- Extensions › MCP servers ----------------------------------------------

/** `KEY=VALUE` per line, the same shape a `.env` file uses — matches how
 * these values are described elsewhere ("Names live in .vak/config.toml").
 * Blank lines and lines without `=` are ignored rather than rejected, so a
 * stray trailing newline never blocks a save. */
function parseEnvLines(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const idx = line.indexOf("=");
    if (idx <= 0) continue;
    const key = line.slice(0, idx).trim();
    if (key) out[key] = line.slice(idx + 1).trim();
  }
  return out;
}
function envToLines(env: Record<string, string> | undefined): string {
  return Object.entries(env ?? {})
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
}

/// A short, opinionated list of servers most operators actually want, so the
/// common case is a click rather than a remembered `npx` invocation. Picking
/// one only fills the form in — nothing is written until Connect is pressed,
/// and every field stays editable. "Something else" clears it back to blank.
const MCP_CATALOG: {
  id: string;
  label: string;
  blurb: string;
  name: string;
  command: string;
  args: string;
  network: boolean;
  env: string;
}[] = [
  {
    id: "github",
    label: "GitHub",
    blurb: "Issues, pull requests, code search",
    name: "github",
    command: "npx",
    args: "-y @modelcontextprotocol/server-github",
    network: true,
    env: "GITHUB_PERSONAL_ACCESS_TOKEN=${GITHUB_PERSONAL_ACCESS_TOKEN}",
  },
  {
    id: "filesystem",
    label: "Files",
    blurb: "Read and write a folder outside the project",
    name: "filesystem",
    command: "npx",
    args: "-y @modelcontextprotocol/server-filesystem .",
    network: false,
    env: "",
  },
  {
    id: "postgres",
    label: "Postgres",
    blurb: "Query a database read-only",
    name: "postgres",
    command: "npx",
    args: "-y @modelcontextprotocol/server-postgres",
    network: true,
    env: "DATABASE_URL=${DATABASE_URL}",
  },
  {
    id: "fetch",
    label: "Fetch a page",
    blurb: "Pull a URL down as readable text",
    name: "fetch",
    command: "uvx",
    args: "mcp-server-fetch",
    network: true,
    env: "",
  },
  {
    id: "custom",
    label: "Something else",
    blurb: "Fill the details in yourself",
    name: "",
    command: "",
    args: "",
    network: false,
    env: "",
  },
];

function McpServersView(props: { ctx: ExtensionsCtx }) {
  const [expanded, setExpanded] = createSignal("");
  const [editing, setEditing] = createSignal("");
  const [editCmd, setEditCmd] = createSignal("");
  const [editArgs, setEditArgs] = createSignal("");
  const [editNetwork, setEditNetwork] = createSignal(false);
  const [editEnv, setEditEnv] = createSignal("");
  const [newName, setNewName] = createSignal("");
  const [newCmd, setNewCmd] = createSignal("");
  const [newArgs, setNewArgs] = createSignal("");
  const [newNetwork, setNewNetwork] = createSignal(false);
  const [newEnv, setNewEnv] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [tavilyKey, setTavilyKey] = createSignal("");
  const [tavilyBusy, setTavilyBusy] = createSignal(false);
  const [tavily, { refetch: refetchTavily }] = createResource(() => api.tavily().catch(() => null));

  const [catalogPick, setCatalogPick] = createSignal("");

  /// Prefill the form from a catalog entry rather than send it: the operator
  /// still sees, and can change, exactly what will be written.
  const applyCatalog = (id: string) => {
    const entry = MCP_CATALOG.find((e) => e.id === id);
    setCatalogPick(id);
    if (!entry || !entry.command) {
      setNewName("");
      setNewCmd("");
      setNewArgs("");
      setNewNetwork(false);
      setNewEnv("");
      return;
    }
    setNewName(entry.name);
    setNewCmd(entry.command);
    setNewArgs(entry.args);
    setNewNetwork(entry.network);
    setNewEnv(entry.env);
  };

  const servers = createMemo(() => Object.entries(props.ctx.mcp()) as [string, McpServerConfig][]);

  const addServer = async () => {
    const name = newName().trim();
    const cmd = newCmd().trim();
    if (!name || !cmd || busy()) return;
    setBusy(true);
    try {
      const env = parseEnvLines(newEnv());
      await api.putMcpServers({
        ...props.ctx.mcp(),
        [name]: {
          command: cmd,
          args: newArgs().trim() ? newArgs().trim().split(/\s+/) : [],
          network: newNetwork(),
          ...(Object.keys(env).length > 0 ? { env } : {}),
        },
      });
      pushToast("info", `Connected ‘${name}’`);
      setNewName("");
      setNewCmd("");
      setNewArgs("");
      setNewNetwork(false);
      setNewEnv("");
      props.ctx.refetchMcp();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const removeServer = async (name: string) => {
    if (!confirmDestructive(`Disconnect “${name}”? Vak stops using its tools.`)) return;
    const next = { ...props.ctx.mcp() };
    delete next[name];
    try {
      await api.putMcpServers(next);
      pushToast("info", `Disconnected ‘${name}’`);
      props.ctx.refetchMcp();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  const beginEdit = (name: string, server: McpServerConfig) => {
    setEditing(name);
    setEditCmd(server.command);
    setEditArgs((server.args ?? []).join(" "));
    setEditNetwork(!!server.network);
    setEditEnv(envToLines(server.env));
  };

  const saveEdit = async (name: string) => {
    if (!editCmd().trim() || busy()) return;
    setBusy(true);
    try {
      const env = parseEnvLines(editEnv());
      await api.putMcpServers({
        ...props.ctx.mcp(),
        [name]: {
          command: editCmd().trim(),
          args: editArgs().trim() ? editArgs().trim().split(/\s+/) : [],
          network: editNetwork(),
          ...(Object.keys(env).length > 0 ? { env } : {}),
        },
      });
      pushToast("info", `Updated ‘${name}’`);
      setEditing("");
      props.ctx.refetchMcp();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const saveTavily = async () => {
    if (!tavilyKey().trim() || tavilyBusy()) return;
    setTavilyBusy(true);
    try {
      await api.enableTavily(tavilyKey());
      setTavilyKey("");
      pushToast("info", "Tavily enabled — the key is stored securely and never displayed");
      refetchTavily();
      props.ctx.refetchMcp();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setTavilyBusy(false);
    }
  };

  const disableTavily = async () => {
    if (tavilyBusy()) return;
    if (!confirmDestructive("Disable Tavily web search?")) return;
    setTavilyBusy(true);
    try {
      await api.disableTavily();
      pushToast("info", "Tavily disabled; its stored key was retained");
      refetchTavily();
      props.ctx.refetchMcp();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setTavilyBusy(false);
    }
  };

  return (
    <>
      <div class="toolbar">
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refetchMcp()}>Refresh</button>
      </div>

      <section class="panel" style="margin-bottom:14px">
        <div class="panel-title-row">
          <div>
            <h2>Tavily web search</h2>
            <p class="dim">
              Add your Tavily key here. Vak stores it in the protected user secret store, passes it
              only to <code>tavily-mcp</code>, and enables outbound network access automatically.
            </p>
          </div>
          <Show when={tavily()?.enabled}>
            <span class="chip chip-tone-success">enabled</span>
          </Show>
        </div>
        <Show when={tavily()?.enabled} fallback={
          <div class="form-row">
            <label for="tavily-key">API key</label>
            <input
              id="tavily-key"
              type="password"
              autocomplete="off"
              placeholder="tvly-…"
              value={tavilyKey()}
              onInput={(e) => setTavilyKey(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && void saveTavily()}
            />
            <button disabled={tavilyBusy() || !tavilyKey().trim()} onClick={() => void saveTavily()}>
              {tavilyBusy() ? "Saving…" : "Save & enable"}
            </button>
          </div>
        }>
          <div class="binding-meta">
            Key present · network enabled · configured as <code>tavily-mcp</code>
          </div>
          <div class="row-gap" style="margin-top:10px">
            <button class="danger small" disabled={tavilyBusy()} onClick={() => void disableTavily()}>
              {tavilyBusy() ? "Disabling…" : "Disable Tavily"}
            </button>
          </div>
        </Show>
      </section>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Connected apps</h2>
            <p class="dim">
              Each row is a program vak starts when it needs the tools that app provides. What it is
              then allowed to do comes from the rules under{" "}
              <a href="#/settings">Settings</a>, and where no rule reaches it, from the setting that
              decides everything else.
            </p>
            <p class="dim">
              This lists only the apps connected for this project. One connected for your whole
              account is still available here, but isn’t shown, so connecting or editing an app here
              can never quietly pin this project to an old copy of it.
            </p>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.mcpLoading()}>
            <table class="table">
              <thead>
                <tr><th>app</th><th>starts with</th><th>internet</th><th>secrets</th><th>rules that apply</th><th /></tr>
              </thead>
              <tbody><SkeletonRows cols={6} /></tbody>
            </table>
          </Match>

          <Match when={servers().length === 0}>
            <div class="empty empty-teach">
              <strong>No apps are connected yet.</strong>
              <p>
A connected app hands vak extra tools — a GitHub client, a database, a browser. Connect one
                below and it starts up the first time something actually uses it; until then it costs
                you nothing.
              </p>
            </div>
          </Match>

          <Match when={servers().length > 0}>
            <table class="table">
              <thead>
                <tr><th>app</th><th>starts with</th><th>internet</th><th>secrets</th><th>rules that apply</th><th /></tr>
              </thead>
              <tbody>
                <For each={servers()}>
                  {([name, server]) => {
                    const matching = createMemo(() => rulesForMcpServer(props.ctx.rules(), name));
                    const envKeys = () => Object.keys(server.env ?? {});
                    const open = () => expanded() === name;
                    return (
                      <>
                        <tr tabindex="0" classList={{ "row-open": open() }} onClick={() => setExpanded(open() ? "" : name)} onKeyDown={(e) => e.key === "Enter" && setExpanded(open() ? "" : name)}>
                          <td class="mono bold">{name}</td>
                          <td class="dim col-command">
                            <span class="path" title={`${server.command} ${(server.args ?? []).join(" ")}`}>
                              {server.command} {(server.args ?? []).join(" ")}
                            </span>
                          </td>
                          <td>
                            <span class={`chip chip-tone-${server.network ? "warning" : "success"}`}>
                              {server.network ? "can reach the internet" : "no internet"}
                            </span>
                          </td>
                          <td class="dim">
                            {envKeys().length > 0 ? `${envKeys().length} secret${envKeys().length === 1 ? "" : "s"}` : "—"}
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
                                  <span class="eyebrow">What gets run</span>
                                  <pre class="mono detail-pre">{server.command} {(server.args ?? []).join(" ")}</pre>
                                </div>
                                <div>
                                  <span class="eyebrow">Secrets passed in</span>
                                  <Show
                                    when={envKeys().length > 0}
                                    fallback={<p class="dim">Nothing beyond vak's own environment.</p>}
                                  >
                                    <div class="chip-stack">
                                      <For each={envKeys()}>{(k) => <span class="chip mono">{k}</span>}</For>
                                    </div>
                                    <p class="dim">
                                      Stored in <code>.vak/config.toml</code> — usually a{" "}
                                      <code>{"${VAR}"}</code> reference resolved from <code>.env</code> at
                                      launch rather than a literal secret, but whatever is written there is
                                      what "Edit command" below will show and can change.
                                    </p>
                                  </Show>
                                </div>
                                <div>
                                  <span class="eyebrow">What it is allowed to do</span>
                                  <Show when={!props.ctx.rulesKnown()}>
                                    <p class="dim">
This version of the server doesn’t report its rules, so what governs calls
                                      to <code>{name}</code> can’t be shown here. Open the project’s{" "}
                                      <code>config.toml</code> to read them, or update vak.
                                    </p>
                                  </Show>
                                  <Show when={props.ctx.rulesKnown()}>
                                  <Show
                                    when={matching().length > 0}
                                    fallback={
                                      <p class="dim">
No rule mentions this app, so every call to it is decided by the
                                        setting under Settings — “{modeLabel(props.ctx.mode())}”, which means{" "}
                                        <strong>{DECISION_WORDS[modeDefaultDecision(props.ctx.mode())]}</strong>.
                                        Add an <code>mcp({name}/*)</code> rule to the config file to
                                        change that for this app alone.
                                      </p>
                                    }
                                  >
                                    <div class="chip-stack">
                                      <For each={matching()}>{(r) => <RuleChip rule={r} />}</For>
                                    </div>
                                    <p class="dim">
When several rules match the same call, the strictest one wins — blocked beats
                                      ask beats allowed, and the order they appear in never matters. A call none
                                      of these cover falls back to “{modeLabel(props.ctx.mode())}”, which means{" "}
                                      <strong>{DECISION_WORDS[modeDefaultDecision(props.ctx.mode())]}</strong>.
                                    </p>
                                  </Show>
                                  </Show>
                                </div>
                              </div>
                              <div class="row-gap" style="margin-top:12px">
                                <Show when={editing() !== name}>
                                  <button class="ghost small" onClick={() => beginEdit(name, server)}>Edit</button>
                                </Show>
                                <button class="danger small" onClick={() => removeServer(name)}>
                                  Disconnect
                                </button>
                              </div>
                              <Show when={editing() === name}>
                                <div class="edit-form compact-edit">
                                  <div class="form-row"><label>Program to run</label><input class="mono" value={editCmd()} onInput={(e) => setEditCmd(e.currentTarget.value)} /></div>
                                  <div class="form-row"><label>Arguments</label><input class="mono" value={editArgs()} onInput={(e) => setEditArgs(e.currentTarget.value)} /></div>
                                  <label class="inherit-toggle">
                                    <input type="checkbox" checked={editNetwork()} onChange={(e) => setEditNetwork(e.currentTarget.checked)} />
                                    This app needs internet access
                                  </label>
                                  <div class="form-row">
                                    <label>Secrets it needs</label>
                                    <textarea class="mono" rows={3} placeholder={"KEY=value, one per line"} value={editEnv()} onInput={(e) => setEditEnv(e.currentTarget.value)} />
                                  </div>
                                  <div class="row-gap">
                                    <button disabled={busy() || !editCmd().trim()} onClick={() => void saveEdit(name)}>Save changes</button>
                                    <button class="ghost small" onClick={() => setEditing("")}>Cancel</button>
                                  </div>
                                </div>
                              </Show>
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
          <summary>Connect an app</summary>
          <p class="dim">
            Pick one of the common ones to fill the details in for you, or set one up by hand.
          </p>
          <div class="pick-grid catalog-grid">
            <For each={MCP_CATALOG}>
              {(entry) => (
                <button
                  type="button"
                  class="catalog-card"
                  classList={{ active: catalogPick() === entry.id }}
                  onClick={() => applyCatalog(entry.id)}
                >
                  <strong>{entry.label}</strong>
                  <em>{entry.blurb}</em>
                </button>
              )}
            </For>
          </div>
          <div class="form-row">
            <label>Name</label>
            <input placeholder="github, filesystem, postgres…" value={newName()} onInput={(e) => setNewName(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>Program to run</label>
            <input class="mono" placeholder="npx, uvx, python3…" value={newCmd()} onInput={(e) => setNewCmd(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>Arguments</label>
            <input class="mono" placeholder="-y @modelcontextprotocol/server-github" value={newArgs()} onInput={(e) => setNewArgs(e.currentTarget.value)} />
          </div>
          <label class="inherit-toggle">
            <input type="checkbox" checked={newNetwork()} onChange={(e) => setNewNetwork(e.currentTarget.checked)} />
            This app needs internet access
          </label>
          <div class="form-row">
            <label>Secrets it needs</label>
            <textarea class="mono" rows={3} placeholder={"GITHUB_TOKEN=${GITHUB_TOKEN}, one per line"} value={newEnv()} onInput={(e) => setNewEnv(e.currentTarget.value)} />
          </div>
          <p class="dim">
            One <code>NAME=value</code> per line. Write <code>{"${NAME}"}</code> as the value to
            pass a variable already in your environment through, instead of pasting the secret here.
          </p>
          <div class="row-gap" style="margin-top:12px">
            <button disabled={busy() || !newName().trim() || !newCmd().trim()} onClick={() => void addServer()}>
              {busy() ? "Connecting…" : "Connect"}
            </button>
          </div>
        </details>
      </section>
    </>
  );
}

function PluginsView(props: { ctx: ExtensionsCtx }) {
  const [path, setPath] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [sourcePath, setSourcePath] = createSignal("");
  const [sourceLabel, setSourceLabel] = createSignal("");
  const [keyId, setKeyId] = createSignal("");
  const [publicKey, setPublicKey] = createSignal("");
  const [signature, setSignature] = createSignal("");
  const [catalogQuery, setCatalogQuery] = createSignal("");
  const [sources, { refetch: refetchSources }] = createResource(() => api.pluginSources().catch(() => ({ sources: [] })));
  const [catalog, { refetch: refetchCatalog }] = createResource(catalogQuery, (query) => api.pluginCatalog(query).catch(() => ({ entries: [], errors: [] })));
  const refresh = props.ctx.refetchPlugins;
  const install = async (update: boolean) => {
    const value = path().trim();
    if (!value || busy()) return;
    setBusy(true);
    try {
      if (update) await api.pluginUpdate(value);
      else await api.pluginInstall(value);
      pushToast("info", update ? "Plugin generation staged" : "Plugin installed disabled");
      setPath("");
      refresh();
      void refetchSources();
      void refetchCatalog();
    } catch (error) {
      pushToast("alert", `${error}`);
    } finally {
      setBusy(false);
    }
  };
  const action = async (name: string, operation: "enable" | "disable" | "rollback" | "remove") => {
    if (busy()) return;
    setBusy(true);
    try {
      await api.pluginAction(name, operation);
      pushToast("info", `${name} ${operation}d`);
      refresh();
    } catch (error) {
      pushToast("alert", `${error}`);
    } finally {
      setBusy(false);
    }
  };
  return <section class="stack">
    <div class="panel">
      <div class="panel-title-row"><div><h2>Install a reviewed package</h2><p>Packages are inspected, content-addressed, and installed disabled until you enable them.</p></div></div>
      <div class="form-row"><label>Local package directory</label><input class="mono" placeholder="/path/to/plugin" value={path()} onInput={(e) => setPath(e.currentTarget.value)} /></div>
      <div class="row-gap"><button disabled={busy() || !path().trim()} onClick={() => void install(false)}>Install disabled</button><button class="ghost" disabled={busy() || !path().trim()} onClick={() => void install(true)}>Stage update</button></div>
    </div>
    <div class="panel">
      <div class="panel-title-row"><div><h2>Catalog sources</h2><p>Register a local marketplace snapshot for review. Sources start disabled and are revalidated before activation.</p></div></div>
      <div class="form-row"><label>Catalog directory<input class="mono" value={sourcePath()} onInput={(e) => setSourcePath(e.currentTarget.value)} placeholder="/path/to/catalog" /></label></div>
      <div class="form-row"><label>Label<input value={sourceLabel()} onInput={(e) => setSourceLabel(e.currentTarget.value)} placeholder="Team catalog" /></label></div>
      <details class="advanced"><summary>Detached Ed25519 evidence (optional)</summary><div class="form-row"><label>Key ID<input class="mono" value={keyId()} onInput={(e) => setKeyId(e.currentTarget.value)} /></label><label>Public key (base64)<input class="mono" value={publicKey()} onInput={(e) => setPublicKey(e.currentTarget.value)} /></label><label>Signature (base64)<input class="mono" value={signature()} onInput={(e) => setSignature(e.currentTarget.value)} /></label></div></details>
      <button disabled={busy() || !sourcePath().trim()} onClick={async () => { setBusy(true); try { const signed = keyId() && publicKey() && signature() ? { key_id: keyId(), public_key: publicKey(), signature: signature() } : undefined; await api.pluginRegisterSource(sourcePath(), sourceLabel() || "Local catalog", signed); setSourcePath(""); setSourceLabel(""); setKeyId(""); setPublicKey(""); setSignature(""); void refetchSources(); pushToast("info", "Catalog source registered disabled"); } catch (error) { pushToast("alert", `${error}`); } finally { setBusy(false); } }}>Register source</button>
      <Show when={(sources()?.sources ?? []).length > 0}><div class="capability-list" style={{ "margin-top": "12px" }}><For each={sources()?.sources ?? []}>{(source) => <div class="capability-item"><div class="panel-title-row"><span><strong>{source.label}</strong><small>{source.format} · {source.trust} · {source.enabled ? "Enabled" : "Disabled"} · {source.signature ? (source.signature.verified ? "Signed" : "Signature invalid") : "Unsigned"}</small></span><code title={source.catalog_digest}>sha256:{source.catalog_digest.slice(0, 12)}</code></div><code>{source.trace_id}</code><div class="settings-actions"><button class="settings-button" disabled={busy()} onClick={async () => { setBusy(true); try { await api.pluginSourceAction(source.id, source.enabled ? "disable" : "enable"); void refetchSources(); } catch (error) { pushToast("alert", `${error}`); } finally { setBusy(false); } }}>{source.enabled ? "Disable source" : "Enable source"}</button><Show when={source.signature}><button class="settings-button danger" disabled={busy()} onClick={async () => { setBusy(true); try { await api.pluginKeyAction(source.signature!.key_id, source.signature!.revoked ? "restore" : "revoke"); void refetchSources(); } finally { setBusy(false); } }}>{source.signature!.revoked ? "Restore key" : "Revoke key"}</button></Show></div></div>}</For></div></Show>
      <div class="form-row"><label>Search catalog entries<input value={catalogQuery()} onInput={(e) => setCatalogQuery(e.currentTarget.value)} placeholder="frontend, testing, release…" /></label></div>
      <Show when={(catalog()?.entries ?? []).length > 0} fallback={<p class="dim">No catalog entries match yet. Enable a verified source only after reviewing it.</p>}>
        <div class="capability-list"><For each={catalog()?.entries ?? []}>{(entry) => <div class="capability-item"><div class="panel-title-row"><span><strong>{entry.name}</strong><small>{entry.source_label} · {entry.source_enabled ? "Source enabled" : "Source disabled"}{entry.version ? ` · v${entry.version}` : ""}</small></span><code title={entry.catalog_digest}>sha256:{entry.catalog_digest.slice(0, 12)}</code></div><p>{entry.description || "No description"}</p><small>{entry.license ? `License: ${entry.license}` : "License: not declared"}</small></div>}</For></div>
      </Show>
    </div>
    <Show when={!props.ctx.pluginsLoading()} fallback={<div class="panel"><div class="spin" /></div>}>
      <For each={props.ctx.plugins()} fallback={<div class="panel empty"><h2>No installed plugins</h2><p>Install a local package after reviewing its publisher, license, capabilities, and digest.</p></div>}>
        {(plugin) => <article class="panel">
          <div class="panel-title-row"><div><h2>{plugin.name}</h2><p>{plugin.description || "No description"}</p></div><span class={`chip ${plugin.enabled ? "chip-tone-allow" : "chip-tone-ask"}`}>{plugin.enabled ? "Enabled" : "Disabled"}</span></div>
          <div class="meta-grid"><span>v{plugin.version}</span><span>{plugin.scope}</span><span>{plugin.format}</span><span class="mono" title={plugin.digest}>sha256:{plugin.digest.slice(0, 12)}</span></div>
          <p class="dim mono" title={plugin.trace_id}>trace {plugin.trace_id}</p>
          <div class="row-gap"><button onClick={() => void action(plugin.name, plugin.enabled ? "disable" : "enable")}>{plugin.enabled ? "Disable" : "Enable"}</button><button class="ghost" onClick={() => void action(plugin.name, "rollback")}>Rollback</button><button class="danger" onClick={() => void action(plugin.name, "remove")}>Remove</button></div>
        </article>}
      </For>
    </Show>
  </section>;
}

// ---- Extensions › Skills ---------------------------------------------------

function SkillsView(props: { ctx: ExtensionsCtx }) {
  const [busyId, setBusyId] = createSignal("");
  const [query, setQuery] = createSignal("");
  const visibleSkills = createMemo(() => {
    const needle = query().trim().toLowerCase();
    return props.ctx.skills().filter((skill) => !needle || skill.name.toLowerCase().includes(needle) || (skill.description ?? "").toLowerCase().includes(needle));
  });

  const act = async (id: string, promote: boolean) => {
    if (!promote && !confirmDestructive("Reject this skill proposal?")) return;
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
        <input class="search-input" aria-label="Filter skills" placeholder="Filter skills…" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} />
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
              <thead><tr><th>skill</th><th>available in</th><th>what it does</th><th>file</th></tr></thead>
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
              <thead><tr><th>skill</th><th>available in</th><th>what it does</th><th>file</th></tr></thead>
              <tbody>
              <For each={visibleSkills()}>
                  {(s) => (
                    <tr class="row-static">
                      <td class="mono bold">{s.name}{s.shadowed ? <span class="chip chip-tone-ask" style={{ "margin-left": "6px" }}>shadowed</span> : null}</td>
                      <td>
                        <span class={`chip ${s.scope === "workspace" ? "chip-tool" : "chip-mode"}`} title={s.scope ?? "user"}>
                          {s.scope === "workspace" ? "this project" : "every project"}
                        </span>
                      </td>
                      <td class="dim">{s.description || "No description written for this skill."}</td>
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

/// The same four events, said the way an operator thinks about them. The
/// wire value never changes — only what the console prints.
const HOOK_EVENT_LABELS: Record<string, string> = {
  pre_tool_use: "Before the agent uses a tool",
  post_tool_use: "After the agent uses a tool",
  session_start: "When a session starts",
  stop: "When a run finishes",
};

/// Milliseconds are the wire unit; seconds are the unit an operator has an
/// intuition for. The stored value is untouched for anything not on the list.
function TimeoutPicker(props: { value: string; onChange: (ms: string) => void }) {
  const CHOICES = ["1000", "5000", "10000", "30000", "60000"];
  const label = (ms: string) => `${Math.round(parseInt(ms, 10) / 100) / 10} seconds`;
  return (
    <select value={props.value} onChange={(e) => props.onChange(e.currentTarget.value)}>
      <For each={CHOICES.includes(props.value) ? CHOICES : [props.value, ...CHOICES]}>
        {(ms) => <option value={ms}>{label(ms)}</option>}
      </For>
    </select>
  );
}

function HooksView(props: { ctx: ExtensionsCtx }) {
  const [event, setEvent] = createSignal<string>("pre_tool_use");
  const [matcher, setMatcher] = createSignal("");
  const [command, setCommand] = createSignal("");
  const [timeout, setTimeoutMs] = createSignal("10000");
  const [failureMode, setFailureMode] = createSignal<"open" | "closed">("open");
  const [busy, setBusy] = createSignal(false);
  const [busyIndex, setBusyIndex] = createSignal(-1);
  const [editingIndex, setEditingIndex] = createSignal(-1);
  const [editEvent, setEditEvent] = createSignal<string>("pre_tool_use");
  const [editMatcher, setEditMatcher] = createSignal("");
  const [editCommand, setEditCommand] = createSignal("");
  const [editTimeout, setEditTimeout] = createSignal("10000");
  const [editFailureMode, setEditFailureMode] = createSignal<"open" | "closed">("open");

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
          failure_mode: failureMode(),
        },
      ]);
      pushToast("info", `Added — runs ${(HOOK_EVENT_LABELS[event()] ?? event()).toLowerCase()}`);
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
    if (!confirmDestructive("Remove this automation? The script itself stays on disk.")) return;
    const next = [...props.ctx.hooks()];
    next.splice(index, 1);
    try {
      await api.putHooks(next);
      pushToast("info", "Automation removed");
      props.ctx.refetchHooks();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  // `PUT /config/hooks` replaces the whole list — there is no per-hook
  // endpoint — so a toggle, an edit, and a delete are all "patch this one
  // element, write back the array."
  const replaceHook = async (index: number, patch: Partial<HookConfig>, ok: string) => {
    setBusyIndex(index);
    const next = [...props.ctx.hooks()];
    const current = next[index];
    if (!current) {
      setBusyIndex(-1);
      return;
    }
    next[index] = { ...current, ...patch };
    try {
      await api.putHooks(next);
      pushToast("info", ok);
      props.ctx.refetchHooks();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusyIndex(-1);
    }
  };

  const beginEdit = (index: number, hook: HookConfig) => {
    // Fields first: `<Show ... keyed>` mounts the form the moment the index
    // is set, and MatcherBuilder reads its value at construction.
    setEditEvent(hook.event);
    setEditMatcher(hook.matcher ?? "");
    setEditCommand(hook.command);
    setEditTimeout(String(hook.timeout_ms));
    setEditFailureMode(hook.failure_mode ?? "open");
    setEditingIndex(index);
  };

  const saveEdit = async () => {
    const index = editingIndex();
    const cmd = editCommand().trim();
    if (index < 0 || !cmd) return;
    await replaceHook(
      index,
      {
        event: editEvent(),
        matcher: editMatcher().trim() || null,
        command: cmd,
        timeout_ms: parseInt(editTimeout(), 10) || 10_000,
        failure_mode: editFailureMode(),
      },
      "Automation updated",
    );
    setEditingIndex(-1);
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
            <h2>Automations</h2>
            <p class="dim">
              An automation runs one of your own scripts at a point in the turn. Treat these as
              powerful: a script that runs before a tool can stop that tool outright, and every one
              of them runs as vak itself — outside the sandbox, and outside every permission rule on
              this page. Only what you choose under “Runs on” narrows when one fires.
            </p>
            <p class="dim">
              This lists only the automations set up for this project. One set up for your whole
              account still runs here, but isn’t shown or editable from this page, so saving here
              can never quietly copy it into this project.
            </p>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.hooksLoading()}>
            <table class="table">
              <thead><tr><th>on</th><th>when</th><th>fires on</th><th>runs</th><th>give up after</th><th /></tr></thead>
              <tbody><SkeletonRows cols={6} /></tbody>
            </table>
          </Match>

          <Match when={props.ctx.hooks().length === 0}>
            <div class="empty empty-teach">
              <strong>No automations are set up.</strong>
              <p>
Nothing runs alongside your turns. Add one to keep a record of what vak does, to stop a
                particular command before it runs, or to give every new session some standing context.
              </p>
            </div>
          </Match>

          <Match when={props.ctx.hooks().length > 0}>
            <table class="table">
              <thead><tr><th>on</th><th>when</th><th>fires on</th><th>runs</th><th>give up after</th><th /></tr></thead>
              <tbody>
                <For each={props.ctx.hooks()}>
                  {(hook, index) => {
                    const scope = createMemo(() =>
                      hook.matcher ? parseRule(hook.matcher, "allow") : null,
                    );
                    const isBusy = () => busyIndex() === index();
                    return (
                      <tr class="row-static" classList={{ dim: !hook.enabled }}>
                        <td>
                          <label class="toggle" title={hook.enabled ? "Turn it off without removing it" : "Turn it back on"}>
                            <input
                              type="checkbox"
                              checked={hook.enabled}
                              disabled={isBusy()}
                              onChange={(e) =>
                                void replaceHook(
                                  index(),
                                  { enabled: e.currentTarget.checked },
                                  e.currentTarget.checked ? "Automation on" : "Automation off",
                                )
                              }
                            />
                          </label>
                        </td>
                        <td><span class="chip chip-phrase chip-mode" title={hook.event}>{HOOK_EVENT_LABELS[hook.event] ?? hook.event}</span></td>
                        <td>
                          <Switch>
                            <Match when={!hook.matcher}>
                              <span class="chip chip-tone-warning">every tool call</span>
                            </Match>
                            {/* Said plainly, with the verbatim matcher on
                                hover: the stored string's glob half is
                                case-significant, so it stays reachable. The
                                parse only decides whether the engine will
                                accept it at all. */}
                            <Match when={scope()}>
                              <span class="chip chip-phrase" title={hook.matcher ?? ""}>{describeMatcher(hook.matcher)}</span>
                            </Match>
                            <Match when={!scope()}>
                              <span class="chip chip-tone-danger mono" title="Vak cannot read this pattern, so this automation will not run. Edit it to fix.">
                                {hook.matcher} · not valid
                              </span>
                            </Match>
                          </Switch>
                        </td>
                        <td class="dim col-command">
                          <span class="path" title={hook.command}>{hook.command}</span>
                        </td>
                        <td class="dim">{hook.timeout_ms < 1000 ? `${hook.timeout_ms}ms` : shortDuration(hook.timeout_ms / 1000)}</td>
                        <td>
                          <div class="row-gap">
                            <button class="ghost small" disabled={isBusy()} onClick={() => beginEdit(index(), hook)}>
                              Edit
                            </button>
                            <button class="danger small" disabled={isBusy()} onClick={() => void removeHook(index())}>
                              Remove
                            </button>
                          </div>
                        </td>
                      </tr>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>

        <Show when={editingIndex() >= 0 ? String(editingIndex()) : ""} keyed>
          <div class="edit-form" aria-label="Edit automation">
            <div class="panel-title-row">
              <div>
                <h3>Edit automation</h3>
                <p class="dim">Changes apply to the next matching call.</p>
              </div>
              <button class="ghost small" onClick={() => setEditingIndex(-1)}>Cancel</button>
            </div>
            <div class="form-row">
              <label>When</label>
              <select value={editEvent()} onChange={(e) => setEditEvent(e.currentTarget.value)}>
                <For each={HOOK_EVENTS}>{(ev) => <option value={ev}>{HOOK_EVENT_LABELS[ev] ?? ev}</option>}</For>
              </select>
            </div>
            <MatcherBuilder value={editMatcher()} onChange={setEditMatcher} tools={MATCHER_TOOLS} />
            <div class="form-row">
              <label>Script to run</label>
              <input class="mono" value={editCommand()} onInput={(e) => setEditCommand(e.currentTarget.value)} />
            </div>
            <div class="form-row">
              <label>Give up after</label>
              <TimeoutPicker value={editTimeout()} onChange={setEditTimeout} />
            </div>
            <div class="form-row">
              <label>On hook failure</label>
              <select value={editFailureMode()} onChange={(e) => setEditFailureMode(e.currentTarget.value as "open" | "closed")}>
                <option value="open">Continue and report</option>
                <option value="closed">Block the operation</option>
              </select>
            </div>
            <button disabled={busyIndex() === editingIndex() || !editCommand().trim()} onClick={() => void saveEdit()}>
              {busyIndex() === editingIndex() ? "Saving…" : "Save changes"}
            </button>
          </div>
        </Show>

        <details class="advanced">
          <summary>Add an automation</summary>
          <p class="dim">
            A hook runs your own script at a point in the turn. Pick when it fires and which calls
            it should see; the script decides the rest.
          </p>
          <div class="form-row">
            <label>When</label>
            <select value={event()} onChange={(e) => setEvent(e.currentTarget.value)}>
              <For each={HOOK_EVENTS}>{(ev) => <option value={ev}>{HOOK_EVENT_LABELS[ev] ?? ev}</option>}</For>
            </select>
          </div>
          <MatcherBuilder value={matcher()} onChange={setMatcher} tools={MATCHER_TOOLS} />
          <div class="form-row">
            <label>Script to run</label>
            <input class="mono" placeholder="/path/to/script.sh" value={command()} onInput={(e) => setCommand(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>Give up after</label>
            <TimeoutPicker value={timeout()} onChange={setTimeoutMs} />
          </div>
          <div class="form-row">
            <label>On hook failure</label>
            <select value={failureMode()} onChange={(e) => setFailureMode(e.currentTarget.value as "open" | "closed")}>
              <option value="open">Continue and report</option>
              <option value="closed">Block the operation</option>
            </select>
          </div>
          <div class="row-gap" style="margin-top:12px">
            <button disabled={busy() || !command().trim()} onClick={() => void addHook()}>
              {busy() ? "Adding…" : "Add automation"}
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
  const [editingId, setEditingId] = createSignal("");
  const [editName, setEditName] = createSignal("");
  const [editPrompt, setEditPrompt] = createSignal("");
  const [editSchedule, setEditSchedule] = createSignal("");
  const [editModelPin, setEditModelPin] = createSignal("");
  /// Set only for a task that runs on a plain interval rather than a cron.
  const [editInterval, setEditInterval] = createSignal<number | null>(null);

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

  const beginEdit = (task: TaskItem) => {
    // Fields before the id, for the same reason as the hooks editor above.
    setEditName(task.name);
    setEditPrompt(task.prompt ?? "");
    setEditSchedule(task.schedule ?? "");
    setEditInterval(task.schedule ? null : (task.interval_secs ?? null));
    setEditModelPin(task.model_pin ?? "");
    setEditingId(task.id);
  };

  const saveEdit = async () => {
    const id = editingId();
    if (!id || !editName().trim() || !editPrompt().trim() || busyId()) return;
    setBusyId(id);
    try {
      await api.patchTask(id, {
        name: editName().trim(),
        prompt: editPrompt().trim(),
        schedule: editSchedule().trim() || null,
        model_pin: editModelPin().trim() || null,
      });
      pushToast("info", "Task updated");
      setEditingId("");
      props.ctx.refetchTasks();
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
              <thead><tr><th>task</th><th>type</th><th>runs</th><th>model</th><th>last run</th><th>state</th><th /></tr></thead>
              <tbody><SkeletonRows cols={7} /></tbody>
            </table>
          </Match>

          <Match when={props.ctx.tasks().length === 0}>
            <div class="empty empty-teach">
              <strong>Nothing is scheduled.</strong>
              <p>
A scheduled task is something you ask vak to do on a repeating schedule — a nightly
                digest, a weekly check — with the result filed in your Inbox.
              </p>
            </div>
          </Match>

          <Match when={props.ctx.tasks().length > 0}>
            <table class="table">
              <thead><tr><th>task</th><th>type</th><th>runs</th><th>model</th><th>last run</th><th>state</th><th /></tr></thead>
              <tbody>
                <For each={props.ctx.tasks()}>
                  {(t) => (
                    <tr class="row-static">
                      <td class="bold">{t.name}</td>
                      <td><span class={`chip ${t.script ? "chip-tool" : "chip-mode"}`}>{t.script ? "runs a script" : "asks vak"}</span></td>
                      <td class="dim" title={t.schedule ?? ""}>
                        {t.schedule ? describeSchedule(t.schedule) : `Every ${describeDuration(t.interval_secs ?? 3600)}`}
                      </td>
                      <td class="dim">{t.model_pin ?? "Project default"}</td>
                      <td class="dim" title={t.last_run_at ?? ""}>{t.last_run_at ? timeAgo(t.last_run_at) : "never"}</td>
                      <td>
                        <span class={t.enabled ? "chip chip-tone-success" : "chip"}>
                          {t.enabled ? "on" : "paused"}
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
                          <button class="ghost small" disabled={busyId() === t.id} onClick={() => beginEdit(t)}>
                            Edit
                          </button>
                          <button
                            class="danger small"
                            disabled={busyId() === t.id}
                            onClick={() => {
                              if (confirmDestructive(`Delete task “${t.name}”?`)) {
                                void guard(t.id, () => api.deleteTask(t.id), "Task deleted");
                              }
                            }}
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

        <Show when={editingId()} keyed>
          <div class="edit-form" aria-label="Edit scheduled task">
            <div class="panel-title-row">
              <div>
                <h3>Edit task</h3>
                <p class="dim">Changes apply to the next run. Existing session history is preserved.</p>
              </div>
              <button class="ghost small" onClick={() => setEditingId("")}>Cancel</button>
            </div>
            <div class="form-row"><label>Name</label><input value={editName()} onInput={(e) => setEditName(e.currentTarget.value)} /></div>
            <div class="form-row"><label>What to do</label><textarea rows={3} value={editPrompt()} onInput={(e) => setEditPrompt(e.currentTarget.value)} /></div>
            <Show when={editInterval() != null && !editSchedule().trim()}>
              <p class="dim">
                This task runs every {describeDuration(editInterval())} at the moment. Leave the
                schedule alone to keep that, or pick one below to switch it over.
              </p>
            </Show>
            <ScheduleBuilder value={editSchedule()} onChange={setEditSchedule} />
            <div class="form-row"><label>Model</label><input class="mono" placeholder="Workspace default" value={editModelPin()} onInput={(e) => setEditModelPin(e.currentTarget.value)} /></div>
            <button disabled={busyId() === editingId() || !editName().trim() || !editPrompt().trim()} onClick={() => void saveEdit()}>
              {busyId() === editingId() ? "Saving…" : "Save changes"}
            </button>
          </div>
        </Show>

        <details class="advanced">
          <summary>Schedule a task</summary>
          <div class="form-row">
            <label>Name</label>
            <input placeholder="Nightly digest, dependency audit…" value={name()} onInput={(e) => setName(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>What to do</label>
            <textarea rows={3} placeholder="What should vak do each time this runs?" value={prompt()} onInput={(e) => setPrompt(e.currentTarget.value)} />
          </div>
          <ScheduleBuilder value={schedule()} onChange={setSchedule} />
          <div class="form-row">
            <label>Model</label>
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
  const [plugins, pluginsActions] = createResource(() => api.plugins());
  const [proposals, proposalsActions] = createResource(() => api.skillProposals());
  const [tasks, tasksActions] = createResource(() => api.tasks());

  const ctx: ExtensionsCtx = {
    plugins: () => plugins()?.plugins ?? [],
    pluginsLoading: () => plugins.loading,
    refetchPlugins: () => void pluginsActions.refetch(),
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
      ["connected apps", mcp],
      ["automations", hooks],
      ["skills", skills],
      ["plugins", plugins],
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
      <PageHeader title="Extensions" description="Configure the external processes, instructions, hooks, and unattended tasks available to vak." />
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
        <Match when={extensionsTab() === "#/integrations/plugins"}><PluginsView ctx={ctx} /></Match>
        <Match when={extensionsTab() === "#/integrations/skills"}><SkillsView ctx={ctx} /></Match>
        <Match when={extensionsTab() === "#/integrations/hooks"}><HooksView ctx={ctx} /></Match>
        <Match when={extensionsTab() === "#/integrations/tasks"}><TasksView ctx={ctx} /></Match>
      </Switch>
    </div>
  );
}

// ---- Memory & Recall -------------------------------------------------------

/// Age → lifecycle bucket for the freshness dot on each note card. Memory
/// notes never expire on their own (docs/design/23-memory.md), so this is
/// purely a "how long has vak been carrying this" signal, not a TTL.
function noteAgeBucket(ts: string): "fresh" | "aging" | "stale" {
  const days = (Date.now() - new Date(ts).getTime()) / 86_400_000;
  if (days < 7) return "fresh";
  if (days < 30) return "aging";
  return "stale";
}

function MemoryView() {
  const [memoryData, { refetch }] = createResource(() => api.memory());
  const [configData, { refetch: refetchConfig }] = createResource(() => api.config());
  const [scope, setScope] = createSignal<"profile" | "project">("project");
  const [tag, setTag] = createSignal("");
  const [noteText, setNoteText] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [editing, setEditing] = createSignal<string | null>(null);
  const [editText, setEditText] = createSignal("");
  const [cleaning, setCleaning] = createSignal(false);
  const [togglingFlag, setTogglingFlag] = createSignal<string | null>(null);
  const [filterScope, setFilterScope] = createSignal<"all" | "profile" | "workspace">("all");
  const [query, setQuery] = createSignal("");

  const notes = createMemo(() => memoryData()?.notes ?? []);
  const profileCount = createMemo(() => notes().filter((n) => n.scope === "profile").length);
  const projectCount = createMemo(() => notes().filter((n) => n.scope !== "profile").length);
  const staleCount = createMemo(() => notes().filter((n) => noteAgeBucket(n.ts) === "stale").length);
  const oldestAgo = createMemo(() => {
    const list = notes();
    if (!list.length) return "—";
    const oldest = list.reduce((a, b) => (new Date(a.ts) < new Date(b.ts) ? a : b));
    return timeAgo(oldest.ts);
  });
  const visibleNotes = createMemo(() => {
    const q = query().trim().toLowerCase();
    return notes().filter((n) => {
      if (filterScope() !== "all") {
        const bucket = n.scope === "profile" ? "profile" : "workspace";
        if (bucket !== filterScope()) return false;
      }
      if (q && !n.text.toLowerCase().includes(q) && !(n.tag ?? "").toLowerCase().includes(q)) return false;
      return true;
    });
  });

  // Real toggles, not a status readout: each flips exactly one [memory]
  // config key, persisted to .vak/config.toml and applied to the live
  // Core immediately (docs/design/23-memory.md) — no restart needed, and
  // no other flag is touched by this call.
  const toggleMemoryFlag = async (
    key: "memory_search_enabled" | "memory_write_enabled" | "memory_reflection" | "memory_skill_proposals",
    next: boolean,
  ) => {
    setTogglingFlag(key);
    try {
      await api.patchConfig({ [key]: next });
      await refetchConfig();
      pushToast("info", `${next ? "Enabled" : "Disabled"}`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setTogglingFlag(null);
    }
  };

  const cleanArtifacts = async () => {
    if (!confirmDestructive("Remove only abandoned memory lock/temp files and empty workspace folders? Notes will not be deleted.")) return;
    setCleaning(true);
    try {
      const report = await api.cleanupMemory();
      pushToast("info", `Cleaned ${report.removed_locks} locks, ${report.removed_temps} temp files, ${report.removed_empty_dirs} empty folders`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setCleaning(false);
    }
  };

  const addNote = async () => {
    if (!noteText().trim() || busy()) return;
    setBusy(true);
    try {
      await api.addMemory(scope(), noteText().trim(), tag().trim() || undefined);
      pushToast("info", "Vak will remember that");
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
    if (!confirmDestructive("Forget this note? Vak stops taking it into account.")) return;
    try {
      const note = memoryData()?.notes?.find((m) => m.id === id);
      await api.forgetMemory(id, note?.scope === "profile" ? "profile" : "workspace");
      pushToast("info", "Forgotten");
      refetch();
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const flagCards: Array<{
    key: "memory_search_enabled" | "memory_write_enabled" | "memory_reflection" | "memory_skill_proposals";
    on: () => boolean;
    title: string;
    desc: string;
  }> = [
    { key: "memory_search_enabled", on: () => configData()?.memory?.search_enabled ?? false, title: "Search past sessions", desc: "Lets vak look up earlier conversations mid-run." },
    { key: "memory_write_enabled", on: () => configData()?.memory?.write_enabled ?? false, title: "Write notes", desc: "Lets vak save what it learns mid-run, not just what you add here." },
    { key: "memory_reflection", on: () => configData()?.memory?.reflection ?? false, title: "Reflect after each run", desc: "A short pass proposing notes/skills from what just happened." },
    { key: "memory_skill_proposals", on: () => configData()?.memory?.skill_proposals ?? false, title: "Propose skills", desc: "Lets reflection suggest new skills for you to review." },
  ];

  return (
    <div class="view">
      <PageHeader
        title="Memory"
        description="Things vak should keep in mind between sessions — about you, or about this project."
        actions={<button class="ghost small" disabled={cleaning()} onClick={() => void cleanArtifacts()}>{cleaning() ? "Cleaning…" : "Clean artifacts"}</button>}
      />

      <div class="stat-strip">
        <StatCard label="Total memories" value={notes().length} />
        <StatCard label="About me" value={profileCount()} sub="carried across every project" />
        <StatCard label="About this project" value={projectCount()} />
        <StatCard label="Oldest note" value={oldestAgo()} tone={staleCount() > 0 ? "warn" : undefined} sub={staleCount() > 0 ? `${staleCount()} unrevisited 30+ days` : undefined} />
      </div>

      <section class="panel">
        <h2>Governance</h2>
        <p class="dim" style="margin-top:-4px">What vak is allowed to do with memory, live — no restart needed.</p>
        <div class="toggle-card-grid" style="margin-top:10px">
          <For each={flagCards}>
            {(f) => (
              <div class="toggle-card" data-on={f.on()}>
                <div class="toggle-card-body">
                  <strong>{f.title}</strong>
                  <span>{f.desc}</span>
                </div>
                <label class="toggle">
                  <input
                    type="checkbox"
                    checked={f.on()}
                    disabled={configData.loading || togglingFlag() === f.key}
                    onChange={(e) => void toggleMemoryFlag(f.key, e.currentTarget.checked)}
                  />
                </label>
              </div>
            )}
          </For>
        </div>
      </section>

      <div class="two-col">
        <section class="panel">
          <h2>What vak remembers ({notes().length})</h2>
          <div class="memory-toolbar">
            <input class="search-input" placeholder="Filter by text or label…" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} />
            <div class="scope-tabs">
              <button classList={{ active: filterScope() === "all" }} onClick={() => setFilterScope("all")}>All</button>
              <button classList={{ active: filterScope() === "profile" }} onClick={() => setFilterScope("profile")}>About me</button>
              <button classList={{ active: filterScope() === "workspace" }} onClick={() => setFilterScope("workspace")}>This project</button>
            </div>
          </div>
          <Show when={!memoryData.loading} fallback={<div class="empty">Loading…</div>}>
            <Show when={!memoryData.error} fallback={<LoadError message={`${memoryData.error}`} onRetry={() => refetch()} />}>
            <Show when={notes().length > 0} fallback={<div class="empty">Nothing remembered yet. Add a note on the right.</div>}>
            <Show when={visibleNotes().length > 0} fallback={<div class="empty">No memories match that filter.</div>}>
              <ul class="hit-list">
                <For each={visibleNotes()}>
                  {(m: MemoryItem) => (
                    <li class="note-card">
                      <div class="note-head">
                        <span class="note-freshness" data-age={noteAgeBucket(m.ts)} title={`Last touched ${timeAgo(m.ts)}`} />
                        <span class={`chip ${m.scope === "profile" ? "chip-mode" : "chip-tool"}`} title={m.scope}>{m.scope === "profile" ? "about me" : "about this project"}</span>
                        <Show when={m.tag}><strong class="mono">{m.tag}</strong></Show>
                        <span class="dim" style="margin-left:auto; font-size:11px">{timeAgo(m.ts)}</span>
                      </div>
                      <Show when={editing() === m.id} fallback={<div class="note-text">{m.text}</div>}>
                        <textarea value={editText()} onInput={(e) => setEditText(e.currentTarget.value)} />
                        <button class="small" onClick={async () => {
                          if (!editText().trim()) return;
                          try {
                            await api.amendMemory(m.id, m.scope === "profile" ? "profile" : "workspace", editText().trim());
                            setEditing(null); refetch();
                            pushToast("info", "Memory amended");
                          } catch (err) {
                            pushToast("alert", `${err}`);
                          }
                        }}>Save</button>
                      </Show>
                      <div class="note-foot">
                        <Show when={editing() !== m.id}>
                          <button class="small" onClick={() => { setEditing(m.id); setEditText(m.text); }}>Amend</button>
                        </Show>
                        <Show when={m.session_id}>
                          <button class="ghost small" onClick={() => navigate(`#/sessions/${m.session_id}`)}>From this session</button>
                        </Show>
                        <span class="spacer" />
                        <button class="danger small" onClick={() => forget(m.id)}>Forget</button>
                      </div>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
            </Show>
            </Show>
          </Show>
        </section>

        <section class="panel">
          <div class="panel-title-row">
            <div>
              <h2>Add a note</h2>
              <p class="dim">Vak reads these at the start of every session.</p>
            </div>
          </div>
          <div class="form-row">
            <label>Applies to</label>
            <select value={scope()} onChange={(e) => setScope(e.currentTarget.value as "profile" | "project")}>
              <option value="project">This project</option>
              <option value="profile">Me, in every project</option>
            </select>
          </div>
          <div class="form-row">
            <label>Label</label>
            <input placeholder="Optional label, e.g. billing" value={tag()} onInput={(e) => setTag(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>Note</label>
            <textarea rows={4} placeholder="What should vak remember?" value={noteText()} onInput={(e) => setNoteText(e.currentTarget.value)} />
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

// ---- FinOps -----------------------------------------------------------------

/// A self-contained SVG bar chart for the 14-day spend trend — no charting
/// dependency, since this is one series with no zoom/tooltip requirement.
/// Every bar is drawn even at zero spend (a thin baseline tick), so a quiet
/// day reads as "no spend", never as "no data".
function SpendTrendChart(props: { points: FinOpsDailyPoint[]; capUsd: number | null }) {
  const width = 640;
  const height = 160;
  const padding = { top: 10, right: 10, bottom: 24, left: 44 };
  const plotW = width - padding.left - padding.right;
  const plotH = height - padding.top - padding.bottom;

  const maxUsd = createMemo(() => {
    const max = Math.max(...props.points.map((p) => p.usd), props.capUsd ?? 0);
    return max > 0 ? max * 1.15 : 1;
  });
  const barW = createMemo(() => (props.points.length ? plotW / props.points.length : 0));
  const yFor = (usd: number) => padding.top + plotH * (1 - usd / maxUsd());
  const capY = createMemo(() => (props.capUsd != null ? yFor(props.capUsd) : null));

  return (
    <svg viewBox={`0 0 ${width} ${height}`} width="100%" height={height} role="img" aria-label="Daily spend, last 14 days">
      {/* Baseline */}
      <line
        x1={padding.left} y1={padding.top + plotH} x2={width - padding.right} y2={padding.top + plotH}
        stroke="var(--border)" stroke-width="1"
      />
      <Show when={capY() != null}>
        <line
          x1={padding.left} y1={capY()!} x2={width - padding.right} y2={capY()!}
          stroke="var(--accent)" stroke-width="1" stroke-dasharray="4 3" opacity="0.6"
        />
        <text x={width - padding.right} y={capY()! - 4} text-anchor="end" fill="var(--accent)" font-size="10">
          cap ${props.capUsd!.toFixed(2)}
        </text>
      </Show>
      <For each={props.points}>
        {(p, i) => {
          const x = padding.left + i() * barW() + barW() * 0.15;
          const w = barW() * 0.7;
          const y = yFor(p.usd);
          const h = Math.max(padding.top + plotH - y, 1.5);
          const label = p.date.slice(5); // MM-DD
          return (
            <g>
              <title>{`${p.date}: $${p.usd.toFixed(4)}`}</title>
              <rect x={x} y={y} width={w} height={h} rx="1.5" fill="var(--accent)" opacity={p.usd > 0 ? 0.85 : 0.25} />
              <Show when={i() % 2 === 0 || props.points.length <= 8}>
                <text
                  x={x + w / 2}
                  y={height - 6}
                  text-anchor="middle"
                  fill="var(--faint)"
                  font-size="9"
                >
                  {label}
                </text>
              </Show>
            </g>
          );
        }}
      </For>
    </svg>
  );
}

function FinOpsRollupTable(props: { title: string; rows: FinOpsRollupEntry[] }) {
  const total = createMemo(() => props.rows.reduce((a, r) => a + r.usd, 0));
  return (
    <div>
      <span class="eyebrow">{props.title}</span>
      <Show when={props.rows.length > 0} fallback={<p class="dim">No dispatches today.</p>}>
        <table class="table">
          <thead><tr><th>{props.title === "By provider" ? "provider" : "model"}</th><th>calls</th><th>usd</th><th>share</th></tr></thead>
          <tbody>
            <For each={[...props.rows].sort((a, b) => b.usd - a.usd)}>
              {(r) => (
                <tr>
                  <td class="mono">{r.name || "(unknown)"}</td>
                  <td>{r.calls}</td>
                  <td class="mono">${r.usd.toFixed(4)}</td>
                  <td class="dim">{total() > 0 ? `${((r.usd / total()) * 100).toFixed(0)}%` : "—"}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </Show>
    </div>
  );
}

function FinOpsView() {
  const [data, { refetch }] = createResource(() => api.finops());
  const [runCapInput, setRunCapInput] = createSignal("");
  const [dayCapInput, setDayCapInput] = createSignal("");
  const [savingCaps, setSavingCaps] = createSignal(false);

  let initialized = false;
  createEffect(() => {
    const d = data();
    if (d && !initialized) {
      initialized = true;
      setRunCapInput(d.run_cap_usd != null ? String(d.run_cap_usd) : "");
      setDayCapInput(d.day_cap_usd != null ? String(d.day_cap_usd) : "");
    }
  });

  const parseCap = (raw: string): number | null | undefined => {
    const trimmed = raw.trim();
    if (trimmed === "") return null; // explicit clear
    const n = Number(trimmed);
    return Number.isFinite(n) && n >= 0 ? n : undefined; // undefined = invalid, don't send
  };

  const saveCaps = async () => {
    const run = parseCap(runCapInput());
    const day = parseCap(dayCapInput());
    if (run === undefined || day === undefined) {
      pushToast("alert", "A budget cap must be a non-negative number, or blank to clear it.");
      return;
    }
    setSavingCaps(true);
    try {
      await api.patchFinops({ max_run_usd: run, max_day_usd: day });
      pushToast("info", "Budget caps saved");
      await refetch();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setSavingCaps(false);
    }
  };

  const dayProgress = createMemo(() => {
    const d = data();
    if (!d || !d.day_cap_usd || d.day_cap_usd <= 0) return null;
    return Math.min(100, (d.day_usd / d.day_cap_usd) * 100);
  });

  return (
    <div class="view">
      <PageHeader
        title="FinOps"
        description="What every dispatch is estimated to cost, where it goes, and the caps that keep it in check."
      />
      <Show when={!data.loading} fallback={<div class="cred-list"><span class="skel skel-block" /><span class="skel skel-block" /></div>}>
        <Show when={!data.error} fallback={<LoadError message={`${data.error}`} onRetry={() => refetch()} />}>
          <div class="stats-grid" style="margin-bottom:14px">
            <StatCard
              label="Spent today"
              value={`$${(data()?.day_usd ?? 0).toFixed(4)}`}
              progress={dayProgress() ?? undefined}
              sub={data()?.day_cap_usd ? `of $${data()!.day_cap_usd!.toFixed(2)} daily budget` : "No daily budget set"}
              tone={dayProgress() != null && dayProgress()! > 90 ? "warn" : undefined}
            />
            <StatCard
              label="Run cap"
              value={data()?.run_cap_usd != null ? `$${data()!.run_cap_usd!.toFixed(2)}` : "No limit"}
              sub="Ceiling checked before every dispatch"
            />
            <StatCard
              label="Dispatches today"
              value={(data()?.by_provider ?? []).reduce((a, p) => a + p.calls, 0)}
              sub={`${data()?.total_rows ?? 0} total in the ledger`}
            />
            <StatCard
              label="Unpriced dispatches"
              value={data()?.unknown_rows ?? 0}
              sub="Model has no known price — cost is unknown, not zero"
              tone={(data()?.unknown_rows ?? 0) > 0 ? "warn" : undefined}
            />
          </div>

          <section class="panel" style="margin-bottom:14px">
            <div class="panel-title-row">
              <div>
                <h2>Spend, last 14 days</h2>
                <p class="dim">Estimated USD per day from the cost ledger. Every dollar figure here is an estimate — providers don't return real cost.</p>
              </div>
            </div>
            <SpendTrendChart points={data()?.daily ?? []} capUsd={data()?.day_cap_usd ?? null} />
          </section>

          <div class="two-col">
            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Budget caps</h2>
                  <p class="dim">Checked before every dispatch. Leave a field blank to remove that cap.</p>
                </div>
              </div>
              <div class="form-row">
                <label>Per-run cap (USD)</label>
                <input class="mono" placeholder="No limit" value={runCapInput()} onInput={(e) => setRunCapInput(e.currentTarget.value)} />
              </div>
              <div class="form-row">
                <label>Per-day cap (USD)</label>
                <input class="mono" placeholder="No limit" value={dayCapInput()} onInput={(e) => setDayCapInput(e.currentTarget.value)} />
              </div>
              <div class="row-gap" style="margin-top:10px">
                <button disabled={savingCaps()} onClick={() => void saveCaps()}>
                  {savingCaps() ? "Saving…" : "Save caps"}
                </button>
              </div>
            </section>

            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Recent budget alerts</h2>
                  <p class="dim">Fired once per threshold per day — 80% and 100% of the daily cap.</p>
                </div>
              </div>
              <Show when={(data()?.recent_alerts ?? []).length > 0} fallback={<p class="dim">No alerts yet.</p>}>
                <ul class="hit-list">
                  <For each={data()?.recent_alerts ?? []}>
                    {(a) => (
                      <li class="inbox-item">
                        <div class="hit-meta">
                          <span class={`chip chip-tone-${a.level === "full" ? "danger" : "warning"}`}>{a.level === "full" ? "100%" : "80%"}</span>
                          <span class="when">{timeAgo(a.ts)}</span>
                        </div>
                        <div class="hit-snippet">Day spend was ${a.day_total_usd.toFixed(2)} · session {shortId(a.session_id)}</div>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
            </section>
          </div>

          <div class="two-col" style="margin-top:14px">
            <section class="panel"><FinOpsRollupTable title="By provider" rows={data()?.by_provider ?? []} /></section>
            <section class="panel"><FinOpsRollupTable title="By model" rows={data()?.by_model ?? []} /></section>
          </div>
        </Show>
      </Show>
    </div>
  );
}

// ---- Search ----------------------------------------------------------------

function highlight(snippet: string): string {
  // FTS snippets contain only <b> emphasis markers, but the surrounding text
  // is still untrusted transcript content. Escape it before using innerHTML.
  const escaped = snippet
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
  return escaped.replaceAll("&lt;b&gt;", "<mark>").replaceAll("&lt;/b&gt;", "</mark>");
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
      <PageHeader title="Search" description="Look through every conversation vak has had, and jump straight to where something was said." />
      <div class="search-hero">
        <input
          class="search-big"
          placeholder="Search everything you’ve ever asked…"
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
          <div class="empty">Start typing to search every conversation. Put a phrase in "quotes" to match it exactly, or end a word with * to match anything that starts with it.</div>
        </Match>
        <Match when={hits()?.length === 0}>
          <div class="empty">Nothing found.</div>
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

/// The audit ledger's own kind strings, each paired with what it means. The
/// value is what `GET /security?kind=` filters on and must not change; the
/// label is all the operator ever needs to read.
const SEC_KINDS: { value: string; label: string }[] = [
  { value: "", label: "Everything" },
  { value: "auth_failure", label: "Failed sign-in" },
  { value: "rate_limit", label: "Rate limited" },
  { value: "chat_allowlist", label: "Chat allowed" },
  { value: "chat_pending", label: "Chat knocked" },
  { value: "chat_approved", label: "Chat approved" },
  { value: "chat_denied", label: "Chat refused" },
  { value: "chat_revoked", label: "Chat removed" },
  { value: "permission_denial", label: "Action blocked" },
  { value: "config_change", label: "Setting changed" },
  { value: "provider_key_change", label: "Provider key changed" },
  { value: "full_access_grant", label: "Full access granted" },
  { value: "full_access_revoke", label: "Full access removed" },
];

const secKindLabel = (kind: string) =>
  SEC_KINDS.find((k) => k.value === kind)?.label ?? kind.replaceAll("_", " ");

function Security() {
  const [kind, setKind] = createSignal("");
  // The source function, not the fetcher, is what Solid tracks — reading
  // `kind()` inside the fetcher (the old `() => api.security(...)` form)
  // runs untracked, so clicking a filter chip never re-fetched until
  // something else happened to trigger a refetch.
  const [events, { refetch }] = createResource(kind, (k) => api.security(500, k || undefined));

  return (
    <div class="view">
      <PageHeader title="Security" description="Every sign-in, blocked action, key change, and chat decision vak has recorded." />
      <div class="toolbar">
        <div class="chips">
          <For each={SEC_KINDS}>
            {(k) => (
              <button
                class="chip-btn"
                classList={{ active: kind() === k.value }}
                onClick={() => setKind(k.value)}
                title={k.value}
              >
                {k.label}
              </button>
            )}
          </For>
        </div>
        <span class="spacer" />
        <button class="ghost" onClick={() => refetch()}>Refresh</button>
      </div>
      <Show when={!events.loading} fallback={<div class="empty">Loading…</div>}>
        <Show
          when={(events()?.events.length ?? 0) > 0}
          fallback={<div class="empty">Nothing recorded. Quiet is good.</div>}
        >
          <table class="table">
            <thead>
              <tr><th>when</th><th>what</th><th>summary</th><th>details</th><th>from</th></tr>
            </thead>
            <tbody>
              <For each={events()?.events}>
                {(e: SecurityEvent) => (
                  <tr data-kind={e.kind}>
                    <td title={e.ts}>{timeAgo(e.ts)}</td>
                    <td><span class="chip chip-phrase" data-kind={e.kind} title={e.kind}>{secKindLabel(e.kind)}</span></td>
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

/// The ledger's own kind strings, said plainly. An unknown kind still falls
/// back to its underscored id rather than vanishing.
const INBOX_KIND_LABELS: Record<string, string> = {
  task_summary: "scheduled task",
  approval_pending: "needs approval",
  approval_denied: "refused",
  budget_alert: "budget",
  digest: "digest",
  heartbeat: "status",
  proposal_opened: "new skill proposed",
};

function Inbox() {
  const [unreadOnly, setUnreadOnly] = createSignal(false);
  const [inbox, { refetch }] = createResource(unreadOnly, (u) => api.inbox(u));
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
      <PageHeader title="Inbox" description="Approvals to give, results from scheduled work, budget warnings, and anything else vak wants you to see." />
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
      <Show when={!inbox.loading} fallback={<div class="empty">Loading…</div>}>
        <Show
          when={(inbox()?.entries.length ?? 0) > 0}
          fallback={<div class="empty">Nothing here. Inbox zero.</div>}
        >
          <ul class="hit-list">
            <For each={inbox()!.entries}>
              {(e) => (
                <li class="inbox-item">
                  <div class="hit-meta">
                    <span class={`chip ${INBOX_KIND_TONE[e.kind] ? `chip-tone-${INBOX_KIND_TONE[e.kind]}` : ""}`}>
                      {INBOX_KIND_LABELS[e.kind] ?? e.kind.replaceAll("_", " ")}
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
    <span class="chip chip-surface" data-surface={surface()} title={`${SURFACE_LABEL[surface()] ?? surface()} chat`}>
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
        Project folder
        <span class="chip chip-phrase" data-on={warm()} style="margin-left:6px">
          {warm() ? "loaded and ready" : "loads on the next message"}
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
        <option value={CUSTOM_WORKSPACE}>Another folder…</option>
      </select>
      <Show when={custom()}>
        <input
          class="mono"
          value={props.value}
          onInput={(e) => props.onChange(e.currentTarget.value)}
          placeholder="/full/path/to/the/folder"
          style="width:100%;margin-top:6px"
        />
        <div class="binding-meta">
Nothing has run here yet, so this path isn’t checked until the first message arrives. A
          typo shows up later as a failure — run “Check for problems” under Settings to catch one.
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
  bots: Bot[];
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
            {props.binding.session_id ? `session ${shortId(props.binding.session_id)}` : "starts a new session on the next message"}
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
            {props.binding.stale ? "new session next" : "up to date"}
          </span>
        </div>
      </div>

      <div class="route-compare">
        <div>
          <span class="eyebrow">Answering with</span>
          <strong>{providerLabel(props.binding.effective_route.provider)}</strong>
          <code>{props.binding.effective_route.model}</code>
          <span class="binding-meta">from {sourceLabel(props.binding.effective_route.source)}</span>
        </div>
        {/* The permission counterpart of "Effective route": what this
            channel actually runs as, and where that came from. A pin that
            the workspace's own mode capped is called out rather than shown
            as if the wider grant were live. */}
        <Show when={props.allowlistEntry?.effective_permission_mode}>
          <div>
            <span class="eyebrow">Allowed to</span>
            <strong>{modeLabel(props.allowlistEntry!.effective_permission_mode)}</strong>
            <span class="binding-meta">
              {props.allowlistEntry!.permission_capped
                ? `reduced — you asked for "${modeLabel(props.allowlistEntry!.permission_mode)}", which is more than this project allows`
                : props.allowlistEntry!.permission_mode
                  ? "set for this chat"
                  : "follows the project"}
            </span>
          </div>
        </Show>
        <div>
          <span class="eyebrow">Current conversation</span>
          <Show when={props.binding.session_contract} fallback={<span class="dim">Not started yet</span>}>
            <strong>{providerLabel(props.binding.session_contract!.provider)}</strong>
            <code>{props.binding.session_contract!.model}</code>
            <span class="binding-meta">v{props.binding.session_contract!.app_version}</span>
          </Show>
        </div>
      </div>

      <Show when={props.binding.stale_reasons.length}>
        <div class="warning">The next message starts a fresh conversation, because this changed: {props.binding.stale_reasons.join(", ").replaceAll("_", " ")}</div>
      </Show>

      <label class="inherit-toggle">
        <input type="checkbox" checked={inherit()} onChange={(e) => setInherit(e.currentTarget.checked)} />
        Use the project's model (untick to choose one for this chat)
      </label>
      <Show when={!inherit()}>
        <div class="binding-controls">
          <select
            value={provider()}
            ref={(el) => syncSelect(el, provider, () => props.providers)}
            onChange={(e) => setProvider(e.currentTarget.value)}
          >
            <For each={props.providers}>{(p) => <option value={p.name}>{providerLabel(p.name)}</option>}</For>
          </select>
          <Show when={models().length} fallback={
            <input class="mono" value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="model id" />
          }>
            <select
              class="mono"
              value={model()}
              ref={(el) => syncSelect(el, model, models)}
              onChange={(e) => setModel(e.currentTarget.value)}
            >
              <For each={models()}>{(m) => <option value={m}>{m}</option>}</For>
            </select>
          </Show>
        </div>
      </Show>
      <div class="row-gap">
        <button disabled={busy() || (!inherit() && !model().trim())} onClick={() => act(
          () => api.patchGatewayBinding(props.binding.target, inherit() ? {} : { provider: provider(), model: model() }),
          `Model updated for ${props.binding.target}`,
        )}>Save model</button>
        <button class="ghost" disabled={busy() || !props.binding.session_id} onClick={() => act(
          () => api.rotateGatewayBinding(props.binding.target),
          `Started a fresh conversation for ${props.binding.target}`,
        )}>Start fresh</button>
        <span class="spacer" />
        <Show when={props.allowlistEntry?.status === "allowed"}>
          <button class="ghost small" disabled={busy()} onClick={() => setEditing((v) => !v)}>
            {editing() ? "Close" : "Edit what it can do"}
          </button>
          <button class="danger small" disabled={busy()} onClick={() => {
            if (window.confirm(`Disconnect ${props.binding.target}? The next message from this chat is turned away, and it goes back to waiting for your approval.`)) {
              void act(() => api.revokeGatewayAllowlist(props.binding.target), `Disconnected ${props.binding.target}`);
            }
          }}>Disconnect</button>
        </Show>
        <button class="danger small" disabled={busy()} onClick={() => {
          if (window.confirm(`Remove ${props.binding.target} and the model chosen for it? The conversation history is kept.`)) {
            void act(() => api.deleteGatewayBinding(props.binding.target), `Removed ${props.binding.target}`);
          }
        }}>Remove</button>
      </div>
      <Show when={editing() && props.allowlistEntry?.status === "allowed"}>
        <ChannelAccessEditor
          entry={props.allowlistEntry!}
          providers={props.providers}
          knownWorkspaces={props.knownWorkspaces}
          corePool={props.corePool}
          bots={props.bots}
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
  { value: "read-only", label: "Look, don't touch", desc: "Reads and searches only; every change refused" },
  { value: "workspace-write", label: "Work inside this project", desc: "Changes files here; anything else asks first" },
  { value: "full-access", label: "No limits", desc: "Nothing is checked with you first" },
];

/// Same three words wherever a wire mode is printed back, in either casing
/// the server may use (`read-only` from the gateway, `ReadOnly` from
/// `GET /config`).
function modeLabel(mode: string | null | undefined): string {
  if (!mode) return "the workspace default";
  const kebab = CHANNEL_MODES.find((m) => m.value === mode);
  if (kebab) return kebab.label;
  return MODES.find((m) => m.value === mode)?.label ?? mode;
}

function defaultChannelPolicy(): ChannelPolicy {
  return {
    tools_allow: null,
    tools_deny: [],
    mcp_allow: null,
    mcp_deny: [],
    skills_allow: null,
    skills_deny: [],
    hooks_allow: null,
    hooks_deny: [],
    mcp_network_deny: [],
  };
}

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
  /// What this pin belongs to, for the toggle's label — "channel" (a chat)
  /// or "bot". Defaults to "channel", the original caller's wording.
  subject?: string;
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
        Give this {props.subject ?? "channel"} its own limits (otherwise it follows the project)
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
              This project is set to “{modeLabel(c())}”. A channel can match that or ask for less,
              never more — so this choice will take effect as “{modeLabel(c())}”.
            </div>
          )}
        </Show>
      </Show>
    </>
  );
}

/// Gemini Live API's named prebuilt voices, as validated in the scratchpad
/// script (docs/design plan, section 2) — the same set `google_live.rs`
/// accepts for `voice_name`.
const VOICE_NAMES = ["Kore", "Puck", "Zephyr", "Charon", "Fenrir", "Aoede"];

/// Optional per-bot/per-chat voice + persona pin, parallel to
/// `ChannelPermissionPicker`. Unchecked means inherit up the same
/// `route`/`permission_mode` chain — `chat.voice.or(bot.voice)`, gated by
/// `inherit_bot_policy`. A controlled component: the parent panel owns the
/// signals and folds them into its own single Save call, exactly like
/// `pinRoute`/`pinPerm` do today.
function VoiceConfigEditor(props: {
  pinned: boolean;
  setPinned: (v: boolean) => void;
  voiceName: string;
  setVoiceName: (v: string) => void;
  persona: string;
  setPersona: (v: string) => void;
  /// What this pin belongs to, for the toggle's label — "bot" or "chat".
  subject: "bot" | "chat";
}) {
  const [previewing, setPreviewing] = createSignal(false);
  let audioEl: HTMLAudioElement | undefined;

  const preview = async () => {
    setPreviewing(true);
    try {
      const blob = await api.speak({
        text: "Hi, this is a preview of my voice.",
        voice_override: {
          voice_name: props.voiceName || null,
          persona: props.persona || null,
        },
      });
      if (!audioEl) audioEl = new Audio();
      audioEl.src = URL.createObjectURL(blob);
      await audioEl.play();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setPreviewing(false);
    }
  };

  return (
    <>
      <label class="inherit-toggle">
        <input
          type="checkbox"
          checked={props.pinned}
          onChange={(e) => props.setPinned(e.currentTarget.checked)}
        />
        Give this {props.subject === "bot" ? "bot" : "chat"} its own voice (otherwise it follows{" "}
        {props.subject === "bot" ? "its parent" : "the bot"})
      </label>
      <Show when={props.pinned}>
        <div class="binding-controls">
          <select value={props.voiceName} onChange={(e) => props.setVoiceName(e.currentTarget.value)}>
            <option value="">Default voice</option>
            <For each={VOICE_NAMES}>{(v) => <option value={v}>{v}</option>}</For>
          </select>
          <button disabled={previewing()} onClick={() => void preview()}>
            {previewing() ? "Playing…" : "Preview"}
          </button>
        </div>
        <textarea
          value={props.persona}
          onInput={(e) => props.setPersona(e.currentTarget.value)}
          placeholder="warm, upbeat, and enthusiastic"
          rows={2}
        />
      </Show>
    </>
  );
}

function ChannelCapabilityPolicy(props: {
  value: ChannelPolicy;
  onChange: (value: ChannelPolicy) => void;
}) {
  const [mcp] = createResource(() => api.mcpServers().catch(() => ({ servers: {} })));
  const [skills] = createResource(() => api.skills().catch(() => ({ skills: [] })));
  const [hooks] = createResource(() => api.hooks().catch(() => ({ hooks: [] })));
  const update = (patch: Partial<ChannelPolicy>) => props.onChange({ ...props.value, ...patch });

  // A server is matched as `name/*` (see `Core::filter_mcp`), a skill by its
  // own name, a hook by `event/command`. The picker offers exactly those
  // values so nothing here has to be typed.
  const mcpOptions = createMemo<AccessOption[]>(() =>
    Object.keys(mcp()?.servers ?? {}).map((name) => ({
      value: `${name}/*`,
      label: name,
      hint: "all of its tools",
    })),
  );
  const skillOptions = createMemo<AccessOption[]>(() =>
    (skills()?.skills ?? []).map((s) => ({
      value: s.name,
      label: s.name,
      hint: s.description?.slice(0, 60),
    })),
  );
  const hookOptions = createMemo<AccessOption[]>(() =>
    (hooks()?.hooks ?? []).map((h) => ({
      value: `${h.event}/${h.command}`,
      label: h.command.split("/").pop() || h.command,
      hint: `on ${HOOK_EVENT_LABELS[h.event] ?? h.event}`,
    })),
  );

  const networkServers = createMemo(
    () => Object.entries(mcp()?.servers ?? {}) as [string, McpServerConfig][],
  );
  const netOff = (name: string) => props.value.mcp_network_deny.includes(`${name}/*`);
  const toggleNet = (name: string) => {
    const pattern = `${name}/*`;
    update({
      mcp_network_deny: netOff(name)
        ? props.value.mcp_network_deny.filter((v) => v !== pattern)
        : [...props.value.mcp_network_deny, pattern],
    });
  };

  return (
    <section class="channel-capabilities">
      <div class="binding-meta">
        By default this channel gets whatever the workspace allows. Anything you change here can
        only take access away, never add it.
      </div>
      <div class="capability-grid">
        <AccessPicker
          title="What it can do on this machine"
          help="The built-in tools — reading files, editing them, running commands."
          noun="tool"
          inheritLabel="Everything the workspace allows"
          limitLabel="Only what I pick"
          noneLabel="No built-in tools"
          options={BUILTIN_TOOLS}
          allow={props.value.tools_allow}
          deny={props.value.tools_deny}
          onChange={(next) => update({ tools_allow: next.allow, tools_deny: next.deny })}
          patternHint="e.g. web*"
        />
        <AccessPicker
          title="Connected apps"
          help="Tools borrowed from the apps connected under Extensions."
          noun="server"
          inheritLabel="Every connected app"
          limitLabel="Only the apps I pick"
          noneLabel="No connected apps"
          options={mcpOptions()}
          loading={mcp.loading}
          allow={props.value.mcp_allow}
          deny={props.value.mcp_deny}
          onChange={(next) => update({ mcp_allow: next.allow, mcp_deny: next.deny })}
          patternHint="e.g. github/*"
        />
        <AccessPicker
          title="Skills"
          help="Which sets of instructions the agent can see on this channel."
          noun="skill"
          inheritLabel="Every skill"
          limitLabel="Only the skills I pick"
          noneLabel="No skills"
          options={skillOptions()}
          loading={skills.loading}
          allow={props.value.skills_allow}
          deny={props.value.skills_deny}
          onChange={(next) => update({ skills_allow: next.allow, skills_deny: next.deny })}
          patternHint="e.g. research*"
        />
        <AccessPicker
          title="Automations"
          help="The hooks an admin has set to run around each turn."
          noun="automation"
          inheritLabel="Every automation"
          limitLabel="Only the ones I pick"
          noneLabel="No automations"
          options={hookOptions()}
          loading={hooks.loading}
          allow={props.value.hooks_allow}
          deny={props.value.hooks_deny}
          onChange={(next) => update({ hooks_allow: next.allow, hooks_deny: next.deny })}
          patternHint="e.g. pre_tool_use/*"
        />
      </div>

      <Show when={networkServers().length > 0}>
        <section class="access-picker">
          <header class="access-head">
            <div>
              <h4>Internet access for connected apps</h4>
              <p class="dim">
                Turn the internet off for an app on this channel only. It stays on everywhere else.
                An app that has no internet to begin with cannot be given it here.
              </p>
            </div>
          </header>
          <div class="pick-grid">
            <For each={networkServers()}>
              {([name, server]) => (
                <label class="pick deny" classList={{ on: netOff(name) }}>
                  <input type="checkbox" checked={netOff(name)} onChange={() => toggleNet(name)} />
                  <span>
                    <strong>{name}</strong>
                    <em>{server.network ? "has internet — uncheck to keep it" : "already offline"}</em>
                  </span>
                </label>
              )}
            </For>
          </div>
        </section>
      </Show>
    </section>
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
  bots: Bot[];
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
  const [policy, setPolicy] = createSignal<ChannelPolicy>(props.entry.policy ?? defaultChannelPolicy());
  // Multi-bot-per-channel: which bot (if any) this chat is bound to, and
  // whether it inherits that bot's policy/permission/route tier or breaks
  // inheritance and resolves purely against the workspace.
  const surface = () => props.entry.key.split(":")[0];
  const surfaceBots = () => props.bots.filter((b) => b.surface === surface());
  const [botId, setBotId] = createSignal(props.entry.bot_id ?? "");
  const [inheritBot, setInheritBot] = createSignal(props.entry.inherit_bot_policy);
  const [pinVoice, setPinVoice] = createSignal(!!props.entry.voice);
  const [voiceName, setVoiceName] = createSignal(props.entry.voice?.voice_name ?? "");
  const [voicePersona, setVoicePersona] = createSignal(props.entry.voice?.persona ?? "");

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
        policy: policy(),
        bot_id: botId() || null,
        inherit_bot_policy: inheritBot(),
        voice: pinVoice() ? { voice_name: voiceName() || null, persona: voicePersona() || null } : null,
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
      <Show when={surfaceBots().length > 0}>
        <label class="inherit-toggle">
          Bot
          <select value={botId()} onChange={(e) => setBotId(e.currentTarget.value)}>
            <option value="">No bot (workspace only)</option>
            <For each={surfaceBots()}>{(b) => <option value={b.id}>{b.label}</option>}</For>
          </select>
        </label>
        <Show when={botId()}>
          <label class="inherit-toggle">
            <input type="checkbox" checked={inheritBot()} onChange={(e) => setInheritBot(e.currentTarget.checked)} />
            Inherit this bot's policy, permission mode, and model (uncheck to resolve against the project only)
          </label>
        </Show>
      </Show>
      <VoiceConfigEditor
        pinned={pinVoice()}
        setPinned={setPinVoice}
        voiceName={voiceName()}
        setVoiceName={setVoiceName}
        persona={voicePersona()}
        setPersona={setVoicePersona}
        subject="chat"
      />
      <label class="inherit-toggle">
        <input type="checkbox" checked={pinRoute()} onChange={(e) => setPinRoute(e.currentTarget.checked)} />
        Give this channel its own model (otherwise it follows the project)
      </label>
      <Show when={pinRoute()}>
        <div class="binding-controls">
          <select
            value={provider()}
            ref={(el) => syncSelect(el, provider, () => props.providers)}
            onChange={(e) => setProvider(e.currentTarget.value)}
          >
            <For each={props.providers}>{(p) => <option value={p.name}>{providerLabel(p.name)}</option>}</For>
          </select>
          <Show when={models().length} fallback={
            <input class="mono" value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="model id" />
          }>
            <select
              class="mono"
              value={model()}
              ref={(el) => syncSelect(el, model, models)}
              onChange={(e) => setModel(e.currentTarget.value)}
            >
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
      <ChannelCapabilityPolicy value={policy()} onChange={setPolicy} />
      <div class="row-gap">
        <button disabled={busy()} onClick={save}>Save changes</button>
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
  const [policy, setPolicy] = createSignal<ChannelPolicy>(props.entry.policy ?? defaultChannelPolicy());

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
        policy: policy(),
      });
      // Surface a capped grant at the moment it happens: the server
      // reduces an over-broad pin to the workspace's own mode, and an
      // operator who is not told would believe the wider grant is live.
      if (entry.permission_capped) {
        pushToast(
          "alert",
          `${props.entry.key}: "${modeLabel(entry.permission_mode)}" is more than that project allows, so it was reduced to "${modeLabel(entry.effective_permission_mode)}".`,
        );
      }
      pushToast("info", `Approved ${props.entry.key}${entry.workspace ? ` — working in ${entry.workspace}` : ", but no project folder was set"}`);
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
      pushToast("info", `Turned away ${props.entry.key}`);
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
          <div class="binding-meta">first messaged {timeAgo(props.entry.added_at)}</div>
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
        Give this channel its own model (otherwise it follows the project)
      </label>
      <Show when={pinRoute()}>
        <div class="binding-controls">
          <select
            value={provider()}
            ref={(el) => syncSelect(el, provider, () => props.providers)}
            onChange={(e) => setProvider(e.currentTarget.value)}
          >
            <For each={props.providers}>{(p) => <option value={p.name}>{providerLabel(p.name)}</option>}</For>
          </select>
          <Show when={models().length} fallback={
            <input class="mono" value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="model id" />
          }>
            <select
              class="mono"
              value={model()}
              ref={(el) => syncSelect(el, model, models)}
              onChange={(e) => setModel(e.currentTarget.value)}
            >
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
      <ChannelCapabilityPolicy value={policy()} onChange={setPolicy} />

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
          ? `${SURFACE_LABEL[props.surface] ?? props.surface} token saved — the bridge restarted itself`
          : `${SURFACE_LABEL[props.surface] ?? props.surface} token saved. Start the bridge yourself from the tray, or run: vak ${props.surface} --server`,
      );
      props.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(null);
    }
  };

  const remove = async () => {
    if (!window.confirm(`Remove the ${SURFACE_LABEL[props.surface] ?? props.surface} bot token? Every chat on it goes quiet until you set a new one.`)) return;
    setBusy("remove");
    try {
      await api.removeBotToken(props.surface);
      pushToast("info", `${SURFACE_LABEL[props.surface] ?? props.surface} token removed`);
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
        {configured() ? "signed in" : "no token yet"}
      </span>
      <span class="binding-meta cred-service">
        {props.state?.managed_service ? "vak restarts the bridge when you save" : "you start the bridge yourself"}
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
            placeholder={`Paste the ${SURFACE_LABEL[props.surface] ?? props.surface} bot token`}
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

/// A second (or third...) bot on a surface that already has its legacy
/// single-token slot filled — or a surface's first bot, run as a named
/// identity from the start rather than the anonymous per-surface slot.
/// Each row is its own credential *and* its own policy/permission/route
/// tier that a chat can inherit from (see `ChannelPolicy::merge` server-side).
/// A bot's own defaults — workspace, model, permission mode, and tool/skill/
/// automation policy — exactly the same controls and the same components
/// (`WorkspacePicker`, `ChannelPermissionPicker`, `ChannelCapabilityPolicy`)
/// as `ChannelAccessEditor` uses for one chat, so an operator sees one
/// consistent settings UI rather than two different ones depending on
/// whether they are editing a bot or a chat. A chat bound to this bot
/// inherits these unless it opts out (`inherit_bot_policy`) or pins its
/// own values, same as `ChannelAccessEditor`'s "Bot" picker already offers.
function BotAccessEditor(props: {
  bot: Bot;
  providers: ProviderSummary[];
  knownWorkspaces: string[];
  corePool: CorePoolEntry[];
  refresh: () => void;
}) {
  const [workspace, setWorkspace] = createSignal(props.bot.workspace ?? "");
  const [pinRoute, setPinRoute] = createSignal(!!props.bot.route);
  const [provider, setProvider] = createSignal(
    props.bot.route?.provider ?? props.providers[0]?.name ?? "",
  );
  const [model, setModel] = createSignal(props.bot.route?.model ?? "");
  const [models, setModels] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [pinPerm, setPinPerm] = createSignal(!!props.bot.permission_mode);
  const [perm, setPerm] = createSignal<PermissionMode>(props.bot.permission_mode ?? "workspace-write");
  const [policy, setPolicy] = createSignal<ChannelPolicy>(props.bot.policy ?? defaultChannelPolicy());
  const [pinVoice, setPinVoice] = createSignal(!!props.bot.voice);
  const [voiceName, setVoiceName] = createSignal(props.bot.voice?.voice_name ?? "");
  const [voicePersona, setVoicePersona] = createSignal(props.bot.voice?.persona ?? "");

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
      await api.updateBot(props.bot.id, {
        workspace: workspace().trim() || null,
        route: pinRoute() && provider() && model() ? { provider: provider(), model: model() } : null,
        permission_mode: pinPerm() ? perm() : null,
        policy: policy(),
        voice: pinVoice() ? { voice_name: voiceName() || null, persona: voicePersona() || null } : null,
      });
      pushToast("info", `${props.bot.label} updated — chats bound to it pick this up on their next message`);
      props.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      <p class="binding-meta" style="margin-bottom:6px">
        These are this bot's defaults across every chat bound to it. A chat can still override them
        for itself, or opt out of inheriting them entirely, from its own row under Chats.
      </p>
      <WorkspacePicker
        value={workspace()}
        onChange={setWorkspace}
        known={props.knownWorkspaces}
        corePool={props.corePool}
      />
      <label class="inherit-toggle">
        <input type="checkbox" checked={pinRoute()} onChange={(e) => setPinRoute(e.currentTarget.checked)} />
        Give this bot its own model (otherwise it follows the project)
      </label>
      <Show when={pinRoute()}>
        <div class="binding-controls">
          <select
            value={provider()}
            ref={(el) => syncSelect(el, provider, () => props.providers)}
            onChange={(e) => setProvider(e.currentTarget.value)}
          >
            <For each={props.providers}>{(p) => <option value={p.name}>{providerLabel(p.name)}</option>}</For>
          </select>
          <Show when={models().length} fallback={
            <input class="mono" value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="model id" />
          }>
            <select
              class="mono"
              value={model()}
              ref={(el) => syncSelect(el, model, models)}
              onChange={(e) => setModel(e.currentTarget.value)}
            >
              <For each={models()}>{(m) => <option value={m}>{m}</option>}</For>
            </select>
          </Show>
        </div>
      </Show>
      <ChannelPermissionPicker pinned={pinPerm()} setPinned={setPinPerm} mode={perm()} setMode={setPerm} subject="bot" />
      <VoiceConfigEditor
        pinned={pinVoice()}
        setPinned={setPinVoice}
        voiceName={voiceName()}
        setVoiceName={setVoiceName}
        persona={voicePersona()}
        setPersona={setVoicePersona}
        subject="bot"
      />
      <ChannelCapabilityPolicy value={policy()} onChange={setPolicy} />
      <div class="row-gap">
        <button disabled={busy()} onClick={() => void save()}>{busy() ? "Saving…" : "Save changes"}</button>
      </div>
    </div>
  );
}

function BotRow(props: {
  bot: Bot;
  providers: ProviderSummary[];
  knownWorkspaces: string[];
  corePool: CorePoolEntry[];
  refresh: () => void;
}) {
  const [editing, setEditing] = createSignal(false);
  const [draft, setDraft] = createSignal("");
  const [busy, setBusy] = createSignal<"save" | "remove" | "delete" | "settings" | null>(null);
  const [settingsOpen, setSettingsOpen] = createSignal(false);

  const save = async () => {
    const token = draft().trim();
    if (!token) return;
    setBusy("save");
    try {
      await api.putBotIdToken(props.bot.id, token);
      setEditing(false);
      setDraft("");
      pushToast("info", `${props.bot.label} token saved`);
      props.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(null);
    }
  };

  const removeToken = async () => {
    if (!window.confirm(`Remove ${props.bot.label}'s token? Its chats go quiet until you set a new one.`)) return;
    setBusy("remove");
    try {
      await api.removeBotIdToken(props.bot.id);
      pushToast("info", `${props.bot.label} token removed`);
      props.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(null);
    }
  };

  const deleteBot = async () => {
    if (!window.confirm(`Delete the bot "${props.bot.label}"? Chats bound to it fall back to no bot tier.`)) return;
    setBusy("delete");
    try {
      await api.deleteBot(props.bot.id);
      pushToast("info", `${props.bot.label} deleted`);
      props.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div class="cred-block">
      <div class="cred-row">
        <div class="cred-id">
          <SurfaceBadge channelKey={`${props.bot.surface}:`} />
          <span class="binding-meta">{props.bot.label}</span>
          <code class="binding-meta">{props.bot.token_env}</code>
        </div>
        <span class="spacer" />
        <Show
          when={editing()}
          fallback={
            <div class="row-gap">
              <button class="ghost small" onClick={() => setSettingsOpen((v) => !v)}>
                {settingsOpen() ? "Hide settings" : "Settings"}
              </button>
              <button class="ghost small" onClick={() => setEditing(true)}>Set token</button>
              <button class="danger small" disabled={busy() === "remove"} onClick={() => void removeToken()}>Remove token</button>
              <button class="danger small" disabled={busy() === "delete"} onClick={() => void deleteBot()}>Delete bot</button>
            </div>
          }
        >
          <div class="cred-edit">
            <input
              type="password"
              autocomplete="off"
              spellcheck={false}
              autofocus
              placeholder={`Paste ${props.bot.label}'s token`}
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

      <Show when={settingsOpen()}>
        <div class="cred-settings">
          <BotAccessEditor
            bot={props.bot}
            providers={props.providers}
            knownWorkspaces={props.knownWorkspaces}
            corePool={props.corePool}
            refresh={props.refresh}
          />
        </div>
      </Show>
    </div>
  );
}

/// Extra bots beyond each surface's single legacy slot, plus the form to
/// add one — this is the whole "I can't add a second bot" gap: the legacy
/// `CredentialRow` above is capped at one row per surface by construction,
/// this list is not.
function ExtraBotsList(props: { ctx: GatewayCtx }) {
  const [adding, setAdding] = createSignal(false);
  const [id, setId] = createSignal("");
  const [surface, setSurface] = createSignal<string>(KNOWN_SURFACES[0]);
  const [label, setLabel] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  const create = async () => {
    if (!id().trim()) return;
    setBusy(true);
    try {
      await api.createBot(id().trim(), surface(), label().trim() || id().trim());
      pushToast("info", `Bot "${id().trim()}" added — set its token below`);
      setAdding(false);
      setId("");
      setLabel("");
      props.ctx.refresh();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="cred-list" style={{ "margin-top": "0.75rem" }}>
      <Show when={props.ctx.bots().length > 0}>
        <For each={props.ctx.bots()}>
          {(bot) => (
            <BotRow
              bot={bot}
              providers={props.ctx.providers()}
              knownWorkspaces={props.ctx.status()?.known_workspaces ?? []}
              corePool={props.ctx.status()?.core_pool.entries ?? []}
              refresh={props.ctx.refresh}
            />
          )}
        </For>
      </Show>
      <Show
        when={adding()}
        fallback={
          <button class="ghost small" onClick={() => setAdding(true)}>+ Add another bot</button>
        }
      >
        <div class="cred-edit">
          <select value={surface()} onInput={(e) => setSurface(e.currentTarget.value)}>
            <For each={KNOWN_SURFACES}>{(s) => <option value={s}>{SURFACE_LABEL[s] ?? s}</option>}</For>
          </select>
          <input
            placeholder="id, e.g. telegram-sales"
            value={id()}
            onInput={(e) => setId(e.currentTarget.value)}
          />
          <input
            placeholder="Label (optional)"
            value={label()}
            onInput={(e) => setLabel(e.currentTarget.value)}
          />
          <button disabled={busy() || !id().trim()} onClick={() => void create()}>
            {busy() ? "Adding…" : "Add"}
          </button>
          <button class="ghost small" onClick={() => setAdding(false)}>Cancel</button>
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
  /// Extra bot identities beyond each surface's single legacy token slot
  /// (multi-bot-per-channel). A surface can be run with zero of these (the
  /// legacy slot alone) or several, each independently policed.
  bots: () => Bot[];
  botsLoading: () => boolean;
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

  // Multiple bots can share one surface (multi-bot-per-channel), so a chat
  // list with no bot column reads as if every Telegram chat talks to the
  // same bot. This resolves the label the row detail already lets you set
  // (`GatewayBindingEditor`'s bot picker) so the list itself shows it too.
  const botsById = createMemo(() => {
    const map = new Map<string, Bot>();
    for (const b of props.ctx.bots()) map.set(b.id, b);
    return map;
  });
  const botLabel = (botId: string | null | undefined) =>
    botId ? (botsById().get(botId)?.label ?? botId) : null;

  // Multi-bot-per-channel: a chat bound to a specific bot has a three-part
  // key (`surface:address:bot_id`) so the same physical chat served by
  // several bots gets independent entries. The bot id already has its own
  // column (`botLabel` above); repeating it in the chat id itself would
  // just be visual noise, so this strips that segment for display only —
  // `binding.target` (used for every API call, filter match, and the row
  // key) is never touched.
  const displayChatId = (target: string) => {
    const parts = target.split(":");
    return parts.length > 2 ? parts.slice(0, 2).join(":") : target;
  };

  const rows = createMemo(() => {
    const needle = filter().trim().toLowerCase();
    if (!needle) return props.ctx.status()?.bindings ?? [];
    return (props.ctx.status()?.bindings ?? []).filter((b) => {
      const label = botLabel(byKey().get(b.target)?.bot_id)?.toLowerCase() ?? "";
      return (
        b.target.toLowerCase().includes(needle) ||
        b.workspace.toLowerCase().includes(needle) ||
        label.includes(needle)
      );
    });
  });

  const registerManually = async () => {
    const key = manualKey().trim();
    const surface = key.split(":", 1)[0];
    if (!key.includes(":") || !(KNOWN_SURFACES as readonly string[]).includes(surface)) {
      pushToast(
        "alert",
        `That doesn't look like a chat id. Write it as app:chat-id — for example telegram:12345 — where the app is one of ${KNOWN_SURFACES.join(", ")}. Bot tokens go under Credentials, never here.`,
      );
      return;
    }
    setRegistering(true);
    try {
      await api.patchGatewayBinding(key, {});
      setManualKey("");
      pushToast("info", `Added ${key}. It uses the project defaults until you change them.`);
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
          placeholder="Find a chat or project…"
          value={filter()}
          onInput={(e) => setFilter(e.currentTarget.value)}
        />
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refresh()}>Refresh</button>
        <button onClick={() => navigate("#/gateway/connect")}>Connect a chat</button>
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
            <h2>Connected chats</h2>
            <Show when={(props.ctx.status()?.bindings.length ?? 0) > 0}>
              <p class="dim">
One row per approved chat: which project it works in, which model answers, and what it
                is allowed to do. Click a row to change any of that, or to disconnect it.
              </p>
            </Show>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.statusLoading()}>
            <table class="table">
              <thead>
                <tr><th>chat</th><th>app</th><th>bot</th><th>project</th><th>model</th><th>can do</th><th>status</th><th /></tr>
              </thead>
              <tbody><SkeletonRows cols={8} /></tbody>
            </table>
          </Match>

          <Match when={(props.ctx.status()?.bindings.length ?? 0) === 0}>
            <div class="empty empty-teach">
              <strong>No chats are connected yet.</strong>
              <p>
A connected chat — a Telegram group, a Discord channel, a Slack conversation — lets people
                talk to vak from where they already are. It takes three steps: add the bot token,
                message the bot from that chat, then approve it here.
              </p>
              <button onClick={() => navigate("#/gateway/connect")}>Connect a chat</button>
            </div>
          </Match>

          <Match when={rows().length === 0}>
            <div class="empty">
              Nothing matches “{filter()}”.{" "}
              <button class="ghost small" onClick={() => setFilter("")}>Clear filter</button>
            </div>
          </Match>

          <Match when={rows().length > 0}>
            <table class="table">
              <thead>
                <tr><th>chat</th><th>app</th><th>bot</th><th>project</th><th>model</th><th>can do</th><th>status</th><th /></tr>
              </thead>
              <tbody>
                <For each={rows()}>
                  {(binding) => {
                    const entry = () => byKey().get(binding.target);
                    const open = () => expanded() === binding.target;
                    return (
                      <>
                        <tr
                          tabindex="0"
                          classList={{ "row-open": open() }}
                          onClick={() => setExpanded(open() ? "" : binding.target)}
                          onKeyDown={(e) => e.key === "Enter" && setExpanded(open() ? "" : binding.target)}
                        >
                          <td class="mono">{displayChatId(binding.target)}</td>
                          <td><SurfaceBadge channelKey={binding.target} /></td>
                          <td>
                            <Show when={botLabel(entry()?.bot_id)} fallback={<span class="dim">—</span>}>
                              <span class="chip chip-mode">{botLabel(entry()?.bot_id)}</span>
                            </Show>
                          </td>
                          <td class="dim col-path"><PathCell path={binding.workspace} /></td>
                          <td class="mono dim">
                            {binding.effective_route.provider} / {binding.effective_route.model}
                            <Show when={binding.override}>
                              <span class="chip chip-mode" style="margin-left:6px">pinned</span>
                            </Show>
                          </td>
                          <td>
                            <Show when={entry()?.effective_permission_mode} fallback={<span class="dim">—</span>}>
                              <span class="chip chip-phrase chip-kind">{modeLabel(entry()!.effective_permission_mode)}</span>
                              <Show when={entry()!.permission_capped}>
                                <span class="chip chip-tone-warning" style="margin-left:6px" title="You asked for more than the project allows, so it was reduced to this.">reduced</span>
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
                              <span class="chip chip-tone-warning" style="margin-left:6px" title="A setting changed. The next message starts a fresh session with the new one.">new session next</span>
                            </Show>
                          </td>
                          <td><span class="chev">{open() ? "⌄" : "›"}</span></td>
                        </tr>
                        <Show when={open()}>
                          <tr class="row-detail">
                            <td colspan={8}>
                              <GatewayBindingEditor
                                binding={binding}
                                providers={props.ctx.providers()}
                                allowlistEntry={entry()}
                                knownWorkspaces={props.ctx.status()?.known_workspaces ?? []}
                                corePool={props.ctx.status()?.core_pool.entries ?? []}
                                bots={props.ctx.bots()}
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
          <summary>Add a chat by its id</summary>
          <p class="dim">
            Only needed if you already know the chat’s id. Normally a chat adds itself the first time
            it messages the bot. Write it as <code>app:chat-id</code> — for example{" "}
            <code>telegram:12345</code>. This is never a bot token; those go under Credentials.
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
          <h2 class="view-title">Connect a chat</h2>
          <p class="dim">Three steps, from a bot with no token to a chat that can talk to vak.</p>
        </div>
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refresh()}>Refresh</button>
        <button class="ghost" onClick={() => navigate("#/gateway")}>All chats</button>
      </div>

      <ol class="steps">
        <li class="step" data-state={stepState(1)}>
          <span class="step-index">1</span>
          <div class="step-body">
            <h3>Add the bot’s token</h3>
            <p class="dim">
              Get it from BotFather (Telegram), the Discord Developer Portal, or your Slack app’s
              “Bot User OAuth Token”. It is saved to your private <code>.env</code> and never shown
              again.
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
            <ExtraBotsList ctx={props.ctx} />
          </div>
        </li>

        <li class="step" data-state={stepState(2)}>
          <span class="step-index">2</span>
          <div class="step-body">
            <h3>Message the bot from the chat you want to connect</h3>
            <p class="dim">
The first message is turned away on purpose. Vak notes the chat down instead of letting it
              in, and that is what puts it in front of you in step 3.
            </p>
            <Show
              when={props.ctx.pending().length > 0}
              fallback={
                <div class="waiting-line">
                  <span class={`dot dot-${conn()}`} />
                  <Show when={credentialDone()} fallback={<span class="dim">Waiting on step 1 — no bot token is set yet.</span>}>
                    <span class="dim">Listening. Send the bot any message and the chat appears in step 3.</span>
                  </Show>
                </div>
              }
            >
              <div class="waiting-line">
                <span class="chip chip-tone-success">
                  {props.ctx.pending().length} chat{props.ctx.pending().length === 1 ? "" : "s"} waiting
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
Approving connects the chat to a project, and lets you give it its own model and its own
              limits if you want to. Anything you leave alone follows the project, and all of it can
              be changed later from Channels.
            </p>
            <Show
              when={props.ctx.pending().length > 0}
              fallback={
                <div class="empty">
                  Nothing is waiting for review.{" "}
                  <Show when={(props.ctx.status()?.bindings.length ?? 0) > 0}>
                    <button class="ghost small" onClick={() => navigate("#/gateway")}>See connected chats</button>
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
            One token per app — how vak signs in to Telegram, Discord, or Slack. Tokens are saved to
            your private <code>.env</code>; this page can set or clear one, but never shows it again.
            A chat id (like <code>telegram:12345</code>) is a different thing entirely and lives
            under Chats.
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
      <ExtraBotsList ctx={props.ctx} />
    </section>
  );
}

// ---- Gateway › Routing & pool ----------------------------------------------

/// Where a default came from, as reported by `default_route.*_source`. An
/// unrecognised source still prints its own id rather than vanishing.
const ROUTE_SOURCES: Record<string, string> = {
  config: "the config file",
  admin: "set here in the console",
  default: "vak's built-in default",
  env: "an environment variable",
  cli: "a command-line flag",
};
const sourceLabel = (source: string | undefined) =>
  source ? (ROUTE_SOURCES[source] ?? source) : "—";

function GatewayHealthView(props: { ctx: GatewayCtx }) {
  const pool = () => props.ctx.status()?.core_pool;
  const [workspace, setWorkspace] = createSignal("");
  const [workspaceDirty, setWorkspaceDirty] = createSignal(false);
  createEffect(() => {
    const current = props.ctx.status()?.workspace;
    if (current && !workspaceDirty()) setWorkspace(current);
  });
  const saveWorkspace = async () => {
    const result = await api.patchGatewayWorkspace(workspace());
    setWorkspaceDirty(false);
    props.ctx.refresh();
    if (result.restart_required) {
      window.setTimeout(() => window.location.reload(), 1200);
    }
  };
  return (
    <>
      <section class="panel gateway-summary">
        <div class="panel-title-row">
          <div>
            <h2>Defaults for connected chats</h2>
            <p class="dim">
              What a chat gets when it hasn’t been given anything of its own. Changing a default
              reaches every chat that follows it — each one picks it up on its next message rather
              than mid-conversation.
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
            <div><span class="eyebrow">Project</span><PathCell path={props.ctx.status()?.workspace ?? ""} budget={46} /></div>
            <div><span class="eyebrow">Default model</span><strong>{providerLabel(props.ctx.status()?.default_route.provider ?? "")}</strong><code>{props.ctx.status()?.default_route.model}</code></div>
            <div>
              <span class="eyebrow">Where that comes from</span>
              <strong>{sourceLabel(props.ctx.status()?.default_route.provider_source)}</strong>
              <span class="binding-meta" title={`${props.ctx.status()?.default_route.provider_source} + ${props.ctx.status()?.default_route.model_source} · ${props.ctx.status()?.default_route.revision}`}>
                model: {sourceLabel(props.ctx.status()?.default_route.model_source)}
              </span>
            </div>
          </div>
        </Show>
        <div class="panel-subsection" style="margin-top:14px">
          <div class="panel-title-row">
            <div>
              <h3>Default project folder</h3>
              <p class="dim">New gateway chats start at the canonical <code>~/vak-home</code> unless you choose another folder. Bot and chat overrides remain independent below.</p>
            </div>
            <button disabled={!workspaceDirty()} onClick={() => void saveWorkspace()}>Save folder</button>
          </div>
          <WorkspacePicker
            value={workspace() || props.ctx.status()?.workspace || ""}
            onChange={(value) => { setWorkspace(value); setWorkspaceDirty(true); }}
            known={props.ctx.status()?.known_workspaces ?? []}
            corePool={pool()?.entries ?? []}
          />
          <button class="ghost small" onClick={() => { setWorkspace(props.ctx.status()?.canonical_default_workspace ?? ""); setWorkspaceDirty(true); }}>
            Use canonical ~/vak-home
          </button>
        </div>
      </section>

      <section class="panel" style="margin-top:14px">
        <div class="panel-title-row">
          <div>
            <h2>Projects loaded right now</h2>
            <p class="dim">
              A project stays loaded and ready after it is used. One that isn’t listed simply loads
              on the next message — nothing is lost either way. Up to {pool()?.max ?? "—"} stay
              loaded, and one drops off after{" "}
              {pool()?.idle_secs == null ? "—" : describeDuration(pool()!.idle_secs)} with nothing to
              do.
            </p>
          </div>
        </div>
        <Switch>
          <Match when={props.ctx.statusLoading()}>
            <div class="cred-list"><span class="skel skel-block" /><span class="skel skel-block" /></div>
          </Match>
          <Match when={(pool()?.entries.length ?? 0) === 0}>
            <div class="empty empty-teach">
              <strong>Nothing is loaded at the moment.</strong>
              <p>
                Nothing is wrong. A project loads on the first message to one of its chats and stays
                ready here until it has been idle for{" "}
                {pool()?.idle_secs == null ? "—" : describeDuration(pool()!.idle_secs)}.
              </p>
            </div>
          </Match>
          <Match when={(pool()?.entries.length ?? 0) > 0}>
            <table class="table">
              <thead>
                <tr><th>project</th><th>can do</th><th>idle for</th><th /></tr>
              </thead>
              <tbody>
                <For each={pool()?.entries}>
                  {(entry) => (
                    <tr class="row-static">
                      <td class="col-path"><PathCell path={entry.workspace} /></td>
                      <td><span class="chip chip-phrase chip-kind">{modeLabel(entry.effective_permission_mode)}</span></td>
                      <td class="dim">{shortDuration(entry.idle_secs)}</td>
                      <td>
                        <Show when={entry.is_default}><span class="chip chip-mode">default</span></Show>
                        <span class="chip chip-tone-success" style="margin-left:6px">ready</span>
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
  { hash: "#/gateway", label: "Chats" },
  { hash: "#/gateway/connect", label: "Connect" },
  { hash: "#/gateway/credentials", label: "Credentials" },
  { hash: "#/gateway/routing", label: "Defaults & status" },
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
  const [bots, { refetch: refetchBots }] = createResource(() => api.listBots().catch(() => ({ bots: [] })));

  const refresh = () => {
    refetchStatus();
    refetchAllowlist();
    refetchSurfaces();
    refetchBots();
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
    bots: () => bots()?.bots ?? [],
    botsLoading: () => bots.loading,
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

/// Wire values are the `Debug` form the server prints (`GET /config`
/// reports `format!("{:?}", mode)`); the labels are what an operator reads.
/// Ordered least to most permissive.
const MODES: { value: string; label: string }[] = [
  { value: "ReadOnly", label: "Look, don't touch" },
  { value: "WorkspaceWrite", label: "Work inside this project" },
  { value: "FullAccess", label: "No limits" },
];

// Cosmetic labels only — the actual set of selectable providers comes from
// `GET /providers` (`vak_core::Core::provider_names`, backed by the
// registration list in `vak-llm/src/registry.rs`). A name missing here
// (a provider added to the registry without a label added here) still
// renders — just under its own registry id.
const PROVIDER_LABELS: Record<string, string> = {
  anthropic: "Anthropic (Claude)",
  openai: "OpenAI (Completions)",
  "openai-responses": "OpenAI (Responses)",
  google: "Google (Gemini)",
  openrouter: "OpenRouter",
  "opencode-zen": "OpenCode Zen",
  ollama: "Ollama (local)",
};
const providerLabel = (id: string) => PROVIDER_LABELS[id] ?? id;

/// The three rule lists, titled and explained the way they read rather than
/// by the keyword they use in config.
const RULE_SECTIONS: { decision: RuleDecision; title: string; empty: string }[] = [
  {
    decision: "deny",
    title: "Never allowed",
    empty: "Nothing is blocked outright beyond what the setting above already decides.",
  },
  {
    decision: "ask",
    title: "Always ask me first",
    empty: "Nothing is forced to check with you beyond what the setting above already decides.",
  },
  {
    decision: "allow",
    title: "Always allowed",
    empty: "Nothing is pre-approved beyond what the setting above already decides.",
  },
];

const MODE_COPY: Record<string, string> = {
  ReadOnly: "It can read and search this project, and nothing else. Every change is refused.",
  WorkspaceWrite: "It can change files inside this project on its own. Anything outside asks you first.",
  FullAccess: "Nothing is checked with you first. Only for a project you trust completely.",
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
  const [maxTurnsInput, setMaxTurnsInput] = createSignal("");
  const [savingMaxTurns, setSavingMaxTurns] = createSignal(false);
  const [togglingSubagents, setTogglingSubagents] = createSignal(false);

  let initialized = false;
  createEffect(() => {
    const c = config();
    if (c && !initialized) {
      initialized = true;
      setSelectedProvider(c.provider || "anthropic");
      setSelectedModel(c.model || "");
      setMaxTurnsInput(String(c.max_turns ?? ""));
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
      pushToast("info", `Key saved for ${providerLabel(selectedProvider())}`);
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
      <PageHeader title="Settings" description="Choose the model, decide how much the agent may do on its own, store provider keys, and keep the console tidy." />
      <div class="two-col">
        <div class="stack">
          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>Model</h2>
                <p class="dim">Which model answers, unless a chat or session picks its own.</p>
              </div>
            </div>
            <Show when={!config.loading} fallback={<div class="cred-list"><span class="skel skel-block" /><span class="skel skel-block" /></div>}>
              <div class="form-row">
                <label>Provider</label>
                <select
                  value={selectedProvider()}
                  ref={(el) => syncSelect(el, selectedProvider, () => providersData()?.providers)}
                  onChange={(e) => setSelectedProvider(e.currentTarget.value)}
                >
                  <For each={providersData()?.providers ?? []}>
                    {(p) => <option value={p.name}>{providerLabel(p.name)}</option>}
                  </For>
                </select>
              </div>
              <div class="form-row">
                <label>Model</label>
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
                  <select
                    value={selectedModel()}
                    ref={(el) => syncSelect(el, selectedModel, discoveredModels)}
                    onChange={(e) => setSelectedModel(e.currentTarget.value)}
                  >
                    <For each={discoveredModels()}>{(m) => <option value={m}>{m}</option>}</For>
                  </select>
                </Show>
              </div>
              <Show when={modelError() && discoveredModels().length === 0}>
                <p class="dim">
                  Couldn’t list models for {providerLabel(selectedProvider())} — usually because
                  its key isn’t stored yet. Add the key below, or type a model name in by hand.
                  ({modelError()})
                </p>
              </Show>
              <div class="row-gap" style="margin-top:10px">
                <button
                  disabled={!selectedModel().trim()}
                  onClick={() =>
                    void guard(
                      () => api.patchConfig({ provider: selectedProvider(), model: selectedModel() }),
                      `Now using ${providerLabel(selectedProvider())} — ${selectedModel()}`,
                    )
                  }
                >
                  Save
                </button>
                <button class="ghost small" disabled={loadingModels()} onClick={() => void discover(selectedProvider())}>
                  {loadingModels() ? "Checking…" : "Refresh the list"}
                </button>
              </div>
            </Show>
          </section>

          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>Provider key</h2>
                <p class="dim">
                  The key for {providerLabel(selectedProvider())}. It is written to your private
                  <code>.env</code> and never shown again, here or anywhere else.
                </p>
              </div>
              <span class={`chip chip-tone-${keyConfigured() ? "success" : "warning"}`}>
                {keyConfigured() ? "saved" : "not saved yet"}
              </span>
            </div>
            <div class="form-row">
              <label>Key</label>
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
                {savingKey() ? "Saving…" : "Save key"}
              </button>
              <Show when={keyConfigured()}>
                <button
                  class="danger small"
                  onClick={() =>
                    void guard(async () => {
                      if (!confirmDestructive(`Delete the stored ${providerLabel(selectedProvider())} key?`)) return;
                      await api.deleteProviderKey(selectedProvider());
                      refetchProviders();
                      setDiscoveredModels([]);
                    }, `Key deleted for ${providerLabel(selectedProvider())}`)
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
                <p class="dim">Just for this browser — nobody else sees the change.</p>
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

          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>Limits</h2>
                <p class="dim">How long a single run can go, and whether it can delegate.</p>
              </div>
            </div>
            <Show when={!config.loading} fallback={<div class="cred-list"><span class="skel skel-block" /></div>}>
              <div class="form-row">
                <label>Max turns</label>
                <input
                  class="mono"
                  type="number"
                  min="1"
                  max="1000"
                  value={maxTurnsInput()}
                  onInput={(e) => setMaxTurnsInput(e.currentTarget.value)}
                />
              </div>
              <div class="row-gap" style="margin-top:10px; margin-bottom:14px">
                <button
                  disabled={
                    savingMaxTurns() ||
                    !maxTurnsInput().trim() ||
                    !(Number(maxTurnsInput()) >= 1 && Number(maxTurnsInput()) <= 1000)
                  }
                  onClick={() => {
                    setSavingMaxTurns(true);
                    void guard(
                      () => api.patchConfig({ max_turns: Number(maxTurnsInput()) }),
                      `Max turns set to ${maxTurnsInput()}`,
                    ).finally(() => setSavingMaxTurns(false));
                  }}
                >
                  {savingMaxTurns() ? "Saving…" : "Save"}
                </button>
              </div>
              <label class="inherit-toggle">
                <input
                  type="checkbox"
                  checked={config()?.subagents ?? false}
                  disabled={togglingSubagents()}
                  onChange={(e) => {
                    const next = e.currentTarget.checked;
                    setTogglingSubagents(true);
                    void guard(
                      () => api.patchConfig({ subagents: next }),
                      next ? "Sub-agents enabled" : "Sub-agents disabled",
                    ).finally(() => setTogglingSubagents(false));
                  }}
                />
                Sub-agents — lets the agent delegate part of a run to a child agent
              </label>
            </Show>
          </section>
        </div>

        <div class="stack">
          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>How much can it do on its own?</h2>
                <p class="dim">
                  This is the answer for anything the specific rules below don’t already cover.
                </p>
              </div>
            </div>
            <div class="mode-grid">
              <For each={MODES}>
                {(m) => (
                  <button
                    class="mode-btn"
                    classList={{ active: config()?.permission_mode === m.value }}
                    onClick={() => void guard(() => api.setMode(m.value), `Now set to \u201C${m.label}\u201D`)}
                    disabled={config()?.permission_mode === m.value}
                  >
                    <span class="mode-name">
                      {m.label}
                      <Show when={config()?.permission_mode === m.value}>
                        <span class="chip chip-tone-success">current</span>
                      </Show>
                    </span>
                    <span class="mode-desc">{MODE_COPY[m.value]}</span>
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
                      This version of the server doesn’t report its specific rules, so they can’t be
                      listed here. The setting above still applies. To see the rules, open the
                      project’s <code>config.toml</code>, or update vak.
                    </p>
                  </div>
                </Show>
              }
            >
              <div class="rule-lists">
                <For each={RULE_SECTIONS}>
                  {({ decision, title, empty }) => (
                    <div>
                      <span class="eyebrow">{title}</span>
                      <Show
                        when={rulesFor(decision).length > 0}
                        fallback={<p class="dim">{empty}</p>}
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
                These are set in the project’s config file and can’t be changed from a browser — a
                console that could widen its own reach wouldn’t be worth much. Hover a rule to see
                exactly how it is written. To see what a rule grants one connected app, open{" "}
                <a href="#/integrations">Extensions</a>.
              </p>
            </Show>
          </section>

          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>Housekeeping</h2>
                <p class="dim">Safe to run any time. Both can take a moment on a big project.</p>
              </div>
            </div>
            <div class="row-gap">
              <button class="ghost" disabled={runningDoctor()} onClick={() => void runDoctor()}>
                {runningDoctor() ? "Checking…" : "Check for problems"}
              </button>
              <button class="ghost" disabled={rebuilding()} onClick={() => void rebuild()}>
                {rebuilding() ? "Rebuilding…" : "Rebuild search"}
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
                <h2>Signed in</h2>
                <p class="dim">
                  Signing out only affects this browser. Anything already running carries on.
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
  group: "Home" | "Work" | "Operations" | "Governance" | "Knowledge" | "Settings";
  badge?: () => string;
  /// Sub-destinations, rendered in the sidebar while the section is open.
  children?: readonly { readonly hash: string; readonly label: string }[];
  /// Which child is current. A section with children owns this, because only
  /// it knows how its sub-routes resolve (a bare section hash is its own
  /// first tab, not "no tab").
  activeChild?: () => string;
}

const OPERATIONS_TABS = [
  { hash: "#/operations", label: "Posture" },
  { hash: "#/operations/work", label: "Live work" },
  { hash: "#/operations/runtime", label: "Runtime & pools" },
  { hash: "#/operations/channels", label: "Channels & delivery" },
  { hash: "#/operations/automations", label: "Automations" },
  { hash: "#/operations/providers", label: "Providers" },
  { hash: "#/operations/incidents", label: "Incidents" },
] as const;

const operationsTab = () => {
  const current = route().split("?", 1)[0] || "#/overview";
  const exact = OPERATIONS_TABS.find((tab) => current === tab.hash);
  if (exact) return exact.hash;
  return OPERATIONS_TABS.slice(1).find((tab) => current.startsWith(`${tab.hash}/`))?.hash ?? "#/operations";
};

function navHref(hash: string): string {
  if (!hash.startsWith("#/operations")) return hash;
  const queryIndex = route().indexOf("?");
  return queryIndex >= 0 ? `${hash}${route().slice(queryIndex)}` : hash;
}

const NAV: NavItem[] = [
  { group: "Home", hash: "#/overview", label: "Home", icon: ICONS.overview },
  { group: "Work", hash: "#/sessions", label: "Sessions", icon: ICONS.sessions },
  { group: "Work", hash: "#/inbox", label: "Approvals", icon: ICONS.inbox, badge: () => unread().toString() || "" },
  {
    group: "Operations",
    hash: "#/operations",
    label: "Operations",
    icon: ICONS.operations,
    children: OPERATIONS_TABS,
    activeChild: operationsTab,
  },
  // Extensions are four distinct governance questions — what external
  // processes can be started, what instructions are loaded, what intercepts
  // a run, what runs unattended — and they read as four screens for the
  // same reason the Gateway does.
  {
    group: "Governance",
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
    group: "Governance",
    hash: "#/gateway",
    label: "Channels",
    icon: ICONS.gateway,
    children: GATEWAY_TABS,
    activeChild: gatewayTab,
  },
  { group: "Governance", hash: "#/security", label: "Security", icon: ICONS.security },
  { group: "Knowledge", hash: "#/memory", label: "Memory", icon: ICONS.memory },
  { group: "Knowledge", hash: "#/feeds", label: "Feeds", icon: ICONS.feeds },
  { group: "Knowledge", hash: "#/search", label: "Search", icon: ICONS.search },
  { group: "Settings", hash: "#/finops", label: "FinOps", icon: ICONS.finops },
  { group: "Settings", hash: "#/settings", label: "Settings", icon: ICONS.settings },
];

/// Source types arrive as the registry's own ids. Print the name people use.
const FEED_TYPE_LABELS: Record<string, string> = {
  rss: "RSS / Atom",
  youtube: "YouTube",
  hacker_news: "Hacker News",
  reddit: "Reddit",
  custom_http: "Custom",
};

function FeedsSection() {
  const [tab, setTab] = createSignal<"overview" | "sources" | "reader" | "alerts" | "workbench">("overview");
  const [searchQuery, setSearchQuery] = createSignal("");
  const [searchResults, setSearchResults] = createSignal<import("./types").FeedSearchResponse | null>(null);
  const [searching, setSearching] = createSignal(false);
  const [wizardOpen, setWizardOpen] = createSignal(false);
  const [alertFormOpen, setAlertFormOpen] = createSignal(false);
  const [editingSource, setEditingSource] = createSignal<string | null>(null);
  const [editInterval, setEditInterval] = createSignal("");
  const [editTrust, setEditTrust] = createSignal("");

  // Errors are surfaced (via LoadError), not swallowed to an empty/null
  // value — a script/MCP failure here used to render as an indefinite
  // "Loading…" with no way to tell a slow fetch from a broken backend.
  const [stats, { refetch: refetchStats }] = createResource(() => api.feedStats());
  const [sourceTypes] = createResource(() => api.feedSourceTypes());
  const [alerts, { refetch: refetchAlerts }] = createResource(() => api.feedAlerts());
  const [configuredSources, { refetch: refetchConfigured }] = createResource(() =>
    api.feedConfiguredSources(),
  );

  const refetchAll = () => {
    refetchStats();
    refetchConfigured();
  };

  const doSearch = async () => {
    const q = searchQuery().trim();
    if (!q) return;
    setSearching(true);
    try {
      const res = await api.feedSearch({ q, limit: 10 });
      setSearchResults(res);
    } catch (e) {
      pushToast("alert", `Search failed: ${e}`);
    }
    setSearching(false);
  };

  const toggleSource = async (src: import("./types").ConfiguredFeedSource) => {
    try {
      await api.feedUpdateSource(src.name, { enabled: !src.enabled });
      pushToast("info", `${src.name} ${src.enabled ? "disabled" : "enabled"}`);
      refetchConfigured();
    } catch (e) {
      pushToast("alert", `Could not update "${src.name}": ${e}`);
    }
  };

  const removeSource = async (name: string) => {
    if (!confirm(`Remove source "${name}"? This stops it from being checked, but keeps items already collected.`)) return;
    try {
      await api.feedDeleteSource(name);
      pushToast("info", `Source "${name}" removed`);
      refetchAll();
    } catch (e) {
      pushToast("alert", `Could not remove "${name}": ${e}`);
    }
  };

  const startEditSource = (src: import("./types").ConfiguredFeedSource) => {
    setEditingSource(src.name);
    setEditInterval(src.check_interval || "1h");
    setEditTrust(src.trust || "medium");
  };

  const saveEditSource = async (name: string) => {
    try {
      await api.feedUpdateSource(name, { interval: editInterval(), trust: editTrust() });
      pushToast("info", `"${name}" updated`);
      setEditingSource(null);
      refetchConfigured();
    } catch (e) {
      pushToast("alert", `Could not update "${name}": ${e}`);
    }
  };

  const removeAlert = async (name: string) => {
    if (!confirm(`Delete alert "${name}"?`)) return;
    try {
      await api.feedDeleteAlert(name);
      pushToast("info", `Alert "${name}" deleted`);
      refetchAlerts();
    } catch (e) {
      pushToast("alert", `Could not delete alert "${name}": ${e}`);
    }
  };

  const triggerIngest = async () => {
    try {
      const res = await api.feedIngest();
      pushToast("info", `Found ${res.new_items} new item${res.new_items === 1 ? "" : "s"} across ${res.sources_ingested} source${res.sources_ingested === 1 ? "" : "s"}`);
      refetchAll();
    } catch (e) {
      pushToast("alert", `Could not check for new items: ${e}`);
    }
  };

  return (
    <div class="view">
      <PageHeader
        title="Feeds"
        description="Sources vak reads on a schedule — blogs, YouTube channels, Reddit, Hacker News — so it can answer from them."
        actions={
          <button class="small" onClick={triggerIngest}>Check for new items</button>
        }
      />
      <div class="tab-bar">
        <button class="tab-btn" classList={{ active: tab() === "overview" }} onClick={() => setTab("overview")}>Overview</button>
        <button class="tab-btn" classList={{ active: tab() === "sources" }} onClick={() => setTab("sources")}>Sources</button>
        <button class="tab-btn" classList={{ active: tab() === "reader" }} onClick={() => setTab("reader")}>Reader</button>
        <button class="tab-btn" classList={{ active: tab() === "alerts" }} onClick={() => setTab("alerts")}>Alerts</button>
        <button class="tab-btn" classList={{ active: tab() === "workbench" }} onClick={() => setTab("workbench")}>Workbench</button>
      </div>

      <Show when={tab() === "overview"}>
        <Show when={!stats.error} fallback={<LoadError message={`${stats.error}`} onRetry={() => refetchStats()} />}>
        <Show when={stats()} fallback={<div class="empty">Loading…</div>}>
          <div class="feed-stats-row">
            <div class="feed-stat">
              <div class="value">{stats()!.total_items}</div>
              <div class="label">Items collected</div>
            </div>
            <div class="feed-stat">
              <div class="value">{stats()!.total_feeds}</div>
              <div class="label">Sources</div>
            </div>
            <div class="feed-stat">
              <div class="value">{stats()!.items_today}</div>
              <div class="label">New today</div>
            </div>
            <div class="feed-stat">
              <div class="value">{stats()!.total_alerts}</div>
              <div class="label">Alerts set up</div>
            </div>
          </div>
          <Show when={stats()!.sources && stats()!.sources!.length > 0}>
            <section class="panel">
              <div class="panel-title-row"><div><h2>Sources</h2></div></div>
              <For each={stats()!.sources}>
                {(src) => (
                  <div class="feed-source-card">
                    <div class="icon">{src.type === "hacker_news" ? "🔥" : src.type === "youtube" ? "▶" : "📡"}</div>
                    <div class="info">
                      <div class="name">{src.name}</div>
                      <div class="meta">{FEED_TYPE_LABELS[src.type] ?? src.type} · {src.item_count} items</div>
                    </div>
                  </div>
                )}
              </For>
            </section>
          </Show>
        </Show>
        </Show>
        <section class="panel">
          <div class="panel-title-row"><div><h2>Search</h2></div></div>
          <div class="toolbar">
            <input
              class="search-input"
              placeholder="Search everything you follow…"
              value={searchQuery()}
              onInput={(e) => setSearchQuery(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && doSearch()}
            />
            <button onClick={doSearch} disabled={searching()}>{searching() ? "Searching…" : "Search"}</button>
          </div>
          <Show when={searchResults()}>
            <Show when={searchResults()!.answer}>
              <div class="feed-answer-box">{searchResults()!.answer}</div>
            </Show>
            <Show when={searchResults()!.follow_up_questions && searchResults()!.follow_up_questions!.length > 0}>
              <div class="feed-follow-ups">
                <For each={searchResults()!.follow_up_questions}>
                  {(q) => (
                    <span class="feed-follow-up" onClick={() => { setSearchQuery(q); doSearch(); }}>{q}</span>
                  )}
                </For>
              </div>
            </Show>
            <For each={searchResults()!.results}>
              {(item) => (
                <div class="feed-search-result">
                  <div class="score-bar"><div class="fill" style={{ width: `${Math.round(item.score * 100)}%` }} /></div>
                  <div class="title"><a href={item.url} target="_blank">{item.title}</a></div>
                  <Show when={item.content}>
                    <div class="excerpt">{item.content.slice(0, 200)}</div>
                  </Show>
                  <div class="evidence">
                    <Show when={item.source_name}><span>{item.source_name}</span></Show>
                    <Show when={item.published_date}><span>{item.published_date}</span></Show>
                    <Show when={item.evidence?.freshness_hours != null}><span>{item.evidence!.freshness_hours}h ago</span></Show>
                  </div>
                </div>
              )}
            </For>
          </Show>
        </section>
      </Show>

      <Show when={tab() === "sources"}>
        <section class="panel">
          <div class="panel-title-row">
            <div><h2>Your sources</h2><p class="dim">What vak is currently checking, grouped by tag.</p></div>
            <button class="small" onClick={() => setWizardOpen(true)}>Add a source</button>
          </div>
          <Show when={!configuredSources.error} fallback={<LoadError message={`${configuredSources.error}`} onRetry={() => refetchConfigured()} />}>
          <Show
            when={configuredSources() && configuredSources()!.sources.length > 0}
            fallback={<div class="empty">No sources yet. Add one below to get started.</div>}
          >
            <For each={configuredSources()!.sources}>
              {(src) => (
                <div class="feed-source-card">
                  <div class="icon">{src.source_type === "hacker_news" ? "🔥" : src.source_type === "youtube" ? "▶" : "📡"}</div>
                  <div class="info">
                    <div class="name">{src.name}</div>
                    <Show
                      when={editingSource() === src.name}
                      fallback={
                        <div class="meta">
                          {FEED_TYPE_LABELS[src.source_type] ?? src.source_type}
                          <Show when={src.check_interval}> · every {src.check_interval}</Show>
                          <Show when={src.trust}> · {src.trust} trust</Show>
                          <Show when={src.url}> · <a href={src.url} target="_blank" rel="noreferrer">{src.url}</a></Show>
                        </div>
                      }
                    >
                      <div class="toolbar" style={{ "margin-top": "6px" }}>
                        <select value={editInterval()} onChange={(e) => setEditInterval(e.currentTarget.value)}>
                          <option value="5m">Every 5 minutes</option>
                          <option value="15m">Every 15 minutes</option>
                          <option value="30m">Every 30 minutes</option>
                          <option value="1h">Every hour</option>
                          <option value="6h">Every 6 hours</option>
                          <option value="1d">Daily</option>
                        </select>
                        <select value={editTrust()} onChange={(e) => setEditTrust(e.currentTarget.value)}>
                          <option value="high">High trust</option>
                          <option value="medium">Medium trust</option>
                          <option value="low">Low trust</option>
                        </select>
                        <button class="small" onClick={() => saveEditSource(src.name)}>Save</button>
                        <button class="ghost small" onClick={() => setEditingSource(null)}>Cancel</button>
                      </div>
                    </Show>
                  </div>
                  <div class="actions" style={{ display: "flex", gap: "6px", "align-items": "center" }}>
                    <span class={`chip ${src.enabled ? "chip-tone-success" : "chip-tone-danger"}`}>
                      {src.enabled ? "on" : "off"}
                    </span>
                    <button class="ghost small" onClick={() => startEditSource(src)}>Edit</button>
                    <button class="ghost small" onClick={() => toggleSource(src)}>{src.enabled ? "Disable" : "Enable"}</button>
                    <button class="ghost small" onClick={() => removeSource(src.name)}>Remove</button>
                  </div>
                </div>
              )}
            </For>
          </Show>
          </Show>
        </section>

        <section class="panel">
          <div class="panel-title-row">
            <div><h2>What you can follow</h2><p class="dim">Pick any of these when adding a source.</p></div>
          </div>
          <Show when={sourceTypes()}>
            <For each={sourceTypes()!.source_types}>
              {(st) => (
                <div class="feed-source-card">
                  <div class="icon">{st.icon === "rss" ? "📡" : st.icon === "youtube" ? "▶" : st.icon === "fire" ? "🔥" : st.icon === "reddit" ? "📱" : "🌐"}</div>
                  <div class="info">
                    <div class="name">{st.name}</div>
                    <div class="meta">{st.description} · checked every {st.default_interval} by default</div>
                  </div>
                </div>
              )}
            </For>
          </Show>
        </section>
      </Show>

      <Show when={tab() === "reader"}>
        <section class="panel">
          <div class="panel-title-row"><div><h2>Reader</h2><p class="dim">Everything collected so far, newest first.</p></div></div>
          <FeedReader />
        </section>
      </Show>

      <Show when={tab() === "alerts"}>
        <section class="panel">
          <div class="panel-title-row">
            <div><h2>Alerts</h2><p class="dim">Tell vak to flag an item when it mentions something you care about.</p></div>
            <button class="small" onClick={() => setAlertFormOpen(true)}>New alert</button>
          </div>
          <Show when={alertFormOpen()}>
            <AlertForm
              onClose={() => setAlertFormOpen(false)}
              onAdded={() => { setAlertFormOpen(false); refetchAlerts(); }}
            />
          </Show>
          <Show when={!alerts.error} fallback={<LoadError message={`${alerts.error}`} onRetry={() => refetchAlerts()} />}>
          <Show when={alerts.loading}>
            <div class="empty">Loading…</div>
          </Show>
          <Show when={!alerts.loading && (alerts()?.alerts.length ?? 0) === 0}>
            <div class="empty">No alerts set up yet.</div>
          </Show>
          <For each={alerts()?.alerts ?? []}>
            {(alert) => (
              <div class="feed-alert-card">
                <div class="info">
                  <div class="name">{alert.name}</div>
                  <div class="match">
                    <Show when={alert.match_config?.keywords?.length}>mentions {alert.match_config!.keywords!.join(", ")}</Show>
                    <Show when={alert.match_config?.tags?.length}> · tagged {alert.match_config!.tags!.join(", ")}</Show>
                    <Show when={alert.match_config?.sources?.length}> · from {alert.match_config!.sources!.join(", ")}</Show>
                  </div>
                </div>
                <div style={{ display: "flex", gap: "6px", "align-items": "center" }}>
                  <span class={`chip ${alert.enabled ? "chip-tone-success" : "chip-tone-danger"}`}>
                    {alert.enabled ? "on" : "off"}
                  </span>
                  <button class="ghost small" onClick={() => removeAlert(alert.name)}>Delete</button>
                </div>
              </div>
            )}
          </For>
          </Show>
        </section>
      </Show>

      <Show when={tab() === "workbench"}>
        <section class="panel">
          <div class="panel-title-row"><div><h2>Workbench</h2><p class="dim">Run a search and see the raw result vak works from — useful for checking why something did or didn’t come back.</p></div></div>
          <div class="form-row">
            <label>Try a search</label>
            <div class="toolbar">
              <input
                class="search-big"
                placeholder="Type a search to try…"
                value={searchQuery()}
                onInput={(e) => setSearchQuery(e.currentTarget.value)}
                onKeyDown={(e) => e.key === "Enter" && doSearch()}
              />
              <button onClick={doSearch} disabled={searching()}>Test</button>
            </div>
          </div>
          <Show when={searchResults()}>
            <pre style={{ "font-size": "11px", "max-height": "400px", overflow: "auto", "white-space": "pre-wrap" }}>
              {JSON.stringify(searchResults(), null, 2)}
            </pre>
          </Show>
        </section>
      </Show>

      <Show when={wizardOpen()}>
        <FeedWizard onClose={() => setWizardOpen(false)} onAdded={() => { setWizardOpen(false); refetchAll(); }} />
      </Show>
    </div>
  );
}

function AlertForm(props: { onClose: () => void; onAdded: () => void }) {
  const [name, setName] = createSignal("");
  const [keywords, setKeywords] = createSignal("");
  const [tags, setTags] = createSignal("");
  const [deliverTo, setDeliverTo] = createSignal("");
  const [saving, setSaving] = createSignal(false);

  const canSubmit = () => name().trim() && (keywords().trim() || tags().trim());

  const submit = async () => {
    setSaving(true);
    try {
      await api.feedAddAlert({
        name: name().trim(),
        keywords: keywords().trim() ? keywords().split(",").map((k) => k.trim()).filter(Boolean) : [],
        tags: tags().trim() ? tags().split(",").map((t) => t.trim()).filter(Boolean) : [],
        deliver_to: deliverTo().trim() || undefined,
      });
      pushToast("info", `Alert "${name()}" created`);
      props.onAdded();
    } catch (e) {
      pushToast("alert", `Could not create alert: ${e}`);
    }
    setSaving(false);
  };

  return (
    <div class="panel" style={{ "margin-bottom": "12px", background: "var(--panel-alt, rgba(255,255,255,0.03))" }}>
      <div class="form-row">
        <label>Name</label>
        <input value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="Funding rounds" />
      </div>
      <div class="form-row">
        <label>Keywords</label>
        <input value={keywords()} onInput={(e) => setKeywords(e.currentTarget.value)} placeholder="series a, seed round — comma separated" />
      </div>
      <div class="form-row">
        <label>Tags</label>
        <input value={tags()} onInput={(e) => setTags(e.currentTarget.value)} placeholder="funding, startups — comma separated" />
      </div>
      <div class="form-row">
        <label>Deliver to</label>
        <input value={deliverTo()} onInput={(e) => setDeliverTo(e.currentTarget.value)} placeholder="Chat key, webhook, or leave blank" />
      </div>
      <div style={{ display: "flex", gap: "8px", "margin-top": "8px" }}>
        <button class="ghost" onClick={props.onClose}>Cancel</button>
        <button onClick={submit} disabled={!canSubmit() || saving()}>{saving() ? "Saving…" : "Create alert"}</button>
      </div>
    </div>
  );
}

function FeedReader() {
  const [items, setItems] = createSignal<import("./types").FeedItem[]>([]);
  const [loading, setLoading] = createSignal(true);

  const loadItems = async () => {
    setLoading(true);
    try {
      const res = await api.feedItems({ limit: 50 });
      setItems(res.items || []);
    } catch (e) {
      pushToast("alert", `Could not load items: ${e}`);
    }
    setLoading(false);
  };

  createEffect(() => loadItems());

  return (
    <Show when={!loading()} fallback={<div class="empty">Loading…</div>}>
      <Show when={items().length === 0}>
        <div class="empty">Nothing collected yet. Add a source, then press “Check for new items”.</div>
      </Show>
      <For each={items()}>
        {(item) => (
          <div class="feed-item-card">
            <div class="title"><a href={item.url} target="_blank">{item.title}</a></div>
            <Show when={item.summary}>
              <div class="summary">{item.summary!.slice(0, 200)}</div>
            </Show>
            <div class="meta">
              <Show when={item.source_name}><span class="source">{item.source_name}</span></Show>
              <Show when={item.published_at}><span class="date">{item.published_at!.slice(0, 10)}</span></Show>
              <Show when={item.source_trust}><span class="trust">{item.source_trust}</span></Show>
            </div>
            <Show when={item.tags && item.tags.length > 0}>
              <div class="tags">
                <For each={item.tags!}>
                  {(tag) => <span class="chip chip-tone-info">{tag}</span>}
                </For>
              </div>
            </Show>
          </div>
        )}
      </For>
    </Show>
  );
}

function FeedWizard(props: { onClose: () => void; onAdded: () => void }) {
  const [step, setStep] = createSignal(1);
  const [selectedType, setSelectedType] = createSignal<string | null>(null);
  const [name, setName] = createSignal("");
  const [url, setUrl] = createSignal("");
  const [interval, setInterval] = createSignal("1h");
  const [tags, setTags] = createSignal("");
  const [trust, setTrust] = createSignal("medium");
  const [sourceTypes] = createResource(() => api.feedSourceTypes());

  const typeOptions = () => {
    const types = sourceTypes();
    if (!types || !types.source_types) {
      return [
        { id: "rss", label: "RSS / Atom", desc: "Any RSS or Atom feed URL" },
        { id: "youtube", label: "YouTube", desc: "Follow a channel's uploads" },
        { id: "hacker_news", label: "Hacker News", desc: "Top stories, best new, Ask HN" },
        { id: "reddit", label: "Reddit", desc: "Follow subreddits" },
        { id: "custom_http", label: "Something else", desc: "Any web address that returns data" },
      ];
    }
    return types.source_types.map((t) => ({
      id: t.id,
      label: t.name || t.id.charAt(0).toUpperCase() + t.id.slice(1).replace(/_/g, " "),
      desc: t.description || `Fetch from ${t.id}`,
    }));
  };

  const needsUrl = () => {
    const t = selectedType();
    return t && (t === "rss" || t === "youtube" || t === "custom_http");
  };

  const canSubmit = () => name() && (!needsUrl() || url());

  const submit = async () => {
    const source: import("./types").FeedSource = {
      name: name(),
      type: selectedType()!,
      url: url() || undefined,
      interval: interval(),
      tags: tags() ? tags().split(",").map((t) => t.trim()) : [],
      trust: trust(),
    };
    try {
      await api.feedAddSource(source);
      props.onAdded();
      pushToast("info", `Source "${name()}" added`);
    } catch (e) {
      pushToast("alert", `Could not add the source: ${e}`);
    }
  };

  return (
    <div class="feed-wizard-overlay" onClick={props.onClose}>
      <div class="feed-wizard" onClick={(e) => e.stopPropagation()}>
        <h2>Add a source</h2>
        <Show when={step() === 1}>
          <div class="step">
            <div class="step-label">What do you want to follow?</div>
            <div class="type-grid">
              <For each={typeOptions()}>
                {(opt) => (
                  <div
                    class={`type-option ${selectedType() === opt.id ? "selected" : ""}`}
                    onClick={() => { setSelectedType(opt.id); setStep(2); }}
                  >
                    <div class="label">{opt.label}</div>
                    <div class="desc">{opt.desc}</div>
                  </div>
                )}
              </For>
            </div>
          </div>
        </Show>
        <Show when={step() === 2}>
          <div class="step">
            <div class="step-label">Details</div>
            <div class="form-row">
              <label>Name</label>
              <input value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="My Feed" />
            </div>
            <Show when={needsUrl()}>
              <div class="form-row">
                <label>URL</label>
                <input value={url()} onInput={(e) => setUrl(e.currentTarget.value)} placeholder="https://example.com/feed.xml" />
              </div>
            </Show>
            <div class="form-row">
              <label>Check for new items</label>
              <select value={interval()} onChange={(e) => setInterval(e.currentTarget.value)}>
                <option value="5m">Every 5 minutes</option>
                <option value="15m">Every 15 minutes</option>
                <option value="30m">Every 30 minutes</option>
                <option value="1h">Every hour</option>
                <option value="6h">Every 6 hours</option>
                <option value="1d">Daily</option>
              </select>
            </div>
            <div class="form-row">
              <label>Tags</label>
              <input value={tags()} onInput={(e) => setTags(e.currentTarget.value)} placeholder="tech, news — separate with commas" />
            </div>
            <div class="form-row">
              <label>Trust level</label>
              <select value={trust()} onChange={(e) => setTrust(e.currentTarget.value)}>
                <option value="high">High — weighs in more on search/alerts</option>
                <option value="medium">Medium</option>
                <option value="low">Low — noted but discounted</option>
              </select>
            </div>
            <div style={{ "margin-top": "12px", display: "flex", gap: "8px" }}>
              <button class="ghost" onClick={() => setStep(1)}>Back</button>
              <button onClick={submit} disabled={!canSubmit()}>Add source</button>
            </div>
          </div>
        </Show>
        <div style={{ "margin-top": "12px", "text-align": "right" }}>
          <button class="ghost" onClick={props.onClose}>Cancel</button>
        </div>
      </div>
    </div>
  );
}

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
    const r = route().split("?", 1)[0] || "#/overview";
    if (r.startsWith("#/sessions/")) return "transcript";
    // Operations owns a real subtree. Resolve it before the generic
    // top-level prefix matcher so nested routes never fall through to a
    // different screen when the hash carries a query or detail segment.
    if (r === "#/operations" || r.startsWith("#/operations/")) return "#/operations";
    return NAV.find((n) => r === n.hash || r.startsWith(`${n.hash}/`))?.hash ?? "#/overview";
  };

  const operationsSection = () => {
    const r = route().split("?", 1)[0];
    if (r === "#/operations/work" || r.startsWith("#/operations/work/")) return "work" as const;
    if (r === "#/operations/runtime" || r.startsWith("#/operations/runtime/")) return "runtime" as const;
    if (r === "#/operations/channels" || r.startsWith("#/operations/channels/")) return "channels" as const;
    if (r === "#/operations/automations" || r.startsWith("#/operations/automations/")) return "automations" as const;
    if (r === "#/operations/providers" || r.startsWith("#/operations/providers/")) return "providers" as const;
    if (r === "#/operations/incidents" || r.startsWith("#/operations/incidents/")) return "incidents" as const;
    return "overview" as const;
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
                {(item, index) => (
                  <>
                    <Show when={index() === 0 || NAV[index() - 1].group !== item.group}>
                      <div class="nav-group-label">{item.group}</div>
                    </Show>
                    <a href={navHref(item.hash)} classList={{ active: currentRoute() === item.hash }}>
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
                              href={navHref(child.hash)}
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
              <Match when={currentRoute() === "#/operations"}><OperationsCenter section={operationsSection()} /></Match>
              <Match when={currentRoute() === "transcript"}>
                <Transcript sessionId={route().slice("#/sessions/".length)} />
              </Match>
              <Match when={currentRoute() === "#/integrations"}><ExtensionsSection /></Match>
              <Match when={currentRoute() === "#/gateway"}><GatewaySection /></Match>
              <Match when={currentRoute() === "#/memory"}><MemoryView /></Match>
              <Match when={currentRoute() === "#/feeds"}><FeedsSection /></Match>
              <Match when={currentRoute() === "#/search"}><SearchView /></Match>
              <Match when={currentRoute() === "#/inbox"}><Inbox /></Match>
              <Match when={currentRoute() === "#/finops"}><FinOpsView /></Match>
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
