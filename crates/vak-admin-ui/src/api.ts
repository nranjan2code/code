import type {
  DataRulesPreview,
  DataIntegrity,
  ConversationLifecycle,
  ErasurePreview,
  ErasureReceipt,
  KeyRotation,
  KeyStatus,
  SyncStatus,
  TrashedSession,
  RunRecord,
  DataPlan,
  DataStatus,
  DataTick,
  DataTransition,
  DataUsage,
  TelemetryLine,
  TelemetrySpan,
  EffectRecord,
  EffectStatus,
  ChatSurface,
  OnboardingState,
  AllowlistEntry,
  AllowlistRoute,
  ChannelPolicy,
  BestOfNRun,
  BusConfig,
  ServerWebConfig,
  ConfigInfo,
  DiscoveredModelsResponse,
  FinOpsStatus,
  SameModelSuggestions,
  FrozenContract,
  GatewayStatus,
  TrafficSnapshot,
  HealthInfo,
  HookConfig,
  InboxEntry,
  McpListResponse,
  McpServerConfig,
  ConfigLayer,
  ConfigScope,
  PromptBlock,
  PromptEffective,
  PromptLayerResponse,
  IntegrationStatus,
  MemoryItem,
  OpsDiagnostics,
  OpsStatus,
  OperationsSnapshot,
  ApprovalAnswer,
  GatewayApprovalPolicy,
  PendingApproval,
  PendingWorkerQuestion,
  PermissionMode,
  PermissionRules,
  PermissionRulesView,
  ProviderListResponse,
  VoiceProviderListResponse,
  RebuildStats,
  CatalogStatus,
  CatalogNode,
  Lineage,
  SearchHit,
  SecurityEvent,
  SessionCheckpoint,
  SessionDiff,
  Commitment,
  CommitmentList,
  IntentExplain,
  IntentPolicy,
  SessionListItem,
  Verdict,
  SkillItem,
  SkillProposal,
  PluginItem,
  MarketplaceSource,
  MarketplaceEntry,
  AutomationItem,
  TranscriptEntry,
  VoiceConfig,
  WorkReceipt,
  WorkProjection,
  ActiveWorker,
  SandboxRecordsResponse,
  SandboxExecutionsResponse,
} from "./types";

export class AuthRequired extends Error {
  constructor() {
    super("authentication required");
  }
}

/** Append `&agent=`/`?agent=` to a URL that may already carry query
 * params, mirroring how `selectedAgentIdOrUndefined()` is threaded through
 * the memory/proposal endpoints (commit 13a3e6b3) — every config-layer
 * endpoint audited alongside those (hooks, MCP, permissions, prompts,
 * plugins, skills/commands, general config, commitments, checkpoints,
 * finops) resolves against the selected Agent's own isolated workspace the
 * same way, so the same helper threads it through here too. */
