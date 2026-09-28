import { createSignal } from "solid-js";
import { api, AuthRequired } from "./api";
import type { SystemEvent } from "./types";

export const [authed, setAuthed] = createSignal<boolean | null>(null); // null = probing
export const [route, setRoute] = createSignal<string>(location.hash || "#/overview");

export type ConnState = "connecting" | "live" | "down";
export const [conn, setConn] = createSignal<ConnState>("connecting");

export interface AgentScopeItem {
  id: string;
  name: string;
  personality?: string;
  lifecycle?: string;
}

export const [adminAgents, setAdminAgents] = createSignal<AgentScopeItem[]>([
  { id: "vak", name: "Vakyartha", personality: "General Purpose Assistant", lifecycle: "active" },
]);

export const [selectedAgentId, setSelectedAgentId] = createSignal<string>(
  localStorage.getItem("vak_admin_selected_agent") || "global",
);

/** A concrete Agent as an `?agent=` query value. Aggregate and shared-default
 * views are handled by their endpoint-specific selectors, so neither sentinel
 * may be sent as an Agent id. Use `selectedAgentId` as a Solid resource source:
 * `undefined` here is a valid API scope, but it suppresses a resource fetch. */
export const selectedAgentIdOrUndefined = () => {
  const id = selectedAgentId();
  return id === "global" || id === "all" ? undefined : id;
};

export async function refreshAdminAgents() {
  try {
    const res = await api.agents();
    const list = res?.agents;
    if (Array.isArray(list) && list.length > 0) {
      const hasVak = list.some((a) => a.id === "vak");
      const full = hasVak
        ? list
        : [{ id: "vak", name: "Vakyartha", personality: "General Purpose Assistant", lifecycle: "active" }, ...list];
      setAdminAgents(full);
    }
  } catch (err) {
    console.warn("Failed to load agent catalogue", err);
  }
}

/** Fetch per-agent data without flooding a local server when the roster grows. */
export async function mapAdminAgents<T>(load: (agent: AgentScopeItem) => Promise<T>, concurrency = 8): Promise<Array<{ agent: AgentScopeItem; value: T }>> {
  const agents = adminAgents();
  const rows: Array<{ agent: AgentScopeItem; value: T }> = [];
  for (let start = 0; start < agents.length; start += concurrency) {
    const batch = agents.slice(start, start + concurrency);
    rows.push(...await Promise.all(batch.map(async (agent) => ({ agent, value: await load(agent) }))));
  }
  return rows;
}

export interface FeedItem {
  id: number;
  ts: string;
  event: SystemEvent;
}

const [feed, setFeed] = createSignal<FeedItem[]>([]);
export { feed };

let feedSeq = 0;

// Toasts: transient pop-ups for high-signal events.
export interface Toast {
  id: number;
  kind: "info" | "warn" | "alert";
  text: string;
}
const [toasts, setToasts] = createSignal<Toast[]>([]);
export { toasts };

export function pushToast(kind: Toast["kind"], text: string) {
  const t: Toast = { id: ++feedSeq, kind, text };
  setToasts((prev) => [...prev.slice(-4), t]);
  setTimeout(() => setToasts((prev) => prev.filter((x) => x.id !== t.id)), 6000);
}

function describe(ev: SystemEvent): { kind: Toast["kind"]; text: string } | null {
  switch (ev.type) {
    case "Agent":
      return {
        kind: ev.data.summary.startsWith("failed") ? "alert" : "info",
        text: `run ${ev.data.summary}`,
      };
    case "SessionCreated":
      return { kind: "info", text: `new session ${ev.data.session_id.slice(0, 8)}` };
    case "GatewayInbound":
      return { kind: "info", text: `${ev.data.surface} · ${ev.data.who}: ${ev.data.preview}` };
    case "ApprovalRequested":
      return { kind: "warn", text: `approval: ${ev.data.tool} (${ev.data.reason || "gate"})` };
    case "ApprovalGranted":
      return { kind: "info", text: `granted ${ev.data.tool}` };
    case "ApprovalDenied":
      return { kind: "info", text: `denied ${ev.data.tool}` };
    case "SecurityEvent":
      return { kind: "alert", text: `${ev.data.kind} · ${ev.data.label}` };
    case "RateLimit":
      return { kind: "warn", text: `rate limited on ${ev.data.provider}` };
    case "ProviderError":
      return { kind: "alert", text: `${ev.data.provider} error` };
    case "Lagged":
      return { kind: "warn", text: `${ev.data.missed} events missed — reconnecting` };
    default:
      return null;
  }
}

