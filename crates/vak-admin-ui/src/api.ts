import type {
  AllowlistEntry,
  AllowlistRoute,
  ChannelPolicy,
  BestOfNRun,
  ChatSurfaceStatus,
  ConfigInfo,
  DiscoveredModelsResponse,
  FinOpsStatus,
  GatewayStatus,
  HealthInfo,
  HookConfig,
  InboxEntry,
  McpListResponse,
  McpServerConfig,
  MemoryItem,
  OpsDiagnostics,
  OpsStatus,
  PendingApproval,
  PermissionMode,
  ProviderListResponse,
  RebuildStats,
  SearchHit,
  SecurityEvent,
  SessionCheckpoint,
  SessionDiff,
  SessionListItem,
  SkillItem,
  SkillProposal,
  TaskItem,
  TranscriptEntry,
  WorkReceipt,
  ActiveSubagent,
} from "./types";

export class AuthRequired extends Error {
  constructor() {
    super("authentication required");
  }
}

async function handle<T>(res: Response): Promise<T> {
  if (res.status === 401 || res.status === 403) throw new AuthRequired();
  // A void-returning mutation (mode change, approval, cancel, steering, a
  // 202-accepted run, most gateway/allowlist writes) answers with an empty
  // body and a bare status code — res.json() on "" throws a SyntaxError,
  // which surfaced every one of those as a failure toast even though the
  // effect had already gone through. Parse leniently instead: empty body is
  // `undefined`, not an error.
  const text = await res.text();
  let body: unknown;
  if (text) {
    try {
      body = JSON.parse(text);
    } catch {
      body = undefined;
    }
  }
  if (!res.ok) {
    const detail =
      body && typeof body === "object" && "error" in (body as Record<string, unknown>)
        ? String((body as Record<string, unknown>).error)
        : `${res.status}`;
    throw new Error(detail);
  }
  return body as T;
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

  /** `total` counts every session matching the filter, not just the page
   * `limit` returned — the list itself is capped, the count isn't. */
  sessions: (limit = 100): Promise<{ sessions: SessionListItem[]; total: number }> =>
    fetch(`/admin/api/sessions?limit=${limit}`).then((r) => handle(r)),

  /** Open a persisted session's ledger in this server process so it can
   * accept runs/steering/cancel and report a live diff. Every session the
   * admin lists comes from the store index, not the in-memory handle
   * registry that `/sessions/{id}/run` etc. actually check — the desktop
   * client attaches on every task switch for the same reason (see
   * `vak-desktop/ui/src/api.ts`). Idempotent: re-attaching an already-live
   * handle is a no-op on the server. */
  attach: (sessionId: string): Promise<{ session_id: string }> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/attach`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ session_id: sessionId }),
    }).then((r) => handle(r)),

  transcript: (
    id: string,
    opts: { limit?: number; offset?: number; kind?: string; role?: string; refresh?: boolean } = {},
  ): Promise<{ session_id: string; entries: TranscriptEntry[]; offset: number; total: number; has_more: boolean }> => {
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

  subagents: (sessionId: string): Promise<{ subagents: ActiveSubagent[] }> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/subagents`).then((r) => handle(r)),

  steerSubagent: (sessionId: string, childId: string, text: string): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/subagents/${encodeURIComponent(childId)}/steer`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ text }),
    }).then((r) => void handle(r)),

  stopSubagent: (sessionId: string, childId: string): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/subagents/${encodeURIComponent(childId)}/stop`, {
      method: "POST",
    }).then((r) => void handle(r)),

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

  patchGatewayBinding: (target: string, route: { provider?: string; model?: string }): Promise<void> =>
    fetch(`/admin/api/gateway/bindings/${encodeURIComponent(target)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(route),
    }).then((r) => void handle(r)),

  rotateGatewayBinding: (target: string): Promise<void> =>
    fetch(`/admin/api/gateway/bindings/${encodeURIComponent(target)}/rotate`, { method: "POST" })
      .then((r) => void handle(r)),

  deleteGatewayBinding: (target: string): Promise<void> =>
    fetch(`/admin/api/gateway/bindings/${encodeURIComponent(target)}`, { method: "DELETE" })
      .then((r) => void handle(r)),

  gatewayAllowlist: (): Promise<{ entries: AllowlistEntry[] }> =>
    fetch("/admin/api/gateway/allowlist").then((r) => handle(r)),

  approveGatewayAllowlist: (
    key: string,
    body: {
      workspace?: string;
      route?: AllowlistRoute;
      permission_mode?: PermissionMode;
      policy?: ChannelPolicy;
      /// Bind to a bot identity (multi-bot-per-channel). Omitted/empty = no bot.
      bot_id?: string;
      /// Defaults to true server-side; only send `false` to break inheritance.
      inherit_bot_policy?: boolean;
    } = {},
  ): Promise<AllowlistEntry> =>
    fetch(`/admin/api/gateway/allowlist/${encodeURIComponent(key)}/approve`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }).then((r) => handle(r)),

  patchGatewayAllowlist: (
    key: string,
    body: {
      workspace?: string;
      route?: AllowlistRoute | Record<string, never>;
      // Omitted / empty clears the pin back to "inherit the workspace".
      permission_mode?: PermissionMode | "";
      policy?: ChannelPolicy;
      /// Absent = leave the current bot binding alone; `null` = unbind;
      /// a string = bind to that bot id.
      bot_id?: string | null;
      inherit_bot_policy?: boolean;
    },
  ): Promise<AllowlistEntry> =>
    fetch(`/admin/api/gateway/allowlist/${encodeURIComponent(key)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }).then((r) => handle(r)),

  denyGatewayAllowlist: (key: string): Promise<AllowlistEntry> =>
    fetch(`/admin/api/gateway/allowlist/${encodeURIComponent(key)}/deny`, { method: "POST" })
      .then((r) => handle(r)),

  revokeGatewayAllowlist: (key: string): Promise<void> =>
    fetch(`/admin/api/gateway/allowlist/${encodeURIComponent(key)}`, { method: "DELETE" })
      .then((r) => void handle(r)),

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

  patchConfig: (patch: {
    provider?: string;
    model?: string;
    max_turns?: number;
    theme?: string;
    /** `[memory]` toggles (docs/design/23-memory.md). Omitted = leave alone. */
    memory_search_enabled?: boolean;
    memory_write_enabled?: boolean;
    memory_reflection?: boolean;
    memory_skill_proposals?: boolean;
  }): Promise<void> =>
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

  /** Archive/unarchive is sidebar visibility only — the ledger itself is
   * never touched. Reaches a ledger file under this process's own
   * workspace only; see `ConfigInfo.workspace_project_hash`. */
  archiveSession: (sessionId: string, archived: boolean): Promise<{ archived: boolean }> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/archive`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ archived }),
    }).then((r) => handle(r)),

  /** The server only allows deleting a session that is already archived
   * and not currently running — a deliberate two-step so nothing vanishes
   * from a single click. Soft-delete: marks it in `deleted.json`, the
   * ledger file itself is untouched. */
  deleteSession: (sessionId: string): Promise<{ deleted: string }> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}`, { method: "DELETE" }).then((r) => handle(r)),

  deleteAllArchived: (): Promise<{ deleted: number }> =>
    fetch("/sessions/archived", { method: "DELETE" }).then((r) => handle(r)),

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

  keepBestRun: (sessionId: string): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/keep`, { method: "POST" }).then((r) =>
      void handle(r),
    ),

  discardBestRun: (sessionId: string): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/discard`, { method: "POST" }).then((r) =>
      void handle(r),
    ),

  diff: (sessionId: string): Promise<SessionDiff> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/diff`).then((r) => handle(r)),

  receipts: (sessionId: string): Promise<WorkReceipt[]> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/receipts`).then((r) => handle(r)),

  checkpoints: (sessionId: string): Promise<{ checkpoints: SessionCheckpoint[] }> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/checkpoints`).then((r) => handle(r)),

  restoreCheckpoint: (sessionId: string, seq: number): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/checkpoints/${seq}/restore`, {
      method: "POST",
    }).then((r) => void handle(r)),

  providers: (): Promise<ProviderListResponse> =>
    fetch("/providers").then((r) => handle(r)),

  models: (providerName: string): Promise<DiscoveredModelsResponse> =>
    fetch(`/providers/${encodeURIComponent(providerName)}/models`).then((r) => handle(r)),

  setProviderKey: (provider: string, key: string): Promise<void> =>
    fetch("/config/key", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ provider, key }),
    }).then((r) => void handle(r)),

  deleteProviderKey: (provider: string): Promise<void> =>
    fetch("/config/key", {
      method: "DELETE",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ provider }),
    }).then((r) => void handle(r)),

  finops: (): Promise<FinOpsStatus> =>
    fetch("/finops").then((r) => handle(r)),

  opsStatus: (): Promise<OpsStatus> =>
    fetch("/ops/status").then((r) => handle(r)),

  opsDiagnostics: (): Promise<OpsDiagnostics> =>
    fetch("/ops/diagnostics").then((r) => handle(r)),

  mcpServers: (): Promise<McpListResponse> =>
    fetch("/config/mcp").then((r) => handle(r)),

  tavily: (): Promise<{ enabled: boolean; key_present: boolean; network: boolean; env_var: string }> =>
    fetch("/config/integrations/tavily").then((r) => handle(r)),

  enableTavily: (key: string): Promise<void> =>
    fetch("/config/integrations/tavily", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ key }),
    }).then((r) => void handle(r)),

  disableTavily: (): Promise<void> =>
    fetch("/config/integrations/tavily/disable", { method: "POST" }).then((r) => void handle(r)),

  putMcpServers: (servers: Record<string, McpServerConfig>): Promise<void> =>
    fetch("/config/mcp", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ servers }),
    }).then((r) => void handle(r)),

  hooks: (): Promise<{ hooks: HookConfig[] }> =>
    fetch("/config/hooks").then((r) => handle(r)),

  putHooks: (hooks: HookConfig[]): Promise<void> =>
    fetch("/config/hooks", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ hooks }),
    }).then((r) => void handle(r)),

  skills: (): Promise<{ skills: SkillItem[] }> =>
    fetch("/skills").then((r) => handle(r)),

  skillProposals: (): Promise<{ proposals: SkillProposal[] }> =>
    fetch("/skills/proposals").then((r) => handle(r)),

  promoteProposal: (id: string): Promise<void> =>
    fetch(`/skills/proposals/${encodeURIComponent(id)}/promote`, { method: "POST" }).then((r) =>
      void handle(r),
    ),

  rejectProposal: (id: string): Promise<void> =>
    fetch(`/skills/proposals/${encodeURIComponent(id)}/reject`, { method: "POST" }).then((r) =>
      void handle(r),
    ),

  tasks: (): Promise<{ tasks: TaskItem[] }> =>
    fetch("/tasks").then((r) => handle(r)),

  createTask: (body: { name: string; prompt?: string; script?: string; interval_secs?: number; schedule?: string; model_pin?: string; deliver_to?: string }): Promise<TaskItem> =>
    fetch("/tasks", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }).then((r) => handle(r)),

  runTaskNow: (id: string): Promise<void> =>
    fetch(`/tasks/${encodeURIComponent(id)}/run-now`, { method: "POST" }).then((r) => void handle(r)),

  patchTask: (id: string, patch: Partial<TaskItem>): Promise<void> =>
    fetch(`/tasks/${encodeURIComponent(id)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    }).then((r) => void handle(r)),

  deleteTask: (id: string): Promise<void> =>
    fetch(`/tasks/${encodeURIComponent(id)}`, { method: "DELETE" }).then((r) => void handle(r)),

  memory: (): Promise<{ notes: MemoryItem[] }> =>
    fetch("/memory").then((r) => handle(r)),

  addMemory: (scope: "profile" | "project", text: string, tag?: string): Promise<void> =>
    fetch("/memory", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        text,
        scope: scope === "profile" ? "profile" : "workspace",
        tag: tag || undefined,
      }),
    }).then((r) => void handle(r)),

  forgetMemory: (noteId: string, scope: "workspace" | "profile"): Promise<void> =>
    fetch(`/memory/${encodeURIComponent(noteId)}?scope=${scope}`, { method: "DELETE" }).then((r) => void handle(r)),

  amendMemory: (noteId: string, scope: "workspace" | "profile", text: string): Promise<void> =>
    fetch(`/memory/${encodeURIComponent(noteId)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ text, scope }),
    }).then((r) => void handle(r)),

  cleanupMemory: (): Promise<{ removed_locks: number; removed_temps: number; removed_empty_dirs: number }> =>
    fetch("/memory/cleanup", { method: "POST" }).then((r) => handle(r)),

  doctor: (): Promise<{ report: string; ok: boolean }> =>
    fetch("/doctor").then((r) => handle(r)),

  inbox: (unreadOnly = false, limit = 100): Promise<{ entries: InboxEntry[]; unread_count: number }> => {
    const p = new URLSearchParams({ limit: String(limit) });
    if (unreadOnly) p.set("unread", "true");
    return fetch(`/inbox?${p}`).then((r) => handle(r));
  },

  inboxAck: (id: string): Promise<void> =>
    fetch(`/inbox/${encodeURIComponent(id)}/ack`, { method: "POST" }).then((r) => void handle(r)),

  unreadCount: (): Promise<{ count: number }> =>
    fetch("/inbox/unread_count").then((r) => handle(r)),

  /** Which chat bridges have a bot token set. `/config` is the only route
   * that reports this — the admin projection (`/admin/api/config`) omits
   * it — and it reports presence only; the token itself never comes back
   * over the wire. */
  chatSurfaces: (): Promise<ChatSurfaceStatus[]> =>
    fetch("/config")
      .then((r) => handle<{ chat_surfaces?: ChatSurfaceStatus[] }>(r))
      .then((c) => c.chat_surfaces ?? []),

  /** Store a chat bridge's bot token in the shared user `.env`. Restarts
   * the bridge for surfaces with a managed service unit (Telegram); the
   * others are started by hand and report `restarted: false`. This is
   * the credential a bridge authenticates to Telegram/Discord/Slack
   * with — distinct from a gateway *binding* (which routes an already-
   * connected chat to a workspace/model, and has no field for this). */
  putBotToken: (
    surface: string,
    token: string,
  ): Promise<{ surface: string; env_var: string; configured: boolean; restarted: boolean }> =>
    fetch(`/config/bot-token/${encodeURIComponent(surface)}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ token }),
    }).then((r) => handle(r)),

  removeBotToken: (
    surface: string,
  ): Promise<{ surface: string; env_var: string; configured: boolean; restarted: boolean }> =>
    fetch(`/config/bot-token/${encodeURIComponent(surface)}`, { method: "DELETE" }).then((r) => handle(r)),

  /** Multi-bot-per-channel (docs/design/34, multi-bot): a `Bot` is an
   * independent credential/policy identity, distinct from the single
   * surface-keyed slot `putBotToken` manages. Several bots can share a
   * surface; a chat picks which one it's bound to via its `bot_id`. */
  listBots: (): Promise<{ bots: import("./types").Bot[] }> =>
    fetch("/gateway/bots").then((r) => handle(r)),

  createBot: (
    id: string,
    surface: string,
    label: string,
  ): Promise<{ bot: import("./types").Bot }> =>
    fetch("/gateway/bots", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ id, surface, label }),
    }).then((r) => handle(r)),

  updateBot: (
    id: string,
    patch: Partial<
      Pick<import("./types").Bot, "label" | "policy" | "permission_mode" | "route" | "workspace">
    >,
  ): Promise<{ bot: import("./types").Bot }> =>
    fetch(`/gateway/bots/${encodeURIComponent(id)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    }).then((r) => handle(r)),

  deleteBot: (id: string): Promise<void> =>
    fetch(`/gateway/bots/${encodeURIComponent(id)}`, { method: "DELETE" }).then(() => undefined),

  putBotIdToken: (
    id: string,
    token: string,
  ): Promise<{ id: string; env_var: string; configured: boolean }> =>
    fetch(`/gateway/bots/${encodeURIComponent(id)}/token`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ token }),
    }).then((r) => handle(r)),

  removeBotIdToken: (
    id: string,
  ): Promise<{ id: string; env_var: string; configured: boolean }> =>
    fetch(`/gateway/bots/${encodeURIComponent(id)}/token`, { method: "DELETE" }).then((r) =>
      handle(r),
    ),

  feedSourceTypes: (): Promise<{ source_types: import("./types").FeedSourceType[] }> =>
    fetch("/feeds/sources").then((r) => handle(r)),

  feedStats: (): Promise<import("./types").FeedStats> =>
    fetch("/feeds/stats").then((r) => handle(r)),

  feedSearch: (params: {
    q: string;
    tags?: string;
    since?: string;
    limit?: number;
    source?: string;
  }): Promise<import("./types").FeedSearchResponse> => {
    const qs = new URLSearchParams({ q: params.q });
    if (params.tags) qs.set("tags", params.tags);
    if (params.since) qs.set("since", params.since);
    if (params.limit) qs.set("limit", String(params.limit));
    if (params.source) qs.set("source", params.source);
    return fetch(`/feeds/search?${qs}`).then((r) => handle(r));
  },

  feedItems: (params?: {
    limit?: number;
    source?: string;
  }): Promise<{ items: import("./types").FeedItem[] }> => {
    const qs = new URLSearchParams();
    if (params?.limit) qs.set("limit", String(params.limit));
    if (params?.source) qs.set("source", params.source);
    const q = qs.toString();
    return fetch(`/feeds/items${q ? `?${q}` : ""}`).then((r) => handle(r));
  },

  feedItem: (id: number): Promise<import("./types").FeedItem> =>
    fetch(`/feeds/items/${id}`).then((r) => handle(r)),

  feedAlerts: (): Promise<{ alerts: import("./types").FeedAlertRule[] }> =>
    fetch("/feeds/alerts").then((r) => handle(r)),

  feedIngest: (): Promise<{ sources_ingested: number; new_items: number; alerts_fired: number; errors: number }> =>
    fetch("/feeds/ingest", { method: "POST" }).then((r) => handle(r)),

  feedAddSource: (source: import("./types").FeedSource): Promise<{ status: string }> =>
    fetch("/feeds/sources", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(source),
    }).then((r) => handle(r)),

  feedDeleteSource: (name: string): Promise<{ status: string }> =>
    fetch(`/feeds/sources/${encodeURIComponent(name)}`, {
      method: "DELETE",
    }).then((r) => handle(r)),
};