function withAgent(url: string, agent?: string): string {
  if (!agent) return url;
  const sep = url.includes("?") ? "&" : "?";
  return `${url}${sep}agent=${encodeURIComponent(agent)}`;
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

/** Preserve HTTP failures for callers that need to refresh after a write. */
async function handleVoid(res: Response): Promise<void> {
  await handle(res);
}

/// Like `handle`, but for an endpoint that answers with raw bytes
/// (`audio/wav`) rather than JSON — `/voice/speak`. A non-JSON error body
/// still parses fine (`body` stays undefined), it just falls back to the
/// bare status code.
async function handleBlob(res: Response): Promise<Blob> {
  if (res.status === 401 || res.status === 403) throw new AuthRequired();
  if (!res.ok) {
    const text = await res.text();
    let detail = `${res.status}`;
    if (text) {
      try {
        const body = JSON.parse(text) as Record<string, unknown>;
        if (body && typeof body === "object" && "error" in body) detail = String(body.error);
      } catch {
        // not JSON — keep the bare status code
      }
    }
    throw new Error(detail);
  }
  return res.blob();
}

export const api = {
  /** ONE login exchange for every browser surface.
   *
   * `/admin/login` was removed when the workspace client arrived — two
   * endpoints setting one cookie is two contracts that must agree forever
   * (AGENTS.md invariant 30). This file kept posting to the dead path, so
   * the console rendered a token form that could never succeed; the
   * middleware 401s an unexempt path before routing, which is why it
   * looked like a wrong token rather than a missing route. */
  async login(token: string): Promise<void> {
    const res = await fetch("/auth/login", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ token }),
    });
    await handle(res);
  },

  async logout(): Promise<void> {
    await handleVoid(await fetch("/auth/logout", { method: "POST" }));
  },

  /** Is there a session, and — on loopback — may we simply be handed one?
   *
   * The probe doubles as the sign-in on this machine: asking someone to
   * go and find a token to reach their own computer is a prompt with no
   * security value. `[server] loopback_auto_login` scopes it. */
  session: () =>
    fetch("/auth/session", { credentials: "same-origin" }).then((r) =>
      handle<{ authenticated: boolean; granted?: string }>(r),
    ),

  health: () => fetch("/health").then((r) => handle<HealthInfo>(r)),
  /** Reloads once a server starting again by itself answers unfenced. */
  reloadWhenBack: (): void => {
    const started = Date.now();
    const check = async () => {
      try {
        const answer = await fetch("/health").then((r) => (r.ok ? r.json() : null));
        if (answer && answer.fenced === false) {
          window.location.reload();
          return;
        }
      } catch {
        // Not back yet.
      }
      if (Date.now() - started < 120_000) window.setTimeout(() => void check(), 1000);
    };
    window.setTimeout(() => void check(), 1500);
  },
  dataStatus: (): Promise<DataStatus> => fetch("/data/status").then((r) => handle(r)),
  /** The install's keep times, and the defaults they were changed from. */
  dataRules: (): Promise<{ label: DataStatus["label"]; defaults: DataStatus["label"]; max_days: number }> =>
    fetch("/data/rules").then((r) => handle(r)),
  /** What new keep times would remove that the current ones keep. */
  previewDataRules: (keepDays: Record<string, number>): Promise<{ preview: DataRulesPreview }> =>
    fetch("/data/rules/preview", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ keep_days: keepDays }),
    }).then((r) => handle(r)),
  /** Saves keep times. A shorter one needs the preview's digest. */
  setDataRules: (keepDays: Record<string, number>, digest?: string): Promise<{ label: DataStatus["label"] }> =>
    fetch("/data/rules", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ keep_days: keepDays, digest }),
    }).then((r) => handle(r)),
  /** Verifies every stored record. Reads the whole data home once. */
  dataIntegrity: (): Promise<DataIntegrity> => fetch("/data/integrity").then((r) => handle(r)),
  dataKeys: (): Promise<KeyStatus> => fetch("/data/keys").then((r) => handle(r)),
  syncStatus: (): Promise<SyncStatus> => fetch("/sync").then((r) => handle(r)),
  /** One of the second copy's actions: setup, now, handover, takeover, forget. */
  syncDo: (verb: "setup" | "now" | "handover" | "takeover" | "forget", body?: unknown): Promise<{ restart_required?: boolean; restarting?: boolean }> =>
    fetch(`/sync/${verb}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body ?? {}) }).then((r) => handle(r)),
  /** The key file for the other machine, sealed under the passphrase. Stored nowhere. */
  syncKeyExport: (passphrase: string): Promise<{ file: string }> =>
    fetch("/sync/key/export", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ passphrase }) }).then((r) => handle(r)),
  syncKeyImport: (file: string, passphrase: string): Promise<{ restart_required: boolean; restarting?: boolean }> =>
    fetch("/sync/key/import", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ file, passphrase }) }).then((r) => handle(r)),
  /** Starts a new key and protects everything again under it. */
  rotateDataKeys: (): Promise<{ rotation: KeyRotation }> =>
    fetch("/data/keys/rotate", { method: "POST" }).then((r) => handle(r)),
  /** Destroys the earlier main keys, once confirmed. */
  retireDataKeys: (): Promise<{ retirement: KeyRotation }> =>
    fetch("/data/keys/retire", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ confirmed: true }) }).then((r) => handle(r)),
  dataUsage: (): Promise<DataUsage> => fetch("/data/usage").then((r) => handle(r)),
  dataPlan: (): Promise<DataPlan> => fetch("/data/lifecycle/plan").then((r) => handle(r)),
  dataTransitions: (): Promise<{ transitions: DataTransition[] }> =>
    fetch("/data/lifecycle/transitions").then((r) => handle(r)),
  /** Runs a retention pass now; it removes only what this install's mode allows. */
  dataTick: (): Promise<DataTick> => fetch("/data/lifecycle/tick", { method: "POST" }).then((r) => handle(r)),

  runs: (status?: string, limit = 200): Promise<{ runs: RunRecord[] }> => {
    const q = new URLSearchParams({ limit: String(limit) });
    if (status) q.set("status", status);
    return fetch(`/runs?${q}`).then((r) => handle(r));
  },
  run: (id: string): Promise<RunRecord> =>
    fetch(`/runs/${encodeURIComponent(id)}`).then((r) => handle(r)),
  runSpans: (id: string): Promise<{ spans: TelemetrySpan[] }> =>
    fetch(`/runs/${encodeURIComponent(id)}/spans`).then((r) => handle(r)),
  telemetryServices: (): Promise<{ services: string[] }> =>
    fetch("/telemetry/services").then((r) => handle(r)),
  telemetryLogs: (filter: { service?: string; level?: string; trace?: string; spans?: boolean } = {}, limit = 200): Promise<{ lines: TelemetryLine[] }> => {
    const q = new URLSearchParams({ limit: String(limit) });
    if (filter.service) q.set("service", filter.service);
    if (filter.level) q.set("level", filter.level);
    if (filter.trace) q.set("trace", filter.trace);
    if (filter.spans) q.set("spans", "true");
    return fetch(`/telemetry/logs?${q}`).then((r) => handle(r));
  },
  effects: (filter: { run?: string; state?: EffectStatus } = {}, limit = 200): Promise<{ effects: EffectRecord[] }> => {
    const q = new URLSearchParams({ limit: String(limit) });
    if (filter.run) q.set("run", filter.run);
    if (filter.state) q.set("state", filter.state);
    return fetch(`/effects?${q}`).then((r) => handle(r));
  },
  effect: (id: string): Promise<EffectRecord> =>
    fetch(`/effects/${encodeURIComponent(id)}`).then((r) => handle(r)),
  /** Send an action again: the same one where the provider drops repeats, else a new one that replaces it. */
  resendEffect: (id: string): Promise<{ ok: boolean; effect: string; new: boolean; status: string }> =>
    fetch(`/effects/${encodeURIComponent(id)}/resend`, { method: "POST" }).then((r) => handle(r)),
  reconcileEffect: (id: string, outcome: "sent" | "not_sent"): Promise<{ ok: boolean; effect: EffectRecord }> =>
    fetch(`/effects/${encodeURIComponent(id)}/reconcile`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ outcome }),
    }).then((r) => handle(r)),

  /** `total` counts every session matching the filter, not just the page
   * `limit` returned — the list itself is capped, the count isn't. */
  sessions: (limit = 100, agent?: string): Promise<{ sessions: SessionListItem[]; total: number; workspace_space_id?: string }> => {
    const q = new URLSearchParams({ limit: String(limit) });
    if (agent && agent !== "all") q.set("agent", agent);
    return fetch(`/admin/api/sessions?${q}`).then((r) => handle(r));
  },

  agents: (): Promise<{ agents: Array<{ id: string; name: string; personality?: string; behaviour?: string }> }> =>
    fetch("/agents").then((r) => handle(r)),

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
  ): Promise<{ session_id: string; entries: TranscriptEntry[]; offset: number; total: number; has_more: boolean; contract?: FrozenContract }> => {
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

  workers: (sessionId: string): Promise<{ workers: ActiveWorker[] }> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/workers`).then((r) => handle(r)),

  steerWorker: (sessionId: string, childId: string, text: string): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/workers/${encodeURIComponent(childId)}/steer`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ text }),
    }).then(handleVoid),

  stopWorker: (sessionId: string, childId: string): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/workers/${encodeURIComponent(childId)}/stop`, {
      method: "POST",
    }).then(handleVoid),

  /** Searches every workspace's conversations, memory and entities. */
  search: (
    q: string,
    filters: { kind?: string; limit?: number } = {},
  ): Promise<{ hits: SearchHit[] }> => {
    const p = new URLSearchParams({ q, all: "true" });
    if (filters.kind) p.set("kind", filters.kind);
    if (filters.limit != null) p.set("limit", String(filters.limit));
    return fetch(`/search?${p}`).then((r) => handle(r));
  },

  security: (limit = 200, kind?: string): Promise<{ events: SecurityEvent[]; total: number }> => {
    const p = new URLSearchParams({ limit: String(limit) });
    if (kind) p.set("kind", kind);
    return fetch(`/admin/api/security?${p}`).then((r) => handle(r));
  },

  rebuild: (): Promise<RebuildStats> =>
    fetch("/catalog/rebuild", { method: "POST" }).then((r) => handle(r)),

  catalogStatus: (): Promise<CatalogStatus> => fetch("/catalog").then((r) => handle(r)),

  sessionNodes: (id: string): Promise<{ nodes: CatalogNode[] }> =>
    fetch(`/catalog/sessions/${encodeURIComponent(id)}/nodes`).then((r) => handle(r)),

  lineage: (id: string): Promise<Lineage> =>
    fetch(`/lineage/${encodeURIComponent(id)}`).then((r) => handle(r)),

  config: (agent?: string) => fetch(withAgent("/admin/api/config", agent)).then((r) => handle<ConfigInfo>(r)),

  gatewayStatus: () =>
    fetch("/admin/api/gateway/status").then((r) => handle<GatewayStatus>(r)),

  traffic: () => fetch("/admin/api/traffic").then((r) => handle<TrafficSnapshot>(r)),

  operations: (): Promise<OperationsSnapshot> =>
    fetch("/ops/center").then((r) => handle<OperationsSnapshot>(r)),

  operationsActions: (): Promise<{ actions: OperationsSnapshot["actions"] }> =>
    fetch("/ops/actions").then((r) => handle(r)),


  opsAction: (service: "gateway" | "bridges", action: "start" | "stop" | "restart" | "install" | "uninstall"): Promise<{ ok: boolean; action?: string; error?: string; receipt_id?: string; receipt_persisted?: boolean; verification?: { status: string; before: string; after: string; detail: string } }> =>
    fetch(`/ops/${service}/${action}`, { method: "POST" }).then((r) => handle(r)),

  // ---- Distributed event bus (vak-bus, docs/design/53) -------------------

  busConfig: (): Promise<BusConfig> =>
    fetch("/config/bus").then((r) => handle<BusConfig>(r)),

  serverWebConfig: (): Promise<ServerWebConfig> =>
    fetch("/config/server").then((r) => handle<ServerWebConfig>(r)),

  putServerWebConfig: (body: { public_url: string; trusted_hosts: string[]; session_ttl_hours: number }): Promise<{ saved: boolean; restart_required: boolean }> =>
    fetch("/config/server", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }).then((r) => handle(r)),

  putBusConfig: (body: {
    nats_url?: string;
    nats_credentials_jwt?: string;
    nats_nkey_seed?: string;
    workspace_secret_env?: string;
  }): Promise<{ configured: boolean }> =>
    fetch("/config/bus", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }).then((r) => handle(r)),

  deleteBusConfig: (): Promise<void> =>
    fetch("/config/bus", { method: "DELETE" }).then(handleVoid),

  // ---- Sandbox (docs/design/2026-sandboxed-execution) --------------------

  sandboxRecords: (): Promise<SandboxRecordsResponse> =>
    fetch("/sandbox/records").then((r) => handle(r)),

  sessionSandboxExecutions: (sessionId: string): Promise<SandboxExecutionsResponse> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/sandbox/executions`).then((r) => handle(r)),

  patchGatewayWorkspace: (workspace: string | null): Promise<{ workspace: string; restart_required: boolean }> =>
    fetch("/admin/api/gateway/workspace", {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ workspace }),
    }).then((r) => handle(r)),

  projects: (): Promise<{ projects: Project[] }> =>
    fetch("/admin/api/projects").then((r) => handle(r)),

  /** What erasing everything kept for a project would remove. */
  projectErasurePreview: (id: string): Promise<{ confirm: string; preview: { digest: string; held: boolean; conversations: number; documents: number; artifacts: number; automations: number; workspace_files: number } }> =>
    fetch(`/data/erasure/projects/${encodeURIComponent(id)}`).then((r) => handle(r)),

  /** Erases it, with the project's name typed. Its folder is not touched. */
  eraseProject: (id: string, digest: string, confirm: string): Promise<{ receipt: ErasureReceipt }> =>
    fetch(`/data/erasure/projects/${encodeURIComponent(id)}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ digest, confirm }),
    }).then((r) => handle(r)),

  /** What erasing everything Vakyartha stored would remove, and the words to type. */
  installErasurePreview: (): Promise<{ confirm: string; preview: { digest: string; held: number; conversations: number; artifacts: number; keys: number } }> =>
    fetch("/data/erasure/install").then((r) => handle(r)),

  /** Erases everything, with the words typed. Vakyartha stops once it has answered. */
  eraseInstall: (digest: string, confirm: string): Promise<{ receipt: ErasureReceipt }> =>
    fetch("/data/erasure/install", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ digest, confirm }),
    }).then((r) => handle(r)),

  patchProject: (id: string, patch: { name?: string; hidden?: boolean; folder?: string }): Promise<{ id: string }> =>
    fetch(`/admin/api/projects/${encodeURIComponent(id)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    }).then((r) => handle(r)),

  patchGatewayBinding: (target: string, route: { provider?: string; model?: string }): Promise<void> =>
    fetch(`/admin/api/gateway/bindings/${encodeURIComponent(target)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(route),
    }).then(handleVoid),

  rotateGatewayBinding: (target: string): Promise<void> =>
    fetch(`/admin/api/gateway/bindings/${encodeURIComponent(target)}/rotate`, { method: "POST" })
      .then(handleVoid),

  deleteGatewayBinding: (target: string): Promise<void> =>
    fetch(`/admin/api/gateway/bindings/${encodeURIComponent(target)}`, { method: "DELETE" })
      .then(handleVoid),

  gatewayAllowlist: (): Promise<{ entries: AllowlistEntry[] }> =>
    fetch("/admin/api/gateway/allowlist").then((r) => handle(r)),

  approveGatewayAllowlist: (
    key: string,
    body: {
      workspace?: string;
      agent_id?: string;
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
      agent_id?: string | null;
      route?: AllowlistRoute | Record<string, never>;
      // Omitted / empty clears the pin back to "inherit the workspace".
      permission_mode?: PermissionMode | "";
      policy?: ChannelPolicy;
      /// Absent = leave the current bot binding alone; `null` = unbind;
      /// a string = bind to that bot id.
      bot_id?: string | null;
      inherit_bot_policy?: boolean;
      /// Absent = leave alone; `null` = clear back to inherit; an object =
      /// pin this chat's own voice.
      voice?: VoiceConfig | null;
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
      .then(handleVoid),

  approvals: (): Promise<{ approvals: PendingApproval[]; total: number }> =>
    fetch("/admin/api/approvals").then((r) => handle(r)),
  questions: (): Promise<{ questions: PendingWorkerQuestion[]; total: number }> =>
    fetch("/admin/api/questions").then((r) => handle(r)),

  /// Answer one gate. `remember` additionally persists the narrowest rule
  /// that covers this call, so the same shape stops asking. Only meaningful
  /// with `approve: true`.
  answer: (
    sessionId: string,
    requestId: string,
    approve: boolean,
    remember = false,
  ): Promise<ApprovalAnswer> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/approvals/${encodeURIComponent(requestId)}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ approve, remember }),
    }).then((r) => handle(r)),

  gatewayApprovals: (): Promise<GatewayApprovalPolicy> =>
    fetch("/gateway/approvals").then((r) => handle(r)),

  setGatewayApprovals: (body: {
    mode: "deny" | "forward";
    approver?: string;
    timeout_secs?: number;
    scope?: ConfigScope;
  }): Promise<GatewayApprovalPolicy> =>
    fetch("/gateway/approvals", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }).then((r) => handle(r)),

  permissionRules: (scope: ConfigScope = "project", agent?: string): Promise<PermissionRulesView> => {
    const s = scope === "project" ? "workspace" : scope;
    return fetch(withAgent(`/config/permissions?scope=${s}`, agent)).then((r) => handle(r));
  },

  setPermissionRules: (
    scope: ConfigScope,
    lists: { allow?: string[]; ask?: string[]; deny?: string[] },
    agent?: string,
  ): Promise<{ scope: ConfigScope; effective: PermissionRules }> => {
    const s = scope === "project" ? "workspace" : scope;
    return fetch("/config/permissions", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ...lists, scope: s, agent }),
    }).then((r) => handle(r));
  },

  setMode: (mode: string, agent?: string): Promise<void> =>
    fetch("/config/mode", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ mode, agent }),
    }).then(handleVoid),

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
   * workspace only; see `ConfigInfo.workspace_space_id`. */
  archiveSession: (sessionId: string, archived: boolean): Promise<{ archived: boolean }> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/archive`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ archived }),
    }).then((r) => handle(r)),

  /** Moves an archived, idle session to the trash, which hides it
   * everywhere, search included. `restoreSession` brings it back for 30
   * days; after that it is erased. */
  trashSession: (sessionId: string): Promise<{ trashed: string }> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}`, { method: "DELETE" }).then((r) => handle(r)),

  trashAllArchived: (): Promise<{ trashed: number }> =>
    fetch("/sessions/archived", { method: "DELETE" }).then((r) => handle(r)),

  /** The trash, each conversation with the day it is erased. */
  trashedSessions: (): Promise<{ sessions: TrashedSession[] }> =>
    fetch("/sessions?trash=true").then((r) => handle(r)),

  restoreSession: (sessionId: string): Promise<{ restored: string }> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/restore`, { method: "POST" }).then((r) => handle(r)),

  /** What erasing would reach, and the title to type. */
  erasurePreview: (sessionId: string): Promise<{ preview: ErasurePreview; confirm: string }> =>
    fetch(`/conversations/${encodeURIComponent(sessionId)}/erasure`).then((r) => handle(r)),

  eraseConversation: (sessionId: string, digest: string, confirm: string): Promise<{ receipt: ErasureReceipt }> =>
    fetch(`/conversations/${encodeURIComponent(sessionId)}/erasure`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ digest, confirm }),
    }).then((r) => handle(r)),

  holdConversation: (sessionId: string, held: boolean): Promise<{ held: boolean }> =>
    fetch(`/conversations/${encodeURIComponent(sessionId)}/hold`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ held }),
    }).then((r) => handle(r)),

  conversationLifecycle: (sessionId: string): Promise<ConversationLifecycle> =>
    fetch(`/conversations/${encodeURIComponent(sessionId)}/lifecycle`).then((r) => handle(r)),

  /** The people other than the owner who wrote in a conversation. */
  conversationGuests: (sessionId: string): Promise<{ guests: { principal: string; name?: string | null }[] }> =>
    fetch(`/conversations/${encodeURIComponent(sessionId)}/guests`).then((r) => handle(r)),

  /** Erases what one guest wrote there, after reading what it reaches. */
  eraseGuest: async (sessionId: string, principal: string): Promise<{ receipt: ErasureReceipt }> => {
    const url = `/conversations/${encodeURIComponent(sessionId)}/guests/${encodeURIComponent(principal)}/erasure`;
    const looked: { preview: { digest: string } } = await fetch(url).then((r) => handle(r));
    return fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ digest: looked.preview.digest }),
    }).then((r) => handle(r));
  },

  /** What erasing one person who wrote to the bots would remove. */
  personErasurePreview: (surface: string, sender: string): Promise<{ preview: { digest: string; held: boolean; conversations: number; shared_conversations: number } }> =>
    fetch("/data/erasure/people/preview", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ surface, sender }),
    }).then((r) => handle(r)),

  /** Erases them. Their id travels in the body, never the address. */
  erasePerson: (surface: string, sender: string, digest: string): Promise<{ receipt: ErasureReceipt }> =>
    fetch("/data/erasure/people", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ surface, sender, digest }),
    }).then((r) => handle(r)),

  /** Lets a person who was erased write to the bots again. */
  allowPerson: (surface: string, sender: string): Promise<{ allowed_again: boolean }> =>
    fetch("/data/erasure/people/allow", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ surface, sender }),
    }).then((r) => handle(r)),

  /** Everything on hold. */
  dataHolds: (): Promise<{ holds: { kind: string; id: string; name?: string | null }[] }> =>
    fetch("/data/holds").then((r) => handle(r)),

  holdArtifact: (artifact: string, on: boolean): Promise<void> =>
    fetch(`/library/${encodeURIComponent(artifact)}/hold`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ on }),
    }).then((r) => handle(r)),

  erasureReceipts: (): Promise<{ receipts: ErasureReceipt[] }> =>
    fetch("/data/erasure/receipts").then((r) => handle(r)),

  runPrompt: (sessionId: string, prompt: string): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/run`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ prompt }),
    }).then(handleVoid),

  steer: (sessionId: string, text: string): Promise<void> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/steering`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ text }),
    }).then(handleVoid),

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

  receipts: async (sessionId: string): Promise<WorkReceipt[]> => {
    const res = await fetch(`/sessions/${encodeURIComponent(sessionId)}/receipts`);
    // A deep-linked historical session may have no live in-memory handle;
    // that is an evidence state, not a client failure. Render an empty
    // receipt set and keep the ledger timeline available.
    if (res.status === 404) return [];
    return handle(res);
  },

  work: (sessionId: string): Promise<WorkProjection | null> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/work`).then((r) => handle(r)),

  workCommand: (sessionId: string, command: Record<string, unknown>): Promise<WorkProjection> =>
    fetch(`/sessions/${encodeURIComponent(sessionId)}/work`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(command),
    }).then((r) => handle(r)),

  checkpoints: (sessionId: string, agent?: string): Promise<{ checkpoints: SessionCheckpoint[] }> =>
    // The session id resolves the owning Agent's own Core server-side while
    // the session is still open; `agent` is the fallback once it's closed
    // (see `resolve_scoped_core`).
    fetch(withAgent(`/sessions/${encodeURIComponent(sessionId)}/checkpoints`, agent)).then((r) => handle(r)),

  restoreCheckpoint: (sessionId: string, seq: number, agent?: string): Promise<void> =>
    fetch(withAgent(`/sessions/${encodeURIComponent(sessionId)}/checkpoints/${seq}/restore`, agent), {
      method: "POST",
    }).then(handleVoid),

  providers: (agent?: string): Promise<ProviderListResponse> =>
    fetch(withAgent("/providers", agent)).then((r) => handle(r)),

  /** `fresh` asks the service now, for a connection test or a refresh a person asked for. */
  models: (providerName: string, agent?: string, fresh = false): Promise<DiscoveredModelsResponse> =>
    fetch(
      withAgent(`/providers/${encodeURIComponent(providerName)}/models${fresh ? "?fresh=true" : ""}`, agent),
    ).then((r) => handle(r)),

  /** Reads every connected service's model list and proposes which ids name `model` (`provider/model`). */
  sameModelSuggestions: (model: string, agent?: string): Promise<SameModelSuggestions> =>
    fetch(withAgent(`/config/route/suggestions?model=${encodeURIComponent(model)}`, agent)).then((r) => handle(r)),

  voiceProviders: (agent?: string): Promise<VoiceProviderListResponse> =>
    fetch(withAgent("/voice/providers", agent)).then((r) => handle(r)),

  setProviderKey: (provider: string, key: string, scope: ConfigScope, agent?: string): Promise<void> =>
    fetch("/config/key", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ provider, key, scope, agent }),
    }).then(handleVoid),

  deleteProviderKey: (provider: string, scope: ConfigScope, agent?: string): Promise<void> =>
    fetch("/config/key", {
      method: "DELETE",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ provider, scope, agent }),
    }).then(handleVoid),

  finops: (agent?: string): Promise<FinOpsStatus> =>
    fetch(withAgent("/finops", agent)).then((r) => handle(r)),

  /** Absent = leave alone, `null` = clear the cap, a number = set it. */
  patchFinops: (
    patch: {
      max_run_usd?: number | null;
      max_day_usd?: number | null;
      price_override?: { model: string; input: number; output: number };
    },
    agent?: string,
  ): Promise<void> =>
    fetch("/finops", {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ...patch, agent }),
    }).then(handleVoid),

  opsStatus: (): Promise<OpsStatus> =>
    fetch("/ops/status").then((r) => handle(r)),

  opsDiagnostics: (): Promise<OpsDiagnostics> =>
    fetch("/ops/diagnostics").then((r) => handle(r)),

  promptLayer: (scope: ConfigScope, agent?: string): Promise<PromptLayerResponse> =>
    fetch(withAgent(`/config/prompts?scope=${scope}`, agent)).then((r) => handle(r)),

  putPromptBlock: (
    scope: ConfigScope,
    block: PromptBlock,
    text: string | null,
    agent?: string,
  ): Promise<void> =>
    fetch("/config/prompts", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ scope, block, text, agent }),
    }).then(handleVoid),

  promptEffective: (agent?: string): Promise<PromptEffective> =>
    fetch(withAgent("/config/prompts/effective", agent)).then((r) => handle(r)),

  promptPreview: (surface: string, role?: string, agent?: string): Promise<PromptEffective> =>
    fetch("/config/prompts/preview", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ surface, ...(role ? { role } : {}), agent }),
    }).then((r) => handle(r)),

  promptRoles: (agent?: string): Promise<{ roles: string[] }> =>
    fetch(withAgent("/config/prompts/roles", agent)).then((r) => handle(r)),

  configLayer: (scope: ConfigScope, agent?: string): Promise<ConfigLayer> =>
    fetch(withAgent(scope === "user" ? "/config/global" : "/config/project", agent)).then((r) => handle(r)),

  patchConfigScope: (scope: ConfigScope, patch: Record<string, unknown>, agent?: string): Promise<void> =>
    fetch(scope === "user" ? "/config/global" : "/config", {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ...patch, agent }),
    }).then(handleVoid),

  mcpServers: (scope: ConfigScope = "project", agent?: string): Promise<McpListResponse> =>
    fetch(withAgent(scope === "user" ? "/config/mcp/global" : "/config/mcp", agent)).then((r) => handle(r)),

  integrationCatalog: (scope: ConfigScope, agent?: string): Promise<{ scope: ConfigScope; integrations: IntegrationStatus[] }> =>
    fetch(withAgent(`/config/integrations?scope=${scope}`, agent)).then((r) => handle(r)),

  enableIntegration: (id: string, scope: ConfigScope, key?: string, agent?: string): Promise<IntegrationStatus> =>
    fetch(`/config/integrations/${encodeURIComponent(id)}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ scope, ...(key?.trim() ? { key: key.trim() } : {}), agent }),
    }).then((r) => handle(r)),

  removeIntegration: (id: string, scope: ConfigScope, agent?: string): Promise<IntegrationStatus> =>
    fetch(withAgent(`/config/integrations/${encodeURIComponent(id)}?scope=${scope}`, agent), { method: "DELETE" }).then((r) => handle(r)),

  putMcpServers: (servers: Record<string, McpServerConfig>, scope: ConfigScope = "project", agent?: string): Promise<void> =>
    fetch(scope === "user" ? "/config/mcp/global" : "/config/mcp", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ servers, agent }),
    }).then(handleVoid),

  hooks: (scope: ConfigScope = "project", agent?: string): Promise<{ hooks: HookConfig[] }> =>
    fetch(withAgent(scope === "user" ? "/config/hooks/global" : "/config/hooks", agent)).then((r) => handle(r)),

  putHooks: (hooks: HookConfig[], scope: ConfigScope = "project", agent?: string): Promise<void> =>
    fetch(scope === "user" ? "/config/hooks/global" : "/config/hooks", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ hooks, agent }),
    }).then(handleVoid),

  skills: (agent?: string): Promise<{ skills: SkillItem[] }> =>
    fetch(withAgent("/skills", agent)).then((r) => handle(r)),

  socialConnectors: (agent?: string): Promise<{ connectors: { id: string; platform: string; summary: string; readiness: "blocked" | "owner_preview" | "identity_link"; reason: string; official_api: string }[] }> =>
    fetch(withAgent("/social/connectors", agent)).then((r) => handle(r)),
  socialXUsage: (agent?: string): Promise<{ month: string; limit: number; used: number }> =>
    fetch(withAgent("/social/x/usage", agent)).then((r) => handle(r)),
  socialCredential: (path: "youtube/key" | "linkedin/client-id" | "linkedin/account" | "reddit/client-id" | "x/token", agent?: string, scope?: "user"): Promise<{ configured?: boolean; inherited?: boolean; connected?: boolean; display_name?: string; expired?: boolean }> => {
    const url = withAgent(`/social/${path}`, agent);
    return fetch(scope ? `${url}${url.includes("?") ? "&" : "?"}scope=user` : url).then((r) => handle(r));
  },
  plugins: (scope?: "workspace" | "user", agent?: string): Promise<{ plugins: PluginItem[] }> =>
    fetch(withAgent(`/plugins${scope ? `?scope=${scope}` : ""}`, agent)).then((r) => handle(r)),
  pluginAudit: (agent?: string): Promise<{ audit: unknown[] }> =>
    fetch(withAgent("/plugins/audit", agent)).then((r) => handle(r)),
  pluginSources: (scope: "workspace" | "user", agent?: string): Promise<{ sources: MarketplaceSource[] }> =>
    fetch(withAgent(`/plugins/sources?scope=${scope}`, agent)).then((r) => handle(r)),
  pluginCatalog: (query = "", scope?: "workspace" | "user", agent?: string): Promise<{ entries: MarketplaceEntry[]; errors: { source_id?: string; error: string }[] }> => {
    const params = new URLSearchParams();
    if (scope) params.set("scope", scope);
    if (query.trim()) params.set("q", query.trim());
    return fetch(withAgent(`/plugins/catalog${params.size ? `?${params}` : ""}`, agent)).then((r) => handle(r));
  },
  pluginRegisterSource: (path: string, label: string, scope: "workspace" | "user", signature?: { key_id: string; public_key: string; signature: string }, agent?: string): Promise<MarketplaceSource> =>
    fetch("/plugins/sources", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ path, label, scope, trust: "manual-review", agent, ...(signature ?? {}) }) }).then((r) => handle(r)),
  pluginKeyAction: (keyId: string, action: "revoke" | "restore", scope: "workspace" | "user", agent?: string): Promise<unknown> =>
    fetch(withAgent(`/plugins/keys/${encodeURIComponent(keyId)}/${action}?scope=${scope}`, agent), { method: "POST" }).then((r) => handle(r)),
  pluginSourceAction: (id: string, action: "enable" | "disable", scope: "workspace" | "user", agent?: string): Promise<MarketplaceSource> =>
    fetch(withAgent(`/plugins/sources/${encodeURIComponent(id)}/${action}?scope=${scope}`, agent), { method: "POST" }).then((r) => handle(r)),
  pluginInstall: (path: string, scope: "workspace" | "user" = "workspace", agent?: string): Promise<PluginItem> =>
    fetch("/plugins/install", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ path, scope, agent }) }).then((r) => handle(r)),
  pluginCatalogInstall: (entry: MarketplaceEntry, scope: "workspace" | "user", agent?: string, update = false): Promise<PluginItem> =>
    fetch("/plugins/catalog/install", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ source_id: entry.source_id, source_scope: entry.source_scope, name: entry.name, scope, agent, update }) }).then((r) => handle(r)),
  pluginUpdate: (path: string, scope: "workspace" | "user" = "workspace", agent?: string): Promise<PluginItem> =>
    fetch("/plugins/update", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ path, scope, agent }) }).then((r) => handle(r)),
  pluginAction: (name: string, action: "enable" | "disable" | "rollback" | "remove", scope: "workspace" | "user", agent?: string): Promise<PluginItem> =>
    fetch(withAgent(`${action === "remove" ? `/plugins/${encodeURIComponent(name)}` : `/plugins/${encodeURIComponent(name)}/${action}`}?scope=${scope}`, agent), { method: action === "remove" ? "DELETE" : "POST" }).then((r) => handle(r)),

  skillProposals: (agent?: string): Promise<{ proposals: SkillProposal[] }> =>
    fetch(agent ? `/skills/proposals?agent=${encodeURIComponent(agent)}` : "/skills/proposals").then((r) => handle(r)),

  promoteProposal: (id: string, agent?: string): Promise<void> => {
    const suffix = agent ? `?agent=${encodeURIComponent(agent)}` : "";
    return fetch(`/skills/proposals/${encodeURIComponent(id)}/promote${suffix}`, { method: "POST" }).then((r) =>
      void handle(r),
    );
  },

  rejectProposal: (id: string, agent?: string): Promise<void> => {
    const suffix = agent ? `?agent=${encodeURIComponent(agent)}` : "";
    return fetch(`/skills/proposals/${encodeURIComponent(id)}/reject${suffix}`, { method: "POST" }).then((r) =>
      void handle(r),
    );
  },

  triggers: (): Promise<{ triggers: AutomationItem[] }> =>
    fetch("/triggers").then((r) => handle(r)),

  createTrigger: (draft: Pick<AutomationItem, "name" | "kind" | "action"> & Partial<AutomationItem>): Promise<AutomationItem> =>
    fetch("/triggers", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(draft),
    }).then((r) => handle(r)),

  /** Replaces an automation's editable fields; extra fields are ignored. */
  putTrigger: (id: string, draft: AutomationItem): Promise<AutomationItem> =>
    fetch(`/triggers/${encodeURIComponent(id)}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(draft),
    }).then((r) => handle(r)),

  runTrigger: (id: string): Promise<void> =>
    fetch(`/triggers/${encodeURIComponent(id)}/run`, { method: "POST" }).then(handleVoid),

  deleteTrigger: (id: string): Promise<void> =>
    fetch(`/triggers/${encodeURIComponent(id)}`, { method: "DELETE" }).then(handleVoid),

  memory: (agent?: string): Promise<{ notes: MemoryItem[] }> =>
    fetch(agent ? `/memory?agent=${encodeURIComponent(agent)}` : "/memory").then((r) => handle(r)),

  addMemory: (scope: "profile" | "project", text: string, tag?: string, agent?: string): Promise<void> =>
    fetch("/memory", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        text,
        scope: scope === "profile" ? "profile" : "workspace",
        tag: tag || undefined,
        agent,
      }),
    }).then(handleVoid),

  forgetMemory: (noteId: string, scope: "workspace" | "profile", agent?: string): Promise<void> => {
    const params = new URLSearchParams({ scope });
    if (agent) params.set("agent", agent);
    return fetch(`/memory/${encodeURIComponent(noteId)}?${params}`, { method: "DELETE" }).then(handleVoid);
  },

  amendMemory: (noteId: string, scope: "workspace" | "profile", text: string, agent?: string): Promise<void> =>
    fetch(`/memory/${encodeURIComponent(noteId)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ text, scope, agent }),
    }).then(handleVoid),


  doctor: (sessionId?: string): Promise<{ report: string; ok: boolean; failures?: number; checks?: Array<{ label: string; ok: boolean; detail: string }>; facts?: string[] }> => {
    const query = sessionId ? `?session=${encodeURIComponent(sessionId)}` : "";
    return fetch(`/doctor${query}`).then((r) => handle(r));
  },

  inbox: (unreadOnly = false, limit = 100): Promise<{ entries: InboxEntry[]; unread_count: number }> => {
    const p = new URLSearchParams({ limit: String(limit) });
    if (unreadOnly) p.set("unread", "true");
    return fetch(`/inbox?${p}`).then((r) => handle(r));
  },

  inboxAck: (id: string): Promise<void> =>
    fetch(`/inbox/${encodeURIComponent(id)}/ack`, { method: "POST" }).then(handleVoid),

  unreadCount: (): Promise<{ count: number }> =>
    fetch("/inbox/unread_count").then((r) => handle(r)),

  /** Store a chat bridge's bot token in the shared user `.env`. Restarts
   * the bridge for surfaces with a managed service unit (Telegram); the
   * others are started by hand and report `restarted: false`. This is
   * the credential a bridge authenticates to Telegram/Discord/Slack
   * with — distinct from a gateway *binding* (which routes an already-
   * connected chat to a workspace/model, and has no field for this). */
  /** The chat transports the server supports, with labels. The only
   * source of channel names in this app. */
  /** The derived setup projection. Never cached: deleting a key or moving
   * a workspace has to make setup incomplete again on the next read. */
  onboarding: (): Promise<OnboardingState> =>
    fetch("/onboarding").then((r) => handle<OnboardingState>(r)),

  /** Install the Shared starter skills and plugins. Idempotent, and it
   * never overwrites a file you have edited. */
  seedCapabilities: (): Promise<{ ok: boolean }> =>
    fetch("/onboarding/seed", { method: "POST" }).then((r) => handle(r)),

  /** What a folder would ask for, read without loading any of it. */
  reviewWorkspace: (path: string): Promise<{
    path: string;
    git: boolean;
    requests_privilege: boolean;
    privileges: string[];
    trusted: boolean;
  }> =>
    fetch("/onboarding/workspace-review", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ path }),
    }).then((r) => handle(r)),

  /** Record an explicit trust decision. Only ever grants: opening safely
   * is the absence of a decision, and is already the default. */
  trustWorkspace: (path: string): Promise<{ trusted: boolean }> =>
    fetch("/onboarding/trust", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ path }),
    }).then((r) => handle(r)),

  /** Start the guided starter session. Read-only regardless of the
   * workspace's configured mode — the cap is applied server-side and is
   * not this caller's to choose. */
  startFirstTask: (): Promise<{ session_id: string; prompt: string; permission_mode: string }> =>
    fetch("/onboarding/first-task", { method: "POST" }).then((r) => handle(r)),

  /** Register services with the platform service manager. The one action
   * that activates; every configuration write is inert until it runs. */
  activateServices: (): Promise<{ ok: boolean; units: { name: string; action: string; error?: string }[] }> =>
    fetch("/ops/services/activate", { method: "POST" }).then((r) => handle(r)),

  chatSurfaces: (): Promise<ChatSurface[]> =>
    fetch("/config")
      .then((r) => handle<{ surfaces?: ChatSurface[] }>(r))
      .then((c) => c.surfaces ?? []),

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
    agent_id?: string,
  ): Promise<{ bot: import("./types").Bot }> =>
    fetch("/gateway/bots", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ id, surface, label, agent_id: agent_id || undefined }),
    }).then((r) => handle(r)),

  updateBot: (
    id: string,
    patch: Partial<
      Pick<
        import("./types").Bot,
        "label" | "agent_id" | "policy" | "permission_mode" | "route" | "workspace" | "voice"
      >
    >,
  ): Promise<{ bot: import("./types").Bot }> =>
    fetch(`/gateway/bots/${encodeURIComponent(id)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    }).then((r) => handle(r)),

  deleteBot: (id: string): Promise<void> =>
    fetch(`/gateway/bots/${encodeURIComponent(id)}`, { method: "DELETE" }).then((r) =>
      handle(r).then(() => undefined),
    ),

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

  library: (): Promise<{ artifacts: import("./types").LibraryArtifact[] }> =>
    fetch("/library").then((r) => handle(r)),

  libraryArtifact: (id: string): Promise<import("./types").LibraryArtifact & { history: import("./types").LibraryVersion[] }> =>
    fetch(`/library/${encodeURIComponent(id)}`).then((r) => handle(r)),

  intakeSources: (): Promise<{ sources: import("./types").IntakeSource[] }> =>
    fetch("/intake/sources").then((r) => handle(r)),

  intakeAddSource: (draft: import("./types").IntakeSourceDraft): Promise<import("./types").IntakeSource> =>
    fetch("/intake/sources", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(draft),
    }).then((r) => handle(r)),

  intakeUpdateSource: (
    id: string,
    patch: Partial<import("./types").IntakeSourceDraft>,
  ): Promise<import("./types").IntakeSource> =>
    fetch(`/intake/sources/${encodeURIComponent(id)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    }).then((r) => handle(r)),

  intakeDeleteSource: (id: string): Promise<void> =>
    fetch(`/intake/sources/${encodeURIComponent(id)}`, { method: "DELETE" }).then((r) => handleVoid(r)),

  intakePoll: (id: string): Promise<void> =>
    fetch(`/intake/sources/${encodeURIComponent(id)}/poll`, { method: "POST" }).then((r) => handleVoid(r)),

  intakeItems: (params?: { source?: string; status?: string; limit?: number }): Promise<{ items: import("./types").CatalogNode[] }> => {
    const qs = new URLSearchParams();
    if (params?.source) qs.set("source", params.source);
    if (params?.status) qs.set("status", params.status);
    if (params?.limit) qs.set("limit", String(params.limit));
    const q = qs.toString();
    return fetch(`/intake/items${q ? `?${q}` : ""}`).then((r) => handle(r));
  },

  intakeItem: (id: string): Promise<import("./types").IntakeItemDetail> =>
    fetch(`/intake/items/${encodeURIComponent(id)}`).then((r) => handle(r)),

  intakeDecide: (id: string, decision: "release" | "quarantine"): Promise<void> =>
    fetch(`/intake/items/${encodeURIComponent(id)}/${decision}`, { method: "POST" }).then((r) => handleVoid(r)),

  intakeAlerts: (): Promise<{ alerts: import("./types").IntakeAlert[] }> =>
    fetch("/intake/alerts").then((r) => handle(r)),

  intakeAddAlert: (draft: import("./types").IntakeAlertDraft): Promise<import("./types").IntakeAlert> =>
    fetch("/intake/alerts", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(draft),
    }).then((r) => handle(r)),

  intakeUpdateAlert: (id: string, patch: Partial<import("./types").IntakeAlertDraft>): Promise<import("./types").IntakeAlert> =>
    fetch(`/intake/alerts/${encodeURIComponent(id)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    }).then((r) => handle(r)),

  intakeDeleteAlert: (id: string): Promise<void> =>
    fetch(`/intake/alerts/${encodeURIComponent(id)}`, { method: "DELETE" }).then((r) => handleVoid(r)),

  /// Synthesize speech via the Live API. Resolution order server-side is
  /// `voice_override` > the resolved chat/bot voice (by `bot_id`/`chat_key`)
  /// > a built-in default — used both for real gateway voice-notes and for
  /// the admin console's Preview button, which always passes an explicit
  /// `voice_override` so it hears the in-progress, unsaved config.
  speak: (body: {
    text: string;
    bot_id?: string;
    chat_key?: string;
    voice_override?: VoiceConfig;
  }): Promise<Blob> =>
    fetch("/voice/speak", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }).then((r) => handleBlob(r)),

  // --- Intent kernel + commitments (docs/design/47-commitment-kernel.md) ---

  /** Resolve a prompt without running it. Free tiers only, so this is safe
   * to call as the user types — it costs nothing and dispatches nothing. */
  explainIntent: (prompt: string, surface?: string): Promise<IntentExplain> => {
    const params = new URLSearchParams({ prompt });
    if (surface) params.set("surface", surface);
    return fetch(`/intent/explain?${params}`).then((r) => handle<IntentExplain>(r));
  },

  intentPolicy: (): Promise<IntentPolicy> =>
    fetch("/intent/policy").then((r) => handle<IntentPolicy>(r)),

  commitments: (all = false, agent?: string): Promise<CommitmentList> =>
    fetch(withAgent(`/commitments?all=${all}`, agent)).then((r) => handle<CommitmentList>(r)),

  commitment: (id: string, agent?: string): Promise<{ commitment: Commitment; events: unknown[] }> =>
    fetch(withAgent(`/commitments/${encodeURIComponent(id)}`, agent)).then((r) =>
      handle<{ commitment: Commitment; events: unknown[] }>(r),
    ),

  /** A refused closure answers 409 with the missing evidence named; `handle`
   * surfaces that message verbatim rather than a bare status. */
  closeCommitment: (id: string, verdict: Verdict, note: string, agent?: string): Promise<void> =>
    fetch(`/commitments/${encodeURIComponent(id)}/close`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ verdict, note, agent }),
    }).then((r) => handle<void>(r)),
};

/** One project (a space) as Configure › Projects lists it. */
export type Project = {
  id: string;
  name: string | null;
  folder: string | null;
  folder_here: boolean;
  trusted: boolean;
  hidden: boolean;
  last_opened: number | null;
};
