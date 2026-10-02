// Every live subscription this tab holds, on at most one connection
// (docs/design/48-web-client.md §4.7).
//
// Components never open an EventSource. They register a watcher here, the
// hub derives the tab's interest from the watchers it holds, and a transport
// carries that interest:
//
// - shared (the web build, where SharedWorker and Web Locks exist): one
//   worker holds one `/stream` for every tab of this origin, so ten tabs
//   cost the browser one of its six per-host connections, not thirty;
// - direct (the desktop shell, or a browser without them): this tab holds
//   its own single `/stream`.
//
// Imports nothing that imports the host or the API client, because both of
// them import this.

import { buildHost } from "./host/port";
import { StreamConnection } from "./streamConnection";
import { NO_INTEREST, unionInterest, type Interest, type StreamFrame, type StreamStatus } from "./streamMux";
import type { TabMessage, WorkerMessage } from "./streamWorker";
import type { ClientEvent, PresentationStreamEvent } from "./types";

export type { StreamStatus } from "./streamMux";

export interface SessionWatcher {
  agent?(event: ClientEvent): void;
  presentation?(frame: PresentationStreamEvent): void;
  side?(event: ClientEvent): void;
  /** The server lost this session's place in its replay ring: what is on
   *  screen may be missing events, and only the durable transcript can be
   *  trusted (docs/design/48-web-client.md §4.4). */
  resync?(): void;
  /** A shared candidate comment changed. */
  coworking?(): void;
  /** The conversation's Canvas was changed, here or on another surface, and
   *  is now at this revision (docs/design/66 §0a). */
  canvas?(revision: number): void;
}

const sessionWatchers = new Map<string, Set<SessionWatcher>>();
const hostWatchers = new Set<(data: unknown) => void>();
const configWatchers = new Set<(data: unknown) => void>();
const statusWatchers = new Set<(status: StreamStatus) => void>();
let status: StreamStatus = "idle";

interface Transport {
  setInterest(interest: Interest): void;
  restart(clearCursor: boolean): void;
}

let transport: Transport | null = null;
let opener: (path: string) => EventSource = (path) => new EventSource(path, { withCredentials: true });

/** How the direct transport opens its EventSource; the API client sets
 *  this so the desktop's bearer token reaches the stream. */
export function setStreamOpener(open: (path: string) => EventSource): void {
  opener = open;
}

/** Reconnect the stream now. `clearCursor` when the backend itself
 *  changed, since a new server numbers its events from scratch. */
export function restartStream(clearCursor: boolean): void {
  transport?.restart(clearCursor);
}

function interest(): Interest {
  return unionInterest([
    { sessions: [...sessionWatchers.keys()], host: hostWatchers.size > 0, config: configWatchers.size > 0 },
  ]);
}

function setStatus(next: StreamStatus) {
  status = next;
  statusWatchers.forEach((watch) => watch(next));
}

/**
 * A live presentation frame carries only `delta`; `snapshot` is present
 * only on the initial connect frame, run settlement, and an explicit resync
 * (docs/audits Finding 2 -- a full-timeline snapshot on every live frame
 * previously measured up to 9.3 MB per answer). When `snapshot` is present
 * it is cross-checked against the envelope's own `session` (the mux
 * routing is already trusted, so a delta-only frame -- which carries no
 * embeddable session id of its own -- is accepted on that routing alone).
 */
function isPresentationFrame(session: string, frame: unknown): frame is PresentationStreamEvent {
  const candidate = frame as PresentationStreamEvent | null;
  if (!candidate || typeof candidate !== "object") return false;
  const snapshot = candidate.snapshot;
  if (snapshot) {
    return (
      snapshot.schema_version === 2 &&
      snapshot.session_id === session &&
      Array.isArray(snapshot.items) &&
      Array.isArray(snapshot.diagnostics)
    );
  }
  return candidate.delta != null;
}

function dispatch(frame: StreamFrame) {
  if (frame.kind === "host") return hostWatchers.forEach((watch) => watch(frame.data));
  if (frame.kind === "config") return configWatchers.forEach((watch) => watch(frame.data));
  const watchers = sessionWatchers.get(frame.session);
  if (!watchers) return;
  for (const watcher of [...watchers]) {
    // One watcher's bug must neither vanish (the defect that once left a
    // finished turn on "Working" with nothing in the console) nor take the
    // other watchers or the stream down with it.
    try {
      switch (frame.kind) {
        case "agent":
          watcher.agent?.(frame.event);
          break;
        case "presentation":
          if (isPresentationFrame(frame.session, frame.frame)) watcher.presentation?.(frame.frame);
          break;
        case "side":
          if (frame.event) watcher.side?.(frame.event);
          break;
        case "resync":
          watcher.resync?.();
          break;
        case "coworking":
          watcher.coworking?.();
          break;
        case "canvas":
          watcher.canvas?.(frame.revision);
          break;
        case "unknown":
          break;
      }
    } catch (error) {
      console.error("vak: error handling stream frame", frame, error);
    }
  }
}

