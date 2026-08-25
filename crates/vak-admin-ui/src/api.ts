import type {
  BestOfNRun,
  ConfigInfo,
  GatewayStatus,
  HealthInfo,
  InboxEntry,
  PendingApproval,
  RebuildStats,
  SearchHit,
  SecurityEvent,
  SessionListItem,
  TranscriptEntry,
} from "./types";

export class AuthRequired extends Error {
  constructor() {
    super("authentication required");
  }
}

async function handle<T>(res: Response): Promise<T> {
  if (res.status === 401 || res.status === 403) throw new AuthRequired();
  if (!res.ok) {
    let detail = `${res.status}`;
    try {
      const body = await res.json();
      if (body?.error) detail = body.error;
    } catch {
      // status code is enough
    }
    throw new Error(detail);
  }
  return (await res.json()) as T;
}

export const api = {
  async login(token: string): Promise<void> {
    const res = await fetch("/admin/login", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ token }),
    });
    await handle(res);
  },

  async logout(): Promise<void> {
    await fetch("/admin/logout", { method: "POST" });
  },

  health: () => fetch("/health").then((r) => handle<HealthInfo>(r)),

  sessions: (limit = 100): Promise<{ sessions: SessionListItem[] }> =>
    fetch(`/admin/api/sessions?limit=${limit}`).then((r) => handle(r)),

  transcript: (
    id: string,
    opts: { limit?: number; offset?: number; kind?: string; role?: string; refresh?: boolean } = {},
  ): Promise<{ session_id: string; entries: TranscriptEntry[]; offset: number; has_more: boolean }> => {
    const q = new URLSearchParams();
    if (opts.limit != null) q.set("limit", String(opts.limit));
    if (opts.offset != null) q.set("offset", String(opts.offset));
    if (opts.kind) q.set("kind", opts.kind);
    if (opts.role) q.set("role", opts.role);
    if (opts.refresh) q.set("refresh", "true");
    return fetch(`/admin/api/sessions/${encodeURIComponent(id)}/transcript?${q}`).then((r) =>
      handle(r),
    );
  },

  search: (
    q: string,
    filters: { role?: string; kind?: string; limit?: number } = {},
  ): Promise<{ hits: SearchHit[]; total: number }> => {
    const p = new URLSearchParams({ q });
    if (filters.role) p.set("role", filters.role);
    if (filters.kind) p.set("kind", filters.kind);
    if (filters.limit != null) p.set("limit", String(filters.limit));
    return fetch(`/admin/api/search?${p}`).then((r) => handle(r));
  },

  security: (limit = 200, kind?: string): Promise<{ events: SecurityEvent[]; total: number }> => {
    const p = new URLSearchParams({ limit: String(limit) });
    if (kind) p.set("kind", kind);
    return fetch(`/admin/api/security?${p}`).then((r) => handle(r));
  },

  rebuild: (): Promise<RebuildStats> =>
    fetch("/admin/api/store/rebuild", { method: "POST" }).then((r) => handle(r)),

  config: () => fetch("/admin/api/config").then((r) => handle<ConfigInfo>(r)),

  gatewayStatus: () =>
    fetch("/admin/api/gateway/status").then((r) => handle<GatewayStatus>(r)),

  approvals: (): Promise<{ approvals: PendingApproval[]; total: number }> =>
    fetch("/admin/api/approvals").then((r) => handle(r)),

  answer: (sessionId: string, requestId: string, approve: boolean): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/approvals/${encodeURIComponent(requestId)}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ approve }),
    }).then((r) => void handle(r)),

  setMode: (mode: string): Promise<void> =>
    fetch("/config/mode", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ mode }),
    }).then((r) => void handle(r)),

  patchConfig: (patch: { provider?: string; model?: string }): Promise<void> =>
    fetch("/config", {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    }).then((r) => void handle(r)),

  cancelRun: (sessionId: string): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/cancel`, { method: "POST" }).then((r) =>
      void handle(r),
    ),

  createSession: (): Promise<{ session_id: string }> =>
    fetch("/sessions", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    }).then((r) => handle(r)),

  runPrompt: (sessionId: string, prompt: string): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/run`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ prompt }),
    }).then((r) => void handle(r)),

  steer: (sessionId: string, text: string): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/steering`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ text }),
    }).then((r) => void handle(r)),

  startBestofn: (sessionId: string, prompt: string, n: number): Promise<{ runs: { session_id: string; branch: string }[] }> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/bestofn`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ prompt, n }),
    }).then((r) => handle(r)),

  bestofn: (): Promise<{ runs: BestOfNRun[]; total: number }> =>
    fetch("/admin/api/bestofn").then((r) => handle(r)),

  inbox: (unreadOnly = false, limit = 100): Promise<{ entries: InboxEntry[]; unread_count: number }> => {
    const p = new URLSearchParams({ limit: String(limit) });
    if (unreadOnly) p.set("unread", "true");
    return fetch(`/inbox?${p}`).then((r) => handle(r));
  },

  inboxAck: (id: string): Promise<void> =>
    fetch(`/inbox/${encodeURIComponent(id)}/ack`, { method: "POST" }).then((r) => void handle(r)),

  unreadCount: (): Promise<{ count: number }> =>
    fetch("/inbox/unread_count").then((r) => handle(r)),
};