/// A 30-minute rolling record of *every* event the hub delivered, kept
/// separately from `feed` because `feed` is a 60-item display buffer. Rate
/// and mix read off a 60-item window are wrong the moment the system gets
/// busy — which is exactly when someone is looking at them — so the pulse
/// on Home reads this instead. Only the timestamp and the variant tag are
/// retained; payloads stay in `feed`.
export interface Pulse {
  ts: number;
  type: string;
}
const PULSE_WINDOW_MS = 30 * 60 * 1000;
const PULSE_MAX = 6000;
const [activity, setActivity] = createSignal<Pulse[]>([]);
export { activity };

/// Wall-clock of the first event this connection saw, so a rate can say
/// what window it is a rate over instead of implying "since forever".
const [observingSince, setObservingSince] = createSignal<number | null>(null);
export { observingSince };

function recordPulse(type: string) {
  const now = Date.now();
  setObservingSince((prev) => prev ?? now);
  setActivity((prev) => {
    const cutoff = now - PULSE_WINDOW_MS;
    const next = prev.length >= PULSE_MAX ? prev.slice(prev.length - PULSE_MAX + 1) : prev.slice();
    next.push({ ts: now, type });
    // Trimming from the front is O(n) but only ever walks the expired
    // prefix; the array is already ordered by arrival.
    let drop = 0;
    while (drop < next.length && next[drop].ts < cutoff) drop++;
    return drop > 0 ? next.slice(drop) : next;
  });
}

// Bumped whenever approval-related events arrive; consumers createResource
// on this to refetch the pending list live.
const [approvalsVersion, bumpApprovals] = createSignal(0);
const [sessionsVersion, bumpSessions] = createSignal(0);
const [statsVersion, bumpStats] = createSignal(0);
const [bestofnVersion, bumpBestofn] = createSignal(0);
export { approvalsVersion, sessionsVersion, statsVersion, bestofnVersion };

export function ingest(ev: SystemEvent) {
  const d = describe(ev);
  if (d) pushToast(d.kind, d.text);
  recordPulse(ev.type);
  setFeed((prev) => [...prev.slice(-59), { id: ++feedSeq, ts: new Date().toISOString(), event: ev }]);
  if (ev.type.startsWith("Approval")) bumpApprovals((v) => v + 1);
  if (ev.type === "SessionCreated" || ev.type === "SessionEntryAppended" || ev.type === "Agent" || ev.type === "ConfigChanged") {
    bumpSessions((v) => v + 1);
    bumpStats((v) => v + 1);
    bumpBestofn((v) => v + 1);
  }
}

let source: EventSource | null = null;
let backoffMs = 1000;

export function connectEvents() {
  source?.close();
  setConn("connecting");
  source = new EventSource("/admin/api/events");
  source.onopen = () => {
    backoffMs = 1000;
    setConn("live");
  };
  source.onmessage = (m) => {
    try {
      ingest(JSON.parse(m.data) as SystemEvent);
    } catch {
      // ignore malformed frames
    }
  };
  source.onerror = () => {
    setConn("down");
    source?.close();
    source = null;
    // EventSource's error event carries no HTTP status, so a closed cookie
    // (server restarted with a fresh token, or a full self uninstall
    // --purge + reinstall under an already-open tab) looks identical to a
    // dropped connection here -- it just reconnects forever, silently
    // failing every time, with the rest of the page still showing
    // whatever it last rendered before the cookie went stale. A real
    // fetch DOES carry a status, so use one as a side-channel auth probe
    // before assuming this is transient and retrying.
    api
      .config()
      .then(() => setTimeout(connectEvents, backoffMs))
      .catch((err) => {
        if (err instanceof AuthRequired) {
          setAuthed(false);
          return;
        }
        setTimeout(connectEvents, backoffMs);
      });
    backoffMs = Math.min(backoffMs * 2, 15000);
  };
}

export function disconnectEvents() {
  source?.close();
  source = null;
  setConn("down");
}

window.addEventListener("hashchange", () => {
  setRoute(location.hash || "#/overview");
});

export function navigate(to: string) {
  location.hash = to;
}