function directTransport(): Transport {
  const connection = new StreamConnection((path) => opener(path), { frame: dispatch, status: setStatus });
  return {
    setInterest: (next) => connection.setInterest(next),
    restart: (clearCursor) => connection.restart(clearCursor),
  };
}

function canShare(): boolean {
  return (
    buildHost() === "web" &&
    typeof SharedWorker === "function" &&
    typeof navigator !== "undefined" &&
    !!navigator.locks &&
    typeof crypto?.randomUUID === "function"
  );
}

function sharedTransport(): Transport {
  let port: MessagePort | null = null;
  let latest: Interest = NO_INTEREST;
  let fallback: Transport | null = null;
  const send = (message: TabMessage) => port?.postMessage(message);

  const attach = () => {
    const worker = new SharedWorker(new URL("./streamWorker.ts", import.meta.url), { name: "vak-stream" });
    const tab = crypto.randomUUID();
    const current = worker.port;
    port = current;
    // A worker that cannot start leaves this tab to hold its own stream
    // rather than none at all.
    worker.onerror = () => {
      if (port !== current || fallback) return;
      port = null;
      fallback = directTransport();
      fallback.setInterest(latest);
    };
    current.onmessage = (message: MessageEvent<WorkerMessage>) => {
      if (port !== current) return;
      if (message.data.type === "frame") dispatch(message.data.frame);
      else setStatus(message.data.status);
    };
    current.start();
    // Hold this tab's lock for its whole life; the worker waits on the same
    // name and learns of the tab's death by being granted it. The hello
    // waits for the grant so the worker's request always queues behind it.
    void navigator.locks.request(`vak-stream-tab:${tab}`, () => {
      if (port === current) {
        send({ type: "hello", tab });
        send({ type: "interest", interest: latest });
      }
      return new Promise<void>(() => {});
    });
  };

  attach();
  // A page leaving for the back/forward cache stops being a subscriber; one
  // restored from it rejoins on a fresh port.
  window.addEventListener("pagehide", () => {
    send({ type: "bye" });
    port = null;
  });
  window.addEventListener("pageshow", (event) => {
    if (event.persisted && !fallback) attach();
  });

  return {
    setInterest(next) {
      latest = next;
      if (fallback) fallback.setInterest(next);
      else send({ type: "interest", interest: next });
    },
    restart(clearCursor) {
      if (fallback) fallback.restart(clearCursor);
      else send({ type: "restart", clearCursor });
    },
  };
}

function reconcile() {
  transport ??= canShare() ? sharedTransport() : directTransport();
  transport.setInterest(interest());
}

function add<T>(set: Set<T>, value: T): () => void {
  set.add(value);
  reconcile();
  return () => {
    if (set.delete(value)) reconcile();
  };
}

/** Follow one session. Returns the unsubscribe. */
export function watchSession(session: string, watcher: SessionWatcher): () => void {
  let watchers = sessionWatchers.get(session);
  if (!watchers) {
    watchers = new Set();
    sessionWatchers.set(session, watchers);
  }
  watchers.add(watcher);
  reconcile();
  return () => {
    const current = sessionWatchers.get(session);
    if (!current?.delete(watcher)) return;
    if (current.size === 0) sessionWatchers.delete(session);
    reconcile();
  };
}

/**
 * Refresh shared-draft state when anything about the session moves: a
 * candidate comment, or any agent event (a revision the Agent prepared).
 * Coalesced so a streaming reply costs one refresh per interval, not one
 * per token.
 */
export function watchCoworking(session: string, refresh: () => void, intervalMs = 750): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const schedule = () => {
    timer ??= setTimeout(() => {
      timer = null;
      refresh();
    }, intervalMs);
  };
  const stop = watchSession(session, { agent: schedule, coworking: schedule });
  return () => {
    if (timer) clearTimeout(timer);
    stop();
  };
}

/** Host-level facts: active workspace, recents, terminal availability. */
export function watchHost(watch: (data: unknown) => void): () => void {
  return add(hostWatchers, watch);
}

/** `ConfigChanged` events from any surface. */
export function watchConfig(watch: (data: unknown) => void): () => void {
  return add(configWatchers, watch);
}

/** The shared connection's state, reported now and on every change. */
export function watchStatus(watch: (status: StreamStatus) => void): () => void {
  statusWatchers.add(watch);
  watch(status);
  return () => statusWatchers.delete(watch);
}
