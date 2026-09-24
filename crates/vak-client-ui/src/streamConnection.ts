// One `/stream` EventSource for whatever interest it is currently given.
//
// Used directly by a tab that cannot share (the desktop shell, a browser
// without SharedWorker or Web Locks) and by the SharedWorker that lets every
// tab share one (streamWorker.ts). Either way it is the only EventSource the
// client opens: a browser allows six HTTP/1.1 connections per host across
// ALL tabs, so each extra long-lived stream is a fetch slot taken for good.

import {
  FRAME_KINDS,
  NO_INTEREST,
  isEmptyInterest,
  parseCursor,
  parseFrame,
  sameInterest,
  streamPath,
  type Interest,
  type StreamFrame,
  type StreamStatus,
} from "./streamMux";

export interface StreamSink {
  frame(frame: StreamFrame): void;
  status(status: StreamStatus): void;
}

/** A change of interest waits this long so a burst (a session switch that
 *  drops one session and adds another, several tabs registering at once)
 *  reopens the stream once rather than once per step. */
const COALESCE_MS = 50;
const RETRY_MIN_MS = 1_000;
const RETRY_MAX_MS = 15_000;

export class StreamConnection {
  private source: EventSource | null = null;
  private applied: Interest = NO_INTEREST;
  private wanted: Interest = NO_INTEREST;
  private cursor = new Map<string, number>();
  private pending: ReturnType<typeof setTimeout> | null = null;
  private retry: ReturnType<typeof setTimeout> | null = null;
  private retryDelay = RETRY_MIN_MS;

  constructor(
    private readonly open: (path: string) => EventSource,
    private readonly sink: StreamSink,
  ) {}

  /** The sessions currently on the wire, for routing cached frames. */
  current(): Interest {
    return this.applied;
  }

  setInterest(interest: Interest): void {
    this.wanted = interest;
    if (this.pending) return;
    this.pending = setTimeout(() => {
      this.pending = null;
      if (this.source && sameInterest(this.applied, this.wanted)) return;
      this.connect();
    }, COALESCE_MS);
  }

  /** Reconnect now, e.g. after a login or a change of backend. A new
   *  backend numbers its events from scratch, so its caller clears the
   *  cursor; the same backend keeps it. */
  restart(clearCursor: boolean): void {
    if (clearCursor) this.cursor.clear();
    this.retryDelay = RETRY_MIN_MS;
    this.connect();
  }

  private connect(): void {
    if (this.retry) {
      clearTimeout(this.retry);
      this.retry = null;
    }
    this.source?.close();
    this.source = null;
    this.applied = this.wanted;
    for (const id of [...this.cursor.keys()]) {
      if (!this.applied.sessions.includes(id)) this.cursor.delete(id);
    }
    if (isEmptyInterest(this.applied)) {
      this.sink.status("idle");
      return;
    }
    const source = this.open(streamPath(this.applied, this.cursor));
    this.source = source;
    this.sink.status("connecting");
    source.onopen = () => {
      this.retryDelay = RETRY_MIN_MS;
      this.sink.status("open");
    };
    source.onerror = () => {
      if (this.source !== source) return;
      // A transient failure is the browser's to retry, and it resumes
      // with Last-Event-ID on its own. Only a stream it gave up on (an
      // HTTP error such as an expired login) needs one of ours.
      if (source.readyState !== EventSource.CLOSED) {
        this.sink.status("reconnecting");
        return;
      }
      this.sink.status("offline");
      this.source = null;
      this.retry = setTimeout(() => {
        this.retry = null;
        this.connect();
      }, this.retryDelay);
      this.retryDelay = Math.min(this.retryDelay * 2, RETRY_MAX_MS);
    };
    for (const kind of FRAME_KINDS) {
      source.addEventListener(kind, (message) => {
        if (this.source !== source) return;
        const event = message as MessageEvent<string>;
        if (kind === "agent" && event.lastEventId) this.cursor = parseCursor(event.lastEventId);
        const frame = parseFrame(kind, event.data);
        if (frame) this.sink.frame(frame);
      });
    }
  }
}
