import type {
  BackupManifest,
  BestOfNRun,
  ConfigInfo,
  DigestReport,
  DoctorReport,
  DiscoveredSkill,
  FinopsStatus,
  GatewayStatus,
  HealthInfo,
  HookConfig,
  InboxEntry,
  ImportReportShape,
  McpServerDef,
  NoteBlock,
  OpsDiagnostics,
  OpsStatusShape,
  PendingApproval,
  ProvidersResponse,
  RebuildStats,
  SearchHit,
  SecurityEvent,
  SessionListItem,
  SkillProposal,
  TaskDef,
  TaskDraft,
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

  patchConfig: (patch: { provider?: string; model?: string; max_turns?: number; permission_mode?: string; theme?: string }): Promise<void> =>
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

  listProviders: (): Promise<ProvidersResponse> =>
    fetch("/providers").then((r) => handle(r)),

  discoverModels: (provider: string): Promise<{ provider: string; models: string[] }> =>
    fetch(`/providers/${encodeURIComponent(provider)}/models`).then((r) => handle(r)),

  putProviderKey: (
    provider: string,
    key: string,
  ): Promise<{ provider: string; env_var: string; configured: boolean }> =>
    fetch("/config/key", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ provider, key }),
    }).then((r) => handle(r)),

  removeProviderKey: (
    provider: string,
  ): Promise<{ provider: string; env_var: string; configured: boolean; shadowed_by_env: boolean }> =>
    fetch("/config/key", {
      method: "DELETE",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ provider }),
    }).then((r) => handle(r)),

  getMcpServers: (): Promise<{ servers: Record<string, McpServerDef> }> =>
    fetch("/config/mcp").then((r) => handle(r)),

  putMcpServers: (servers: Record<string, McpServerDef>): Promise<{ saved: boolean; count: number }> =>
    fetch("/config/mcp", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ servers }),
    }).then((r) => handle(r)),

  getHooks: (): Promise<{ hooks: HookConfig[] }> =>
    fetch("/config/hooks").then((r) => handle(r)),

  putHooks: (hooks: HookConfig[]): Promise<{ saved: boolean; count: number }> =>
    fetch("/config/hooks", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ hooks }),
    }).then((r) => handle(r)),

  listSkills: (): Promise<{ skills: DiscoveredSkill[] }> =>
    fetch("/skills").then((r) => handle(r)),

  listProposals: (): Promise<{ proposals: SkillProposal[] }> =>
    fetch("/skills/proposals").then((r) => handle(r)),

  promoteProposal: (id: string): Promise<{ promoted: string }> =>
    fetch(`/skills/proposals/${encodeURIComponent(id)}/promote`, { method: "POST" }).then((r) =>
      handle(r),
    ),

  rejectProposal: (id: string): Promise<{ rejected: string }> =>
    fetch(`/skills/proposals/${encodeURIComponent(id)}/reject`, { method: "POST" }).then((r) =>
      handle(r),
    ),

  listMemory: (): Promise<{ notes: NoteBlock[] }> =>
    fetch("/memory").then((r) => handle(r)),

  appendMemory: (draft: {
    kind: string;
    tag: string;
    text: string;
    scope?: string;
    session_id?: string;
  }): Promise<NoteBlock> =>
    fetch("/memory", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(draft),
    }).then((r) => handle(r)),

  amendMemory: (id: string, scope: string, text: string): Promise<{ amended: string }> =>
    fetch(`/memory/${encodeURIComponent(id)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ text, scope }),
    }).then((r) => handle(r)),

  forgetMemory: (id: string, scope: string): Promise<{ forgotten: string; bytes: number }> =>
    fetch(`/memory/${encodeURIComponent(id)}?scope=${encodeURIComponent(scope)}`, {
      method: "DELETE",
    }).then((r) => handle(r)),

  listTasks: (): Promise<{ tasks: TaskDef[] }> =>
    fetch("/tasks").then((r) => handle(r)),

  createTask: (draft: TaskDraft): Promise<unknown> =>
    fetch("/tasks", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(draft),
    }).then((r) => handle(r)),

  patchTask: (id: string, patch: Partial<TaskDef>): Promise<TaskDef> =>
    fetch(`/tasks/${encodeURIComponent(id)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(patch),
    }).then((r) => handle(r)),

  deleteTask: (id: string): Promise<unknown> =>
    fetch(`/tasks/${encodeURIComponent(id)}`, { method: "DELETE" }).then((r) => handle(r)),

  runTaskNow: (id: string): Promise<unknown> =>
    fetch(`/tasks/${encodeURIComponent(id)}/run-now`, { method: "POST" }).then((r) => handle(r)),

  opsStatus: (): Promise<OpsStatusShape> =>
    fetch("/ops/status").then((r) => handle(r)),

  opsAction: (service: string, action: string): Promise<{ ok: boolean; error?: string }> =>
    fetch(`/ops/${encodeURIComponent(service)}/${encodeURIComponent(action)}`, {
      method: "POST",
    }).then((r) => handle(r)),

  opsDiagnostics: (): Promise<OpsDiagnostics> =>
    fetch("/ops/diagnostics").then((r) => handle(r)),

  finopsStatus: (): Promise<FinopsStatus> =>
    fetch("/finops").then((r) => handle(r)),

  digest: (days: number): Promise<DigestReport> =>
    fetch(`/digest?days=${days}`).then((r) => handle(r)),

  doctor: (): Promise<DoctorReport> =>
    fetch("/doctor").then((r) => handle(r)),

  backupExport: (
    destDir: string,
    includeSecrets: boolean,
  ): Promise<{ manifest: BackupManifest; included_secrets: boolean }> =>
    fetch("/backup/export", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ dest_dir: destDir, include_secrets: includeSecrets }),
    }).then((r) => handle(r)),

  backupImport: (
    srcDir: string,
    conflict: "skip" | "rename",
  ): Promise<ImportReportShape> =>
    fetch("/backup/import", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ src_dir: srcDir, conflict }),
    }).then((r) => handle(r)),
};
