/// <reference lib="webworker" />
// Holds the ONE `/stream` connection every web-client tab of this origin
// shares (docs/design/48-web-client.md §4.7).
//
// Each tab posts the interest it has; this worker opens one stream for the
// union and forwards each frame to the tabs that asked for its session. A tab
// that closes, crashes or is discarded cannot say so reliably, so each holds
// a Web Lock for its lifetime and this worker waits on the same lock: it is
// granted exactly when the tab is gone, and that tab's interest is dropped.

import { StreamConnection } from "./streamConnection";
import { NO_INTEREST, unionInterest, wants, type Interest, type StreamFrame, type StreamStatus } from "./streamMux";

declare const self: SharedWorkerGlobalScope;

export type TabMessage =
  | { type: "hello"; tab: string }
  | { type: "interest"; interest: Interest }
  | { type: "restart"; clearCursor: boolean }
  | { type: "bye" };

export type WorkerMessage =
  | { type: "frame"; frame: StreamFrame }
  | { type: "status"; status: StreamStatus };

const tabs = new Map<MessagePort, Interest>();
let status: StreamStatus = "idle";
/** Latest snapshot-bearing frame per session, for a tab that joins a
 *  subscription another tab already holds: the server only sends a
 *  snapshot on connect, run settlement or resync, and an ordinary live
 *  frame in between carries only `delta` (docs/audits Finding 2). Only a
 *  snapshot-bearing frame is a valid replacement for a fresh subscriber --
 *  a cached delta has nothing to apply itself onto. */
const lastPresentation = new Map<string, StreamFrame>();
let lastHost: StreamFrame | null = null;

function hasSnapshot(frame: StreamFrame): boolean {
  if (frame.kind !== "presentation") return false;
  const payload = frame.frame as { snapshot?: unknown } | null;
  return !!payload?.snapshot;
}

const connection = new StreamConnection(
  (path) => new EventSource(path, { withCredentials: true }),
  {
    frame(frame) {
      if (frame.kind === "presentation" && hasSnapshot(frame)) lastPresentation.set(frame.session, frame);
      if (frame.kind === "host") lastHost = frame;
      for (const [port, interest] of tabs) {
        if (wants(interest, frame)) post(port, { type: "frame", frame });
      }
    },
    status(next) {
      status = next;
      for (const port of tabs.keys()) post(port, { type: "status", status });
    },
  },
);

function post(port: MessagePort, message: WorkerMessage) {
  port.postMessage(message);
}

function reconcile() {
  const union = unionInterest(tabs.values());
  for (const id of [...lastPresentation.keys()]) {
    if (!union.sessions.includes(id)) lastPresentation.delete(id);
  }
  if (!union.host) lastHost = null;
  connection.setInterest(union);
}

function setInterest(port: MessagePort, interest: Interest) {
  const before = tabs.get(port) ?? NO_INTEREST;
  tabs.set(port, interest);
  // Subscriptions already on the wire will not be re-snapshotted for this
  // tab, so hand it the latest one now.
  const live = connection.current();
  for (const id of interest.sessions) {
    if (before.sessions.includes(id) || !live.sessions.includes(id)) continue;
    const cached = lastPresentation.get(id);
    if (cached) post(port, { type: "frame", frame: cached });
  }
  if (interest.host && !before.host && live.host && lastHost) post(port, { type: "frame", frame: lastHost });
  reconcile();
}

function drop(port: MessagePort) {
  if (!tabs.delete(port)) return;
  port.close();
  reconcile();
}

self.onconnect = (event) => {
  const port = event.ports[0];
  port.onmessage = (message: MessageEvent<TabMessage>) => {
    const data = message.data;
    switch (data.type) {
      case "hello":
        if (!tabs.has(port)) tabs.set(port, NO_INTEREST);
        post(port, { type: "status", status });
        void navigator.locks.request(`vak-stream-tab:${data.tab}`, () => drop(port));
        break;
      case "interest":
        if (tabs.has(port)) setInterest(port, data.interest);
        break;
      case "restart":
        connection.restart(data.clearCursor);
        break;
      case "bye":
        drop(port);
        break;
    }
  };
  port.start();
};
