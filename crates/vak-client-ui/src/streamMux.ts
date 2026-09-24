// The wire contract of `GET /stream` (crates/vak-server/src/stream.rs), as
// pure functions: shared by the tab, the SharedWorker and the node tests.
//
// No imports beyond types, so the worker bundle stays small and this file
// runs under `node --experimental-strip-types` as it stands.

import type { ClientEvent } from "./types";

/** What one connection carries. */
export interface Interest {
  sessions: string[];
  host: boolean;
  config: boolean;
}

export const NO_INTEREST: Interest = { sessions: [], host: false, config: false };

/** One `/stream` frame, parsed once and routed by `session`. */
export type StreamFrame =
  | { kind: "host"; data: unknown }
  | { kind: "config"; data: unknown }
  | { kind: "agent"; session: string; event: ClientEvent }
  | { kind: "presentation"; session: string; frame: unknown }
  | { kind: "side"; session: string; event: ClientEvent | null }
  | { kind: "coworking"; session: string }
  | { kind: "resync"; session: string; reason: string }
  | { kind: "unknown"; session: string };

/** `connecting`/`reconnecting` are the browser working on it; `offline`
 *  means it gave up and a retry is scheduled; `idle` means nothing is
 *  wanted, so nothing is open. */
export type StreamStatus = "idle" | "connecting" | "open" | "reconnecting" | "offline";

/** Every named event the server sends. */
export const FRAME_KINDS = ["host", "config", "agent", "presentation", "side", "coworking", "resync", "unknown"] as const;

/** Parse one named SSE event. `null` for anything malformed. */
export function parseFrame(kind: string, raw: string): StreamFrame | null {
  let data: unknown;
  try {
    data = JSON.parse(raw);
  } catch {
    return null;
  }
  if (kind === "host" || kind === "config") return { kind, data };
  if (!data || typeof data !== "object") return null;
  const body = data as Record<string, unknown>;
  if (typeof body.session !== "string") return null;
  const session = body.session;
  switch (kind) {
    case "agent":
      return { kind, session, event: body.event as ClientEvent };
    case "presentation":
      return { kind, session, frame: body.frame };
    case "side":
      return { kind, session, event: body.lagged ? null : (body.event as ClientEvent) };
    case "coworking":
    case "unknown":
      return { kind, session };
    case "resync":
      return { kind, session, reason: String(body.reason ?? "") };
    default:
      return null;
  }
}

/** The union of several interests, sessions sorted so equal sets compare equal. */
export function unionInterest(all: Iterable<Interest>): Interest {
  const sessions = new Set<string>();
  let host = false;
  let config = false;
  for (const interest of all) {
    interest.sessions.forEach((id) => sessions.add(id));
    host ||= interest.host;
    config ||= interest.config;
  }
  return { sessions: [...sessions].sort(), host, config };
}

export function sameInterest(a: Interest, b: Interest): boolean {
  return (
    a.host === b.host &&
    a.config === b.config &&
    a.sessions.length === b.sessions.length &&
    a.sessions.every((id, index) => id === b.sessions[index])
  );
}

export function isEmptyInterest(interest: Interest): boolean {
  return interest.sessions.length === 0 && !interest.host && !interest.config;
}

/** `<session>:<seq>` pairs joined by `,` — the id of every agent frame. */
export function parseCursor(raw: string): Map<string, number> {
  const cursor = new Map<string, number>();
  for (const pair of raw.split(",")) {
    const at = pair.lastIndexOf(":");
    if (at <= 0) continue;
    const seq = Number(pair.slice(at + 1));
    if (Number.isSafeInteger(seq) && seq >= 0) cursor.set(pair.slice(0, at), seq);
  }
  return cursor;
}

export function formatCursor(cursor: Map<string, number>): string {
  return [...cursor].map(([session, seq]) => `${session}:${seq}`).join(",");
}

/** The `/stream` URL for an interest, resuming only the sessions it keeps. */
export function streamPath(interest: Interest, cursor: Map<string, number>): string {
  const query = new URLSearchParams();
  interest.sessions.forEach((id) => query.append("session", id));
  if (interest.host) query.set("host", "1");
  if (interest.config) query.set("config", "1");
  const kept = new Map([...cursor].filter(([id]) => interest.sessions.includes(id)));
  if (kept.size > 0) query.set("cursor", formatCursor(kept));
  return `/stream?${query}`;
}

/** Whether a frame belongs to a subscriber with this interest. */
export function wants(interest: Interest, frame: StreamFrame): boolean {
  if (frame.kind === "host") return interest.host;
  if (frame.kind === "config") return interest.config;
  return interest.sessions.includes(frame.session);
}
