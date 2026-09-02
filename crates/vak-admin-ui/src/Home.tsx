/// Home — the operator's first screen.
///
/// Home answers four questions, in this order, and nothing else:
///
///   1. Is the system healthy?      → the readiness ring, one segment per
///                                    subsystem, each backed by a real probe.
///   2. What is waiting on me?      → one ranked queue, every blocking or
///                                    asking signal in the product, merged.
///   3. What is it doing right now? → live runs, warm pool, services, the
///                                    event pulse.
///   4. What is it costing?         → today's spend against the cap, the
///                                    burn rate behind it, and what is
///                                    unpriced.
///
/// AGENTS.md invariant 26 governs every number here: this is a projection of
/// evidence, never a synthetic dashboard. A signal that could not be read is
/// `unknown` and says so; it is never folded into the healthy count, and an
/// empty attention queue states which probes produced the emptiness rather
/// than rendering a decorative green tick.
import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { api, AuthRequired } from "./api";
import {
  EVENT_LABELS, PageHeader, PathCell, SETUP_STEPS, modeLabel, providerLabel, secKindLabel,
  summarizeEvent,
} from "./display";
import {
  activity, approvalsVersion, conn, feed, navigate, observingSince, pushToast, sessionsVersion,
  setAuthed, statsVersion,
} from "./store";
import { clock, shortId, timeAgo } from "./time";
import type {
  FinOpsStatus, OnboardingState, OperationsSnapshot, PendingApproval, StepState,
} from "./types";

// ---- shared vocabulary -----------------------------------------------------

/// Every state a subsystem segment can be in. `off` is a deliberate operator
/// choice (a disabled gateway, an unset budget) and is neither healthy nor a
/// problem; `unknown` is an unread probe and is never treated as either.
type SignalState = "ok" | "warn" | "bad" | "off" | "unknown";

const STATE_WORD: Record<SignalState, string> = {
  ok: "healthy",
  warn: "needs a look",
  bad: "failing",
  off: "not configured",
  unknown: "unread",
};

interface Subsystem {
  id: string;
  label: string;
  state: SignalState;
  /// The evidence, in words. Never a restatement of the state.
  detail: string;
  href: string;
}

type Severity = "critical" | "warning" | "info";

const SEVERITY_WORD: Record<Severity, string> = {
  critical: "Blocking",
  warning: "Needs a look",
  info: "For review",
};

const SEVERITY_RANK: Record<Severity, number> = { critical: 0, warning: 1, info: 2 };

interface AttentionItem {
  id: string;
  severity: Severity;
  title: string;
  detail: string;
  action: string;
  href: string;
}

const money = (usd: number, places = 4) => `$${(usd + 0).toFixed(places)}`;

