import { createSignal } from "solid-js";
import type { SystemEvent } from "./types";

export const [authed, setAuthed] = createSignal<boolean | null>(null); // null = probing
export const [route, setRoute] = createSignal<string>(location.hash || "#/overview");

export type ConnState = "connecting" | "live" | "down";
export const [conn, setConn] = createSignal<ConnState>("connecting");

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
    setTimeout(connectEvents, backoffMs);
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