const duration = (secs: number) => {
  if (secs < 60) return `${Math.max(0, Math.round(secs))}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
  return `${Math.floor(secs / 86_400)}d ${Math.floor((secs % 86_400) / 3600)}h`;
};

const countdown = (iso: string | null | undefined): string => {
  if (!iso) return "unscheduled";
  const delta = (new Date(iso).getTime() - Date.now()) / 1000;
  if (Number.isNaN(delta)) return "unscheduled";
  return delta <= 0 ? `overdue by ${duration(-delta)}` : `in ${duration(delta)}`;
};


/// Incident fingerprints Home already derives a row of its own from.
///
/// `/ops/center` folds the same signals Home reads — failed checks, held
/// gates, the outbox, the gateway unit — into durable incident records, so
/// rendering both produced three rows for one dead-lettered queue. The
/// incident is the better *record* and the worse *prompt*: it is titled for
/// an operations timeline, not for someone deciding what to do next. Home
/// keeps its own wording and drops the duplicate; every incident still
/// appears in full on the Operations Center, and an incident from any other
/// fingerprint is surfaced here untouched.
const INCIDENTS_STATED_DIRECTLY = new Set([
  "doctor:health-checks",
  "permission:pending-approvals",
  "delivery:outbox",
  "delivery:outbox-read",
  "service:gateway",
]);

const incompleteSteps = (state: OnboardingState | null | undefined) => {
  if (!state) return [];
  // The wizard's own ordered list, so "the first thing still unfinished"
  // names the same step on both screens.
  return SETUP_STEPS.map((step) => ({ ...step, value: state[step.key] as StepState }))
    .filter((step) => step.value?.state === "incomplete");
};

// ---- the readiness ring ----------------------------------------------------

function arcPath(cx: number, cy: number, r: number, startDeg: number, endDeg: number): string {
  const point = (deg: number) => {
    const rad = ((deg - 90) * Math.PI) / 180;
    return [cx + r * Math.cos(rad), cy + r * Math.sin(rad)] as const;
  };
  const [x1, y1] = point(startDeg);
  const [x2, y2] = point(endDeg);
  const large = endDeg - startDeg > 180 ? 1 : 0;
  return `M ${x1.toFixed(2)} ${y1.toFixed(2)} A ${r} ${r} 0 ${large} 1 ${x2.toFixed(2)} ${y2.toFixed(2)}`;
}

/// One arc per subsystem, drawn from that subsystem's own probe.
///
/// State is carried by three independent channels — colour, stroke pattern,
/// and the word printed in the legend — because the ring has to stay
/// readable to someone who cannot separate the greens from the reds.
function ReadinessRing(props: { subsystems: Subsystem[]; sampledAt?: string }) {
  const size = 168;
  const centre = size / 2;
  const radius = 68;
  const gap = 5;
  const slice = createMemo(() => 360 / Math.max(1, props.subsystems.length));
  const healthy = createMemo(() => props.subsystems.filter((s) => s.state === "ok").length);
  const counted = createMemo(() => props.subsystems.filter((s) => s.state !== "off").length);
  const failing = createMemo(() => props.subsystems.filter((s) => s.state === "bad").length);
  const watching = createMemo(() => props.subsystems.filter((s) => s.state === "warn").length);
  const unread = createMemo(() => props.subsystems.filter((s) => s.state === "unknown").length);
  const off = createMemo(() => props.subsystems.filter((s) => s.state === "off").length);

  const verdict = createMemo(() => {
    if (failing() > 0) return { tone: "bad", text: `${failing()} subsystem${failing() === 1 ? "" : "s"} failing` };
    if (unread() > 0) return { tone: "unknown", text: `${unread()} probe${unread() === 1 ? "" : "s"} unread` };
    if (watching() > 0) return { tone: "warn", text: `${watching()} need${watching() === 1 ? "s" : ""} a look` };
    if (counted() === 0) return { tone: "unknown", text: "nothing configured yet" };
    return { tone: "ok", text: "all probes reporting healthy" };
  });

  return (
    <section class="panel home-ring-panel">
      <div class="panel-title-row">
        <div>
          <span class="eyebrow">Readiness</span>
          <h2>System health</h2>
          <p class="dim">One arc per subsystem, each from its own live probe.</p>
        </div>
        <Show when={props.sampledAt}>
          <span class="mono dim">sampled {clock(props.sampledAt!)}</span>
        </Show>
      </div>
      <div class="home-ring-body">
        <div class="home-ring-figure">
          <svg viewBox={`0 0 ${size} ${size}`} width={size} height={size} role="img"
            aria-label={`${healthy()} of ${counted()} subsystems healthy`}>
            <circle cx={centre} cy={centre} r={radius} fill="none" stroke="var(--border-soft)" stroke-width="13" />
            <For each={props.subsystems}>
              {(subsystem, index) => (
                <path
                  class={`ring-arc ring-arc-${subsystem.state}`}
                  d={arcPath(centre, centre, radius, index() * slice() + gap / 2, (index() + 1) * slice() - gap / 2)}
                  fill="none"
                  stroke-width="13"
                  stroke-linecap="butt"
                >
                  <title>{`${subsystem.label}: ${STATE_WORD[subsystem.state]} — ${subsystem.detail}`}</title>
                </path>
              )}
            </For>
          </svg>
          <div class="home-ring-centre">
            <strong>{healthy()}<span>/{counted()}</span></strong>
            <span>healthy</span>
          </div>
        </div>
        <div class="home-ring-legend">
          <div class={`home-ring-verdict tone-${verdict().tone}`}>{verdict().text}</div>
          <ul>
            <For each={props.subsystems}>
              {(subsystem) => (
                <li>
                  <button class="home-ring-row" onClick={() => navigate(subsystem.href)}>
                    <span class={`ring-mark ring-mark-${subsystem.state}`} aria-hidden="true" />
                    <span class="home-ring-name">{subsystem.label}</span>
                    <span class={`home-ring-state tone-${subsystem.state}`}>{STATE_WORD[subsystem.state]}</span>
                    <span class="home-ring-detail">{subsystem.detail}</span>
                  </button>
                </li>
              )}
            </For>
          </ul>
          <Show when={off() > 0}>
            <p class="dim home-ring-foot">
              {off()} subsystem{off() === 1 ? " is" : "s are"} switched off and excluded from the count.
            </p>
          </Show>
        </div>
      </div>
    </section>
  );
}

// ---- the attention queue ---------------------------------------------------

const QUEUE_VISIBLE = 8;

function AttentionQueue(props: {
  items: AttentionItem[];
  probes: number;
  sampledAt?: string;
  degraded: boolean;
}) {
  const bySeverity = (severity: Severity) => props.items.filter((item) => item.severity === severity).length;
  // Nothing is hidden — the tally above always counts the whole queue and
  // the rest is one click away. The cap exists so a bad day does not push
  // spend, live work, and the event stream three screens down.
  const [expanded, setExpanded] = createSignal(false);
  const shown = createMemo(() => (expanded() ? props.items : props.items.slice(0, QUEUE_VISIBLE)));
  const hidden = createMemo(() => props.items.length - shown().length);
  return (
    <section class="panel home-attention" classList={{ "panel-alert": bySeverity("critical") > 0 }}>
      <div class="panel-title-row">
        <div>
          <span class="eyebrow">Queue</span>
          <h2>Needs you</h2>
          <p class="dim">Everything across vak that is blocked, asking, or drifting — in one order.</p>
        </div>
        <div class="home-severity-tally">
          <span class="tally tone-bad">{bySeverity("critical")} blocking</span>
          <span class="tally tone-warn">{bySeverity("warning")} to look at</span>
          <span class="tally tone-info">{bySeverity("info")} to review</span>
        </div>
      </div>
      <Show
        when={props.items.length > 0}
        fallback={
          <div class="home-clear">
            <strong>Nothing is waiting on you.</strong>
            <p class="dim">
              {props.degraded
                ? "Some probes could not be read, so this is an incomplete answer — open Operations for the evidence."
                : `Derived from ${props.probes} live probes${props.sampledAt ? ` sampled at ${clock(props.sampledAt)}` : ""}. This is a reading, not a placeholder.`}
            </p>
          </div>
        }
      >
        <ul class="home-attention-list">
          <For each={shown()}>
            {(item) => (
              <li class={`home-attention-row sev-${item.severity}`}>
                <span class={`home-sev tone-${item.severity}`}>{SEVERITY_WORD[item.severity]}</span>
                <span class="home-attention-copy">
                  <strong>{item.title}</strong>
                  <small>{item.detail}</small>
                </span>
                <button class="ghost small" onClick={() => navigate(item.href)}>{item.action}</button>
              </li>
            )}
          </For>
        </ul>
        <Show when={hidden() > 0 || expanded()}>
          <button class="text-action home-queue-more" onClick={() => setExpanded(!expanded())}>
            {expanded() ? "Show fewer" : `Show ${hidden()} more ${hidden() === 1 ? "item" : "items"} →`}
          </button>
        </Show>
      </Show>
    </section>
  );
}

// ---- approval gates --------------------------------------------------------

/// The one thing on Home that is answered here rather than linked to. An
/// approval gate is a run that has already stopped; making the operator
/// navigate to unblock it is the whole cost of the gate.
function ApprovalGates(props: { approvals: PendingApproval[]; onAnswered: () => void }) {
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
      props.onAnswered();
    }
  };

  return (
    <section class="panel panel-alert home-gates">
      <div class="panel-title-row">
        <div>
          <span class="eyebrow">Blocked work</span>
          <h2>{props.approvals.length} run{props.approvals.length === 1 ? " has" : "s have"} stopped for your answer</h2>
          <p class="dim">Each one is paused mid-turn. Nothing else in that session moves until you answer.</p>
        </div>
      </div>
      <ul class="home-gate-list">
        <For each={props.approvals}>
          {(a) => (
            <li>
              <div class="home-gate-head">
                <span class="chip chip-tool mono">{a.tool}</span>
                <span class="mono dim">{shortId(a.session_id)}</span>
                <span class="when">waiting {timeAgo(a.requested_at)}</span>
              </div>
              <Show when={a.reason}><div class="home-gate-reason">{a.reason}</div></Show>
              <details class="home-gate-args">
                <summary>What it wants to run</summary>
                <pre class="mono">{a.args_json}</pre>
              </details>
              <div class="row-gap">
                <button class="approve" disabled={busyId() === a.request_id} onClick={() => answer(a, true)}>Approve</button>
                <button class="danger" disabled={busyId() === a.request_id} onClick={() => answer(a, false)}>Deny</button>
                <span class="spacer" />
                <button class="ghost small" onClick={() => navigate(`#/sessions/${a.session_id}`)}>Read the session</button>
              </div>
            </li>
          )}
        </For>
      </ul>
    </section>
  );
}

// ---- right now -------------------------------------------------------------

function RightNow(props: { ops: OperationsSnapshot | null; error: boolean }) {
  const runs = () => props.ops?.runs ?? [];
  const pool = () => props.ops?.pool;
  const poolFill = createMemo(() => {
    const p = pool();
    if (!p || !p.max) return 0;
    return Math.min(100, (p.entries.length / p.max) * 100);
  });
  const nextTask = createMemo(() => {
    const tasks = (props.ops?.tasks ?? []).filter((t) => t.enabled && t.next_fire);
    return [...tasks].sort((a, b) => Date.parse(a.next_fire!) - Date.parse(b.next_fire!))[0] ?? null;
  });

  return (
    <section class="panel home-now">
      <div class="panel-title-row">
        <div>
          <span class="eyebrow">Right now</span>
          <h2>What is running</h2>
          <p class="dim">Live handles, warm workers, and the next unattended job.</p>
        </div>
        <button class="ghost small" onClick={() => navigate("#/operations/work")}>Open live work</button>
      </div>
      <Show when={!props.error} fallback={<div class="error-state"><strong>Control plane unreadable</strong><p>The operations snapshot did not load, so nothing here can be reported.</p></div>}>
        <Show when={runs().length > 0} fallback={<div class="empty">No run is in flight. The agent is idle, not stuck.</div>}>
          <ul class="home-run-list">
            <For each={runs()}>
              {(run) => (
                <li>
                  <button onClick={() => navigate(`#/operations/work/runs/${encodeURIComponent(run.session_id)}`)}>
                    <span class={`run-pip run-pip-${run.state === "running" ? "live" : "held"}`} />
                    <span class="mono">{shortId(run.session_id)}</span>
                    <span class={`home-run-state tone-${run.state === "running" ? "ok" : "warn"}`}>
                      {run.state === "waiting_approval" ? "held at a gate" : run.state}
                    </span>
                    <span class="dim"><PathCell path={run.workspace} budget={26} /></span>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <dl class="home-now-grid">
          <div>
            <dt>Warm workers</dt>
            <dd>
              <span class="mono">{pool()?.entries.length ?? "—"} / {pool()?.max ?? "—"}</span>
              <div class="meter"><i style={{ width: `${poolFill()}%` }} /></div>
            </dd>
          </div>
          <div>
            <dt>Gateway service</dt>
            <dd>
              <span class={`state-word tone-${props.ops?.services.gateway_healthy ? "ok" : props.ops?.gateway.enabled ? "bad" : "off"}`}>
                {props.ops?.services.gateway.state ?? "unknown"}
              </span>
              <span class="dim">{props.ops?.gateway.enabled ? "chat is on" : "chat is off"}</span>
            </dd>
          </div>
          <div>
            <dt>Next scheduled job</dt>
            <dd>
              <Show when={nextTask()} fallback={<span class="dim">none scheduled</span>}>
                <span>{nextTask()!.name}</span>
                <span class="dim">{countdown(nextTask()!.next_fire)}</span>
              </Show>
            </dd>
          </div>
          <div>
            <dt>Server</dt>
            <dd>
              <span class="mono">v{props.ops?.server.version ?? "—"}</span>
              <span class="dim">up {props.ops ? duration(props.ops.server.uptime_secs) : "—"} · pid {props.ops?.server.pid ?? "—"}</span>
            </dd>
          </div>
        </dl>
      </Show>
    </section>
  );
}

// ---- the pulse -------------------------------------------------------------

const PULSE_SPAN_MS = 10 * 60 * 1000;
const PULSE_BUCKETS = 40;

/// Throughput off the live hub, not off the display buffer.
///
/// `feed` keeps sixty entries so the log stays readable; a rate computed
/// from sixty entries silently pins itself the moment the system gets busy,
/// which is precisely when the rate matters. `activity` in the store keeps
/// thirty minutes of bare timestamps for this panel alone.
function PulsePanel() {
  // The chart has to scroll even while nothing arrives, or a dead feed looks
  // identical to a busy one that stopped a second ago.
  const [tick, setTick] = createSignal(Date.now());
  const timer = window.setInterval(() => setTick(Date.now()), 3000);
  onCleanup(() => window.clearInterval(timer));

  const buckets = createMemo(() => {
    const now = tick();
    const width = PULSE_SPAN_MS / PULSE_BUCKETS;
    const counts = new Array(PULSE_BUCKETS).fill(0) as number[];
    for (const item of activity()) {
      const offset = now - item.ts;
      if (offset < 0 || offset >= PULSE_SPAN_MS) continue;
      const index = PULSE_BUCKETS - 1 - Math.floor(offset / width);
      if (index >= 0 && index < PULSE_BUCKETS) counts[index] += 1;
    }
    return counts;
  });

  const perMinute = createMemo(() => {
    const now = tick();
    const span = 5 * 60 * 1000;
    const since = observingSince();
    const recent = activity().filter((item) => now - item.ts < span).length;
    // The divisor never drops below a minute. Dividing three events by the
    // one second that had actually elapsed reported 176.5 events/minute on a
    // system that had just seen three — arithmetically true and completely
    // useless. A floored window under-reads for the first minute and then
    // converges on the real five-minute rate, which is the honest trade.
    const minutes = since ? Math.min(5, Math.max(1, (now - since) / 60_000)) : 5;
    return recent / minutes;
  });

  const mix = createMemo(() => {
    const counts = new Map<string, number>();
    for (const item of activity()) counts.set(item.type, (counts.get(item.type) ?? 0) + 1);
    const total = activity().length;
    return [...counts.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5)
      .map(([type, count]) => ({ type, count, share: total ? (count / total) * 100 : 0 }));
  });

  /// Name the window the rate is actually over, so a figure from the first
  /// forty seconds of a connection is not read as a steady state.
  const rateWindow = createMemo(() => {
    const since = observingSince();
    if (!since) return "no events yet";
    const observed = (tick() - since) / 60_000;
    return observed >= 5 ? "over the last 5 minutes" : `over the first ${duration(Math.max(60, (tick() - since) / 1000))}`;
  });

  const peak = createMemo(() => Math.max(1, ...buckets()));

  return (
    <section class="panel home-pulse">
      <div class="panel-title-row">
        <div>
          <span class="eyebrow">Live</span>
          <h2>Pulse</h2>
          <p class="dim">Every event the hub delivered, over the last ten minutes.</p>
        </div>
        <span class={`chip ${conn() === "live" ? "chip-tone-success" : "chip-tone-warning"}`}>{conn()}</span>
      </div>
      <div class="home-pulse-rate">
        <strong>{perMinute().toFixed(1)}</strong>
        <span>events / minute</span>
        <span class="dim">{rateWindow()}</span>
      </div>
      <div class="home-pulse-chart" role="img" aria-label={`Event rate over the last ten minutes, peak ${peak()} per bucket`}>
        <For each={buckets()}>
          {(count) => (
            <i
              style={{ height: `${Math.max(count > 0 ? 8 : 2, (count / peak()) * 100)}%` }}
              classList={{ quiet: count === 0 }}
              title={`${count} event${count === 1 ? "" : "s"}`}
            />
          )}
        </For>
      </div>
      <div class="home-pulse-axis"><span>10m ago</span><span>now</span></div>
      <Show when={mix().length > 0} fallback={<div class="empty">No events yet on this connection.</div>}>
        <div class="home-mix">
          <span class="eyebrow">Mix over the last 30 minutes</span>
          <For each={mix()}>
            {(row) => (
              <div class="home-mix-row">
                <span title={row.type}>{EVENT_LABELS[row.type] ?? row.type}</span>
                <div class="home-mix-track"><i style={{ width: `${Math.max(3, row.share)}%` }} /></div>
                <strong class="mono">{row.count}</strong>
              </div>
            )}
          </For>
        </div>
      </Show>
    </section>
  );
}

// ---- money -----------------------------------------------------------------

function SpendSpark(props: { points: { date: string; usd: number }[]; capUsd: number | null }) {
  const peak = createMemo(() => Math.max(...props.points.map((p) => p.usd), props.capUsd ?? 0, 0.0001));
  return (
    <div class="home-spend-spark" role="img" aria-label="Daily spend over the last fourteen days">
      <For each={props.points}>
        {(point) => (
          <i
            classList={{ over: props.capUsd != null && point.usd > props.capUsd, quiet: point.usd <= 0 }}
            style={{ height: `${Math.max(point.usd > 0 ? 6 : 2, (point.usd / peak()) * 100)}%` }}
            title={`${point.date}: ${money(point.usd)}`}
          />
        )}
      </For>
    </div>
  );
}

function MoneyPanel(props: { finops: FinOpsStatus | null; error: boolean }) {
  const spend = () => (props.finops?.day_usd ?? 0) + 0;
  const cap = () => props.finops?.day_cap_usd ?? null;
  const share = createMemo(() => {
    const c = cap();
    return c && c > 0 ? (spend() / c) * 100 : null;
  });

  /// Hours elapsed in the operator's own day — the cost ledger's "today" is
  /// the same local day this browser is in.
  const elapsedHours = () => {
    const now = new Date();
    return (now.getHours() * 3600 + now.getMinutes() * 60 + now.getSeconds()) / 3600;
  };
  const burn = createMemo(() => (elapsedHours() > 0.25 ? spend() / elapsedHours() : null));

  /// What the current rate actually implies, said the shortest true way.
  ///
  /// A midnight extrapolation from forty minutes of spend is arithmetic, not
  /// information — at 01:00 it reported "on pace for $104.70" against a $5
  /// cap. With a cap, the useful projection is *when the cap lands*; without
  /// one, the day-end figure is only offered once enough of the day has run
  /// to mean anything.
  const pace = createMemo<string | null>(() => {
    const rate = burn();
    if (rate == null || rate <= 0 || spend() <= 0) return null;
    const capUsd = cap();
    if (capUsd != null) {
      if (spend() >= capUsd) return "cap already reached";
      const hoursLeft = (capUsd - spend()) / rate;
      if (elapsedHours() + hoursLeft >= 24) return "cap holds at this rate";
      const at = new Date();
      at.setTime(at.getTime() + hoursLeft * 3_600_000);
      return `cap reached around ${at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
    }
    if (elapsedHours() < 3) return null;
    return `${money(rate * 24, 2)} by midnight`;
  });

  const topModel = createMemo(() => [...(props.finops?.by_model ?? [])].sort((a, b) => b.usd - a.usd)[0] ?? null);
  const topProvider = createMemo(() => [...(props.finops?.by_provider ?? [])].sort((a, b) => b.usd - a.usd)[0] ?? null);
  const unpriced = createMemo(() => {
    const f = props.finops;
    if (!f || f.total_rows === 0) return null;
    return f.unknown_rows > 0 ? { rows: f.unknown_rows, total: f.total_rows } : null;
  });

  return (
    <section class="panel home-money">
      <div class="panel-title-row">
        <div>
          <span class="eyebrow">FinOps</span>
          <h2>Spend today</h2>
          <p class="dim">Settled provider cost from work receipts, against the admission cap.</p>
        </div>
        <button class="ghost small" onClick={() => navigate("#/finops")}>Budgets</button>
      </div>
      <Show when={!props.error} fallback={<div class="error-state"><strong>Cost ledger unreadable</strong><p>No spend figure can be shown without it.</p></div>}>
        <div class="home-money-head">
          <div class="home-money-figure">
            <strong>{money(spend())}</strong>
            <span class="dim">{cap() != null ? `of a ${money(cap()!, 2)} daily cap` : "no daily cap set"}</span>
          </div>
          <Show when={share() != null}>
            <div class="home-cap">
              <div class="meter meter-tall">
                <i
                  classList={{ warn: share()! >= 80 && share()! < 100, alert: share()! >= 100 }}
                  style={{ width: `${Math.min(100, share()!)}%` }}
                />
                <span class="meter-mark" style={{ left: "80%" }} />
              </div>
              <span class={`dim ${share()! >= 80 ? "tone-warn" : ""}`}>{share()!.toFixed(0)}% used · mark at 80%</span>
            </div>
          </Show>
        </div>
        <dl class="home-money-grid">
          <div>
            <dt>Burn rate</dt>
            <dd>{burn() != null ? `${money(burn()!)} / hour` : <span class="dim">too early in the day</span>}</dd>
          </div>
          <div>
            <dt>At this rate</dt>
            <dd>{pace() ?? <span class="dim">too early to project</span>}</dd>
          </div>
          <div>
            <dt>Most expensive model</dt>
            <dd>{topModel() ? <span class="mono">{topModel()!.name || "(unknown)"}</span> : <span class="dim">no dispatches today</span>}</dd>
          </div>
          <div>
            <dt>Through</dt>
            <dd>{topProvider() ? providerLabel(topProvider()!.name) : <span class="dim">—</span>}</dd>
          </div>
        </dl>
        <Show when={(props.finops?.daily?.length ?? 0) > 1}>
          <div class="home-spend-trend">
            <span class="eyebrow">Last {props.finops!.daily.length} days</span>
            <SpendSpark points={props.finops!.daily} capUsd={cap()} />
          </div>
        </Show>
        <Show when={unpriced()}>
          <p class="home-caveat">
            {unpriced()!.rows} of {unpriced()!.total} dispatches carry no price, so the figure above is a floor, not a total.
          </p>
        </Show>
        <Show when={(props.finops?.recent_alerts?.length ?? 0) > 0}>
          <ul class="home-alert-list">
            <For each={props.finops!.recent_alerts.slice(0, 3)}>
              {(alert) => (
                <li>
                  <span class={`home-sev tone-${alert.level === "full" ? "critical" : "warning"}`}>
                    {alert.level === "full" ? "Cap reached" : "80% of cap"}
                  </span>
                  <span class="dim">{money(alert.day_total_usd)} on {timeAgo(alert.ts)}</span>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>
    </section>
  );
}

// ---- Home ------------------------------------------------------------------

export function Home() {
  // One heavy poller. `/ops/center` is the control plane's coherent sample —
  // health checks, incidents, runs, outbox, tasks, pool, services, security
  // and server facts arrive together, so Home reads it once instead of
  // stitching eight endpoints into a picture that was never true at one
  // instant. Everything else refetches off the event hub.
  //
  // Sampling it also reconciles the durable incident ledger, exactly as the
  // Operations Center's own poll does. That is the point: an incident is now
  // opened and resolved while an operator sits on Home, not only while
  // someone happens to have Operations open.
  const [ops, opsActions] = createResource(() => api.operations().catch(() => null));
  const [finops, finopsActions] = createResource(() => api.finops().catch(() => null));
  const [approvals, approvalActions] = createResource(approvalsVersion, () => api.approvals().catch(() => null));
  const [sessions] = createResource(sessionsVersion, () => api.sessions().catch(() => null));
  const [bestofn] = createResource(sessionsVersion, () => api.bestofn().catch(() => null));
  const [config] = createResource(statsVersion, () => api.config().catch(() => null));
  const [onboarding] = createResource(statsVersion, () => api.onboarding().catch(() => null));
  const [gateway] = createResource(statsVersion, () => api.gatewayStatus().catch(() => null));
  const [allowlist] = createResource(statsVersion, () => api.gatewayAllowlist().catch(() => null));
  const [proposals] = createResource(statsVersion, () => api.skillProposals().catch(() => null));
  const [inbox] = createResource(statsVersion, () => api.inbox(true, 20).catch(() => null));

  const opsTimer = window.setInterval(() => opsActions.refetch(), 10_000);
  const finopsTimer = window.setInterval(() => finopsActions.refetch(), 30_000);
  onCleanup(() => {
    window.clearInterval(opsTimer);
    window.clearInterval(finopsTimer);
  });

  // An approval granted or denied anywhere — this page, a chat, the CLI —
  // changes the held-run count in the snapshot, so re-sample rather than
  // wait out the ten-second poll. The first run is the mount, which the
  // resource has already fetched.
  let approvalsSeen = false;
  createEffect(() => {
    approvalsVersion();
    if (approvalsSeen) opsActions.refetch();
    approvalsSeen = true;
  });

  const snapshot = () => ops() ?? null;
  const opsFailed = () => !ops.loading && ops() == null;

  const failedChecks = createMemo(() => (snapshot()?.health.checks ?? []).filter((c) => c.status === "fail"));
  const openIncidents = createMemo(() =>
    (snapshot()?.incidents ?? [])
      .filter((i) => i.status !== "resolved")
      .filter((i) => !INCIDENTS_STATED_DIRECTLY.has(i.fingerprint ?? "")),
  );
  const criticalIncidents = createMemo(() => openIncidents().filter((i) => i.severity === "critical"));
  const pendingChats = createMemo(() => (allowlist()?.entries ?? []).filter((e) => e.status === "pending"));
  const staleBindings = createMemo(() => (gateway()?.bindings ?? []).filter((b) => b.stale));
  const securityToday = createMemo(
    () => (snapshot()?.security ?? []).filter((e) => Date.now() - new Date(e.ts).getTime() < 86_400_000),
  );
  const overdueTasks = createMemo(() =>
    (snapshot()?.tasks ?? []).filter(
      (t) => t.enabled && !t.running && t.next_fire && Date.now() - Date.parse(t.next_fire) > 600_000,
    ),
  );
  const providerTrouble = createMemo(() =>
    activity().filter((a) => a.type === "ProviderError" || a.type === "RateLimit").length,
  );
  const setupGaps = createMemo(() => incompleteSteps(onboarding()));
  const capShare = createMemo(() => {
    const f = finops();
    if (!f || f.day_cap_usd == null || f.day_cap_usd <= 0) return null;
    return ((f.day_usd + 0) / f.day_cap_usd) * 100;
  });

  // ---- subsystems ----------------------------------------------------------

  const subsystems = createMemo<Subsystem[]>(() => {
    const snap = snapshot();
    const unknown = (id: string, label: string, href: string): Subsystem => ({
      id, label, href, state: "unknown", detail: "probe did not answer",
    });

    const doctor: Subsystem = !snap
      ? unknown("doctor", "Doctor", "#/operations/incidents")
      : {
          id: "doctor", label: "Doctor", href: "#/operations/incidents",
          state: failedChecks().length > 0 ? "bad" : "ok",
          detail: failedChecks().length > 0
            ? failedChecks().map((c) => c.label).join(", ")
            : `${snap.health.checks.length} checks passing`,
        };

    const route: Subsystem = !snap
      ? unknown("route", "Model route", "#/settings")
      : {
          id: "route", label: "Model route", href: "#/settings",
          state: onboarding()?.provider.state === "incomplete" ? "bad" : "ok",
          detail: `${providerLabel(snap.health.provider)} · ${snap.health.model}`,
        };

    const permission: Subsystem = !snap
      ? unknown("permission", "Permissions", "#/settings")
      : {
          id: "permission", label: "Permissions", href: "#/settings",
          state: snap.health.warnings.length > 0 ? "warn" : "ok",
          detail: `${modeLabel(snap.health.permission_mode)} · ${snap.health.sandbox} sandbox`,
        };

    const gatewaySub: Subsystem = !snap
      ? unknown("gateway", "Chat gateway", "#/gateway")
      : !snap.gateway.enabled
        ? { id: "gateway", label: "Chat gateway", href: "#/gateway", state: "off", detail: "chat is switched off" }
        : {
            id: "gateway", label: "Chat gateway", href: "#/operations/runtime",
            state: snap.services.gateway_healthy ? "ok" : "bad",
            detail: `service ${snap.services.gateway.state}`,
          };

    const channels: Subsystem = !allowlist()
      ? unknown("channels", "Channels", "#/gateway")
      : pendingChats().length > 0
        ? { id: "channels", label: "Channels", href: "#/gateway", state: "warn", detail: `${pendingChats().length} chat(s) waiting to be let in` }
        : staleBindings().length > 0
          ? { id: "channels", label: "Channels", href: "#/gateway/routing", state: "warn", detail: `${staleBindings().length} binding(s) drifted from their route` }
          : (allowlist()!.entries.length === 0
              ? { id: "channels", label: "Channels", href: "#/gateway/connect", state: "off", detail: "no chat connected" }
              : { id: "channels", label: "Channels", href: "#/gateway", state: "ok", detail: `${allowlist()!.entries.filter((e) => e.status === "allowed").length} chat(s) allowed` });

    const delivery: Subsystem = !snap
      ? unknown("delivery", "Delivery", "#/operations/channels")
      : snap.outbox.error
        ? { id: "delivery", label: "Delivery", href: "#/operations/channels", state: "unknown", detail: "outbox could not be read" }
        : snap.outbox.dead_letter > 0
          ? { id: "delivery", label: "Delivery", href: "#/operations/channels", state: "bad", detail: `${snap.outbox.dead_letter} message(s) gave up` }
          : snap.outbox.pending > 0
            ? { id: "delivery", label: "Delivery", href: "#/operations/channels", state: "warn", detail: `${snap.outbox.pending} still in flight` }
            : { id: "delivery", label: "Delivery", href: "#/operations/channels", state: "ok", detail: "outbox clear" };

    const gates: Subsystem = !snap
      ? unknown("gates", "Approval gates", "#/inbox")
      : {
          id: "gates", label: "Approval gates", href: "#/inbox",
          state: snap.gateway.approvals.pending > 0 ? "warn" : "ok",
          detail: snap.gateway.approvals.pending > 0
            ? `${snap.gateway.approvals.pending} run(s) held`
            : `nothing held · chat gates ${snap.gateway.approvals.mode}`,
        };

    const budget: Subsystem = !finops()
      ? unknown("budget", "Budget", "#/finops")
      : capShare() == null
        ? { id: "budget", label: "Budget", href: "#/finops", state: "off", detail: "no daily cap set" }
        : {
            id: "budget", label: "Budget", href: "#/finops",
            state: capShare()! >= 100 ? "bad" : capShare()! >= 80 ? "warn" : "ok",
            detail: `${capShare()!.toFixed(0)}% of today's cap used`,
          };

    return [doctor, route, permission, gatewaySub, channels, delivery, gates, budget];
  });

  // ---- the queue -----------------------------------------------------------

  const attention = createMemo<AttentionItem[]>(() => {
    const items: AttentionItem[] = [];
    const snap = snapshot();

    if (opsFailed()) {
      items.push({
        id: "ops-unreadable", severity: "critical",
        title: "The control plane did not answer",
        detail: "Health, incidents, runs, and delivery state are all unknown until it does.",
        action: "Open Operations", href: "#/operations",
      });
    }

    const gaps = setupGaps();
    if (onboarding() && !onboarding()!.core_ready && gaps.length > 0) {
      const first = gaps[0];
      items.push({
        id: "setup", severity: "critical",
        title: `Setup is unfinished — ${first.title.toLowerCase()}`,
        detail: first.value.state === "incomplete" ? first.value.repair : "Finish the remaining steps.",
        action: "Finish setup", href: "#/setup",
      });
    }

    const pending = approvals()?.approvals ?? [];
    if (pending.length > 0) {
      items.push({
        id: "approvals", severity: "critical",
        title: `${pending.length} run${pending.length === 1 ? "" : "s"} stopped for approval`,
        detail: `Oldest has been waiting ${timeAgo(pending[pending.length - 1].requested_at)} on ${pending[0].tool}.`,
        action: "Answer below", href: "#/inbox",
      });
    }

    if (failedChecks().length > 0) {
      items.push({
        id: "doctor", severity: "critical",
        title: `Doctor found ${failedChecks().length} failing check${failedChecks().length === 1 ? "" : "s"}`,
        detail: failedChecks().map((c) => `${c.label}: ${c.detail}`).join(" · "),
        action: "See the evidence", href: "#/operations/incidents",
      });
    }

    for (const incident of criticalIncidents().slice(0, 3)) {
      items.push({
        id: `incident-${incident.id}`, severity: "critical",
        title: incident.title,
        detail: `${incident.detail} · seen ${incident.occurrences ?? 1}× since ${incident.first_seen ? timeAgo(incident.first_seen) : "recently"}`,
        action: "Open incident", href: `#/operations/incidents/${encodeURIComponent(incident.id)}`,
      });
    }

    // One row for the outbox, not one per state: dead-lettered and pending
    // are the same queue seen at two depths, and splitting them made a
    // single stuck adapter read as two independent problems.
    if (snap && snap.outbox.dead_letter > 0) {
      items.push({
        id: "outbox", severity: "critical",
        title: `${snap.outbox.dead_letter} repl${snap.outbox.dead_letter === 1 ? "y" : "ies"} never reached anyone`,
        detail: `The adapter exhausted its retries. The work happened; the answer did not arrive.${snap.outbox.pending > 0 ? ` ${snap.outbox.pending} more are still retrying behind them.` : ""}`,
        action: "Replay them", href: "#/operations/channels",
      });
    } else if (snap && snap.outbox.pending > 0) {
      items.push({
        id: "outbox", severity: "warning",
        title: `${snap.outbox.pending} outbound message${snap.outbox.pending === 1 ? "" : "s"} still queued`,
        detail: "Durably recorded and retrying. Nothing is lost, but nothing has landed yet.",
        action: "Watch the queue", href: "#/operations/channels",
      });
    }

    if (capShare() != null && capShare()! >= 100) {
      items.push({
        id: "cap-full", severity: "critical",
        title: "Today's budget cap is spent",
        detail: `${money(finops()!.day_usd)} against a ${money(finops()!.day_cap_usd!, 2)} cap. New work is refused at admission.`,
        action: "Adjust the cap", href: "#/finops",
      });
    } else if (capShare() != null && capShare()! >= 80) {
      items.push({
        id: "cap-warn", severity: "warning",
        title: `Budget is ${capShare()!.toFixed(0)}% spent`,
        detail: `${money(finops()!.day_usd)} of ${money(finops()!.day_cap_usd!, 2)} used today.`,
        action: "Review spend", href: "#/finops",
      });
    }

    if (pendingChats().length > 0) {
      items.push({
        id: "pending-chats", severity: "warning",
        title: `${pendingChats().length} chat${pendingChats().length === 1 ? "" : "s"} knocking`,
        detail: `Someone messaged a bot and is being held out until you decide: ${pendingChats().map((e) => e.key).slice(0, 3).join(", ")}.`,
        action: "Review access", href: "#/gateway",
      });
    }

    if (snap && snap.gateway.enabled && !snap.services.gateway_healthy) {
      items.push({
        id: "gateway-down", severity: "warning",
        title: "Chat is on, but the gateway service is not running",
        detail: `The service manager reports "${snap.services.gateway.state}". Inbound messages are not being answered.`,
        action: "Start it", href: "#/operations/runtime",
      });
    }

    if (staleBindings().length > 0) {
      items.push({
        id: "stale-bindings", severity: "warning",
        title: `${staleBindings().length} chat binding${staleBindings().length === 1 ? "" : "s"} drifted`,
        detail: (staleBindings()[0].stale_reasons ?? []).join(" · ") || "The frozen session route no longer matches the workspace.",
        action: "Open routing", href: "#/gateway/routing",
      });
    }

    for (const warning of snapshot()?.health.warnings ?? []) {
      items.push({
        id: `warning-${warning.slice(0, 24)}`, severity: "warning",
        title: "Configuration warning",
        detail: warning,
        action: "Open settings", href: "#/settings",
      });
    }

    if (providerTrouble() > 0) {
      items.push({
        id: "provider-errors", severity: "warning",
        title: `${providerTrouble()} provider error${providerTrouble() === 1 ? "" : "s"} in the last half hour`,
        detail: "Rate limits and upstream failures walk the frozen ladder, but they slow every run behind them.",
        action: "Provider posture", href: "#/operations/providers",
      });
    }

    if (overdueTasks().length > 0) {
      items.push({
        id: "overdue-tasks", severity: "warning",
        title: `${overdueTasks().length} scheduled job${overdueTasks().length === 1 ? "" : "s"} overdue`,
        detail: overdueTasks().map((t) => t.name).slice(0, 3).join(", "),
        action: "Open automations", href: "#/operations/automations",
      });
    }

    for (const incident of openIncidents().filter((i) => i.severity !== "critical").slice(0, 3)) {
      items.push({
        id: `incident-${incident.id}`, severity: "warning",
        title: incident.title,
        detail: incident.detail,
        action: "Open incident", href: `#/operations/incidents/${encodeURIComponent(incident.id)}`,
      });
    }

    const drafts = bestofn()?.total ?? 0;
    if (drafts > 0) {
      items.push({
        id: "bestofn", severity: "info",
        title: `${drafts} draft attempt${drafts === 1 ? "" : "s"} waiting on a choice`,
        detail: "The same task was tried several ways. Each branch stays until you keep or discard it.",
        action: "Pick one", href: "#/sessions",
      });
    }

    const unreadInbox = inbox()?.unread_count ?? 0;
    if (unreadInbox > 0) {
      items.push({
        id: "inbox", severity: "info",
        title: `${unreadInbox} unread in the inbox`,
        detail: (inbox()?.entries ?? []).slice(0, 2).map((e) => e.title).join(" · ") || "Proactive check-ins and alerts vak raised on its own.",
        action: "Read them", href: "#/inbox",
      });
    }

    const proposalCount = proposals()?.proposals.length ?? 0;
    if (proposalCount > 0) {
      items.push({
        id: "proposals", severity: "info",
        title: `${proposalCount} skill proposal${proposalCount === 1 ? "" : "s"} to judge`,
        detail: "vak noticed a repeated pattern and drafted a skill for it. Nothing is installed until you promote it.",
        action: "Review", href: "#/integrations/skills",
      });
    }

    if (securityToday().length > 0) {
      items.push({
        id: "security", severity: "info",
        title: `${securityToday().length} security event${securityToday().length === 1 ? "" : "s"} in 24 hours`,
        detail: [...new Set(securityToday().map((e) => secKindLabel(e.kind)))].slice(0, 4).join(" · "),
        action: "Open the log", href: "#/security",
      });
    }

    if (onboarding() && onboarding()!.core_ready && !onboarding()!.unattended_ready) {
      items.push({
        id: "unattended", severity: "info",
        title: "vak stops when you close this",
        detail: "Background services are not installed, so scheduled jobs and chat only run while a window is open.",
        action: "Set it up", href: "#/setup",
      });
    }

    return items.sort((a, b) => SEVERITY_RANK[a.severity] - SEVERITY_RANK[b.severity]);
  });

  // How many independent probes produced the queue — printed on the empty
  // state so "nothing is waiting" is a measured claim, not a decoration.
  const probeCount = createMemo(
    () => [ops(), finops(), approvals(), onboarding(), gateway(), allowlist(), proposals(), inbox(), bestofn()]
      .filter((value) => value != null).length,
  );

  const recentSessions = createMemo(() =>
    [...(sessions()?.sessions ?? [])]
      .sort((a, b) => new Date(b.last_ts).getTime() - new Date(a.last_ts).getTime())
      .slice(0, 6),
  );
  const feedItems = createMemo(() => [...feed()].reverse().slice(0, 40));

  const newSession = async () => {
    try {
      const { session_id } = await api.createSession();
      navigate(`#/sessions/${session_id}`);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  const refreshAll = () => {
    opsActions.refetch();
    finopsActions.refetch();
    approvalActions.refetch();
    pushToast("info", "Re-read every probe");
  };

  const blocking = createMemo(() => attention().filter((i) => i.severity === "critical").length);
  const headline = createMemo(() => {
    if (opsFailed()) return "The control plane is not answering.";
    if (blocking() > 0) return `${blocking()} thing${blocking() === 1 ? "" : "s"} ${blocking() === 1 ? "is" : "are"} blocking work.`;
    if (attention().length > 0) return `${attention().length} thing${attention().length === 1 ? "" : "s"} to look at.`;
    return "Everything is clear.";
  });

  return (
    <div class="view home-view">
      <PageHeader
        title="Home"
        description="Health, what needs you, what is running, and what it costs."
        actions={<>
          <button class="ghost" onClick={refreshAll}>Refresh</button>
          <button onClick={newSession}>+ New session</button>
        </>}
      />

      <section class="home-masthead" data-tone={opsFailed() || blocking() > 0 ? "bad" : attention().length > 0 ? "warn" : "ok"}>
        <div class="home-masthead-main">
          <div class="hero-kicker">
            <span class={`dot dot-${conn()}`} /> {conn() === "live" ? "Live telemetry" : `Telemetry ${conn()}`}
            <Show when={snapshot()}>
              <span class="dim"> · sampled {clock(snapshot()!.generated_at)}</span>
            </Show>
          </div>
          <h2>{headline()}</h2>
          <Show when={snapshot()} fallback={<p>Reading the control plane…</p>}>
            <ul class="home-facts">
              <li><span>Working in</span><PathCell path={snapshot()!.server.cwd} budget={40} /></li>
              <li>
                <span>Answering with</span>
                {providerLabel(config()?.provider ?? snapshot()!.health.provider)}
                <em class="mono">{config()?.model ?? snapshot()!.health.model}</em>
              </li>
              <li><span>Allowed to</span>{modeLabel(config()?.permission_mode ?? snapshot()!.health.permission_mode)}</li>
              <li><span>Contained by</span>{snapshot()!.health.sandbox}</li>
            </ul>
          </Show>
        </div>
        <div class="home-masthead-side">
          <button class="ghost small" onClick={() => navigate("#/settings")}>Change the model</button>
          <button class="ghost small" onClick={() => navigate("#/operations")}>Operations Center</button>
        </div>
      </section>

      <div class="home-top">
        <ReadinessRing subsystems={subsystems()} sampledAt={snapshot()?.generated_at} />
        <AttentionQueue
          items={attention()}
          probes={probeCount()}
          sampledAt={snapshot()?.generated_at}
          degraded={opsFailed()}
        />
      </div>

      <Show when={(approvals()?.approvals.length ?? 0) > 0}>
        <ApprovalGates approvals={approvals()!.approvals} onAnswered={() => approvalActions.refetch()} />
      </Show>

      <div class="home-grid">
        <RightNow ops={snapshot()} error={opsFailed()} />
        <PulsePanel />
        <MoneyPanel finops={finops() ?? null} error={!finops.loading && finops() == null} />

        <section class="panel home-history">
          <div class="panel-title-row">
            <div>
              <span class="eyebrow">History</span>
              <h2>Recent sessions</h2>
              <p class="dim">The latest conversations, across every project this store indexes.</p>
            </div>
            <button class="ghost small" onClick={() => navigate("#/sessions")}>View all</button>
          </div>
          <Show when={recentSessions().length > 0} fallback={<div class="empty">No sessions recorded yet.</div>}>
            <div class="recent-sessions">
              <For each={recentSessions()}>
                {(session) => (
                  <button class="recent-session" onClick={() => navigate(`#/sessions/${session.session_id}`)}>
                    <span class="session-pulse" />
                    <span class="mono">{shortId(session.session_id)}</span>
                    <span class="session-entries">{session.entry_count} message{session.entry_count === 1 ? "" : "s"}</span>
                    <span class="when">{timeAgo(session.last_ts)}</span>
                  </button>
                )}
              </For>
            </div>
          </Show>
          <Show when={sessions()}>
            <p class="dim home-ring-foot">
              {sessions()!.total.toLocaleString()} session{sessions()!.total === 1 ? "" : "s"} indexed,{" "}
              {sessions()!.sessions.reduce((a, s) => a + s.entry_count, 0).toLocaleString()} message
              {sessions()!.sessions.reduce((a, s) => a + s.entry_count, 0) === 1 ? "" : "s"} recorded
              {sessions()!.total > sessions()!.sessions.length ? ` across the ${sessions()!.sessions.length} most recent` : ""}.
            </p>
          </Show>
        </section>

        <section class="panel home-feed-panel">
          <div class="panel-title-row">
            <div>
              <span class="eyebrow">Stream</span>
              <h2>Happening now</h2>
              <p class="dim">Every event as it arrives, newest first.</p>
            </div>
            <span class="chip chip-tone-info">{feed().length} held</span>
          </div>
          <Show when={feedItems().length > 0} fallback={<div class="empty">Waiting for the first event on this connection.</div>}>
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
