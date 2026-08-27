import { invoke } from "@tauri-apps/api/core";
import type {
  AgentEvent,
  BackendInfo,
  Health,
  Message,
  SessionSummary,
  ConfigSnapshot,
} from "./types";

let base = "";
let token = "";
let projectId = "";
let projectRoot = "";
let configRevision = 0;
let rawConfig: Record<string, any> | null = null;
const activeRuns = new Map<string, string>();

export function backendUrl(): string {
  return base;
}

export async function initBackend(): Promise<BackendInfo> {
  return invoke<BackendInfo>("backend_info");
}

export function adoptBackend(info: BackendInfo): void {
  if (info.ready && info.base_url && info.token) {
    base = info.base_url;
    token = info.token;
    projectId = info.project_id ?? "";
    projectRoot = info.cwd ?? "";
  } else {
    base = "";
    token = "";
    projectId = "";
    projectRoot = "";
  }
}

export function isBackendReady(): boolean {
  return !!base;
}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${base}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
      ...(init?.headers ?? {}),
    },
  });
  const text = await res.text();
  let parsed: unknown = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = text;
  }
  if (!res.ok) {
    const msg =
      (parsed as { error?: string })?.error ?? `${res.status} ${res.statusText}`;
    throw new Error(msg);
  }
  return parsed as T;
}

// ---- ops (background services) ------------------------------------------------

export interface OpsStatusShape {
  gateway: { state: string };
  gateway_healthy: boolean;
}

export function opsStatus(): Promise<OpsStatusShape> {
  return health().then((h) => ({ gateway: { state: h.status === "ok" ? "running" : "unreachable" }, gateway_healthy: h.status === "ok" }));
}

export function opsAction(
  service: "gateway",
  action: "start" | "stop" | "restart" | "install" | "uninstall",
): Promise<{ ok: boolean; error?: string }> {
  return invoke<{ ok: boolean; error?: string }>("service_action", { service, action })
    .then(() => ({ ok: true }))
    .catch((error) => ({ ok: false, error: String(error) }));
}

export interface OpsDiagnostics {
  health: { status: string; provider: string; model: string; sandbox: string; permission_mode: string; warnings: string[] };
  services: OpsStatusShape;
  gateway: { enabled: boolean; bindings: { target: string; session_id: string }[]; approvals: { mode: string; approver?: string | null; pending: number } };
  flows: { name: string; runs: number }[];
}

export function opsDiagnostics(): Promise<OpsDiagnostics> {
  return Promise.all([health(), opsStatus(), listTasks()]).then(([h, services, tasks]) => ({ health: h, services, gateway: { enabled: services.gateway_healthy, bindings: [], approvals: { mode: "runtime", pending: 0 } }, flows: tasks.tasks.map((task) => ({ name: task.name, runs: 0 })) }));
}

// ---- learning (memory notes + skill proposals) -------------------------------

export interface NoteBlock {
  id: string;
  ts: string;
  kind: string;
  tag: string;
  session_id: string;
  text: string;
  /** "workspace" (per-project MEMORY.md) | "profile" (global USER.md). */
  scope?: "workspace" | "profile";
}

export type MemoryScope = NonNullable<NoteBlock["scope"]>;

export function forgetMemory(id: string, scope: MemoryScope): Promise<{ forgotten: string; bytes: number }> {
  void scope;
  return req<{ forgotten: boolean }>(`/memory/${encodeURIComponent(id)}`, { method: "DELETE", body: "{}" })
    .then((r) => ({ forgotten: id, bytes: r.forgotten ? 0 : 0 }));
}

export function amendMemory(id: string, scope: MemoryScope, text: string): Promise<{ amended: string }> {
  void scope;
  return req(`/memory/${encodeURIComponent(id)}`, { method: "PATCH", body: JSON.stringify({ text }) })
    .then(() => ({ amended: id }));
}

// ---- recall search -----------------------------------------------------------

export interface SearchHit {
  session_id: string;
  entry_id: string;
  ts: string;
  role: string;
  score: number;
  snippet: string;
  /** Set only in global mode: hash of the project the hit came from. */
  project_hash?: string;
}

export function searchSessions(
  q: string,
  limit: number,
  all: boolean,
): Promise<{ all: boolean; hits: SearchHit[] }> {
  void all;
  const needle = q.toLocaleLowerCase();
  return listSessions().then(async ({ sessions }) => {
    const hits: SearchHit[] = [];
    for (const session of sessions) {
      const transcript = await transcript(session.session_id);
      transcript.messages.forEach((message, index) => {
        const text = message.content.filter((block) => block.type === "text").map((block) => block.text).join("\n");
        if (text.toLocaleLowerCase().includes(needle)) hits.push({ session_id: session.session_id, entry_id: String(index), ts: session.updated_at, role: message.role, score: 1, snippet: text.slice(0, 240) });
      });
    }
    return { all: true, hits: hits.slice(0, limit) };
  });
}

export interface SkillProposal {
  id: string;
  name: string;
  description: string;
}

export interface DiscoveredSkill {
  name: string;
  description: string;
  source?: string;
  scope?: string;
}

export function listMemory(): Promise<{ notes: NoteBlock[] }> {
  return req<{ items: Array<{ id: string; created_at: string; kind: string; tag: string; text: string; project_id?: string | null; scope: string }> }>(`/memory?project_id=${encodeURIComponent(projectId)}&scope=workspace`).then((r) => ({ notes: r.items.map((n) => ({ id: n.id, ts: n.created_at, kind: n.kind, tag: n.tag, session_id: "", text: n.text, scope: n.scope as MemoryScope })) }));
}

export function listProposals(): Promise<{ proposals: SkillProposal[] }> {
  return listSkills().then((r) => ({ proposals: r.skills.filter((skill) => skill.scope === "proposed").map((skill) => ({ id: skill.name, name: skill.name, description: skill.description })) }));
}

export function promoteProposal(id: string): Promise<{ promoted: string }> {
  return req(`/skills/${encodeURIComponent(id)}/promote`, { method: "POST", body: "{}" }).then(() => ({ promoted: id }));
}

export function rejectProposal(id: string): Promise<{ rejected: string }> {
  return req(`/skills/${encodeURIComponent(id)}/reject`, { method: "POST", body: "{}" }).then(() => ({ rejected: id }));
}

// ---- sessions ---------------------------------------------------------------

export function listSessions(): Promise<{ sessions: SessionSummary[] }> {
  return req<{ items: Array<{ id: string; project_id: string; created_at: string; status: string }> }>("/sessions").then((r) => ({
    sessions: r.items.map((s) => ({ session_id: s.id, cwd: projectRoot, created_at: s.created_at, updated_at: s.created_at, entries: 0, running: s.status !== "idle" })),
  }));
}

export async function createSession(): Promise<{ session_id: string }> {
  if (!projectId) return Promise.reject(new Error("no project selected"));
  const configured = await req<{ config: { provider: { name?: string }; model: { name?: string }; permission: { mode: string }; sandbox: { backend: string } } }>(`/config?project_id=${encodeURIComponent(projectId)}`);
  return req<{ session_id: string }>("/sessions", { method: "POST", body: JSON.stringify({ project_id: projectId, contract: { provider: configured.config.provider.name ?? "", model: configured.config.model.name ?? "", route_ladder: [], system_prompt: "", permission_mode: configured.config.permission.mode === "workspace_write" ? "WorkspaceWrite" : configured.config.permission.mode === "full_access" ? "FullAccess" : "ReadOnly", sandbox: configured.config.sandbox.backend === "landlock" ? "Landlock" : configured.config.sandbox.backend === "docker" ? "Docker" : configured.config.sandbox.backend === "none" ? "None" : "Seatbelt", tool_catalogue_revision: "desktop", context_limit: 128000 } }) });
}

export function attachSession(id: string): Promise<{ session_id: string }> {
  return req<{ messages: Message[] }>(`/sessions/${encodeURIComponent(id)}/transcript`).then(() => ({ session_id: id }));
}

export async function transcript(id: string): Promise<{
  count: number;
  usage: Record<string, number>;
  messages: Message[];
}> {
  const body = await req<{ messages: Message[] }>(`/sessions/${encodeURIComponent(id)}/transcript`);
  return { count: body.messages.length, usage: {}, messages: body.messages };
}

/** Markdown export (shared renderer with the TUI); text, not JSON. */
export async function transcriptMarkdown(id: string): Promise<string> {
  const transcript = await transcript(id);
  return transcript.messages.map((message) => {
    const text = message.content.filter((block) => block.type === "text").map((block) => block.text).join("\n");
    return `## ${message.role}\n\n${text}`;
  }).join("\n\n");
}

export function runPrompt(id: string, prompt: string): Promise<void> {
  return req<{ run: { run_id: string } }>("/runs", {
    method: "POST",
    body: JSON.stringify({ session_id: id, project_id: projectId, input: prompt }),
  }).then((result) => { activeRuns.set(id, result.run.run_id); });
}

export function steer(id: string, text: string): Promise<void> {
  return runPrompt(id, text).then(() => undefined);
}

export function cancelRun(id: string): Promise<void> {
  const run = activeRuns.get(id);
  if (!run) return Promise.resolve();
  return req(`/runs/${encodeURIComponent(run)}/cancel`, { method: "POST", body: JSON.stringify({ reason: "user" }) }).then(() => { activeRuns.delete(id); });
}

export function runSide(id: string, question: string): Promise<void> {
  return runPrompt(id, question);
}

export function cancelSide(id: string): Promise<void> {
  return cancelRun(id);
}

export function answerApproval(
  id: string,
  requestId: string,
  approve: boolean,
): Promise<unknown> {
  void id;
  return req(`/approvals/${encodeURIComponent(requestId)}/resolve`, {
    method: "POST",
    body: JSON.stringify({ allow: approve, response: { source: "desktop" } }),
  });
}

export function health(): Promise<Health> {
  // /health is intentionally open; still send the header for consistency.
  return req<{ status: string }>("/health").then((h) => ({ status: h.status, provider: "", model: "", permission_mode: "ReadOnly", sandbox: "seatbelt", context_window: 0, cwd: "", warnings: [] }));
}

export function setPermissionMode(mode: string): Promise<void> {
  const wire = mode === "read-only" ? "ReadOnly" : mode === "workspace-write" ? "WorkspaceWrite" : "FullAccess";
  return req<{ revision: number }>(`/config?project_id=${encodeURIComponent(projectId)}`).then((snapshot) => req(`/config/permission-mode`, { method: "POST", body: JSON.stringify({ project_id: projectId, revision: snapshot.revision, mode: wire }) })).then(() => undefined);
}

export function getConfig(): Promise<ConfigSnapshot> {
  return req<{ revision: number; config: any; warnings: string[] }>(`/config?project_id=${encodeURIComponent(projectId)}`).then((r) => { configRevision = r.revision; rawConfig = r.config; return ({
    provider: r.config.provider.name ?? "", model: r.config.model.name ?? "", max_tokens: 0, max_turns: r.config.limits.max_turns ?? 0, permission_mode: r.config.permission.mode === "workspace_write" ? "WorkspaceWrite" : r.config.permission.mode === "full_access" ? "FullAccess" : "ReadOnly", max_retries: 0, retry_base_backoff_ms: 0, request_timeout_secs: 0, run_retry_attempts: 0, run_retry_base_backoff_ms: 0, circuit_breaker_threshold: 0, circuit_breaker_cooldown_secs: 0, context_window: 0, theme: "", bell: true, stop_policy: { enabled: false, marker_gate: false, verify_gate: false, max_blocks: 0 }, route: { objective: "", fallback_models: [], max_fallbacks: 0, quality_hints: [] }, paths: { project_config: "", sessions_home: "", cwd: projectRoot }, warnings: r.warnings,
  }); });
}

export function listProviders(): Promise<import("./types").ProvidersResponse> {
  return req<import("./types").ProvidersResponse>("/providers");
}

/** Live model list for one provider, discovered from its API. */
export function discoverModels(provider: string): Promise<{ provider: string; models: string[] }> {
  return req<{ provider: string; models: string[] }>(`/providers/${encodeURIComponent(provider)}/models`);
}

export function putProviderKey(
  provider: string,
  key: string,
): Promise<{ provider: string; env_var: string; configured: boolean }> {
  return req<{ provider: string; env_var: string; configured: boolean }>(`/providers/${encodeURIComponent(provider)}/key`, { method: "POST", body: JSON.stringify({ key }) });
}

/** Revoke a provider key stored on this device. */
export function removeProviderKey(
  provider: string,
): Promise<{ provider: string; env_var: string; configured: boolean; shadowed_by_env: boolean }> {
  return req<{ provider: string; env_var: string; configured: boolean; shadowed_by_env: boolean }>(`/providers/${encodeURIComponent(provider)}/key`, { method: "DELETE", body: "{}" });
}

export function patchConfig(patch: { provider?: string; model?: string; max_turns?: number; permission_mode?: string; theme?: string }): Promise<void> {
  if (!rawConfig) return Promise.reject(new Error("config is not loaded"));
  const config = structuredClone(rawConfig) as Record<string, any>;
  if (patch.provider !== undefined) config.provider = { ...(config.provider ?? {}), name: patch.provider };
  if (patch.model !== undefined) config.model = { ...(config.model ?? {}), name: patch.model };
  if (patch.max_turns !== undefined) config.limits = { ...(config.limits ?? {}), max_turns: patch.max_turns };
  if (patch.permission_mode !== undefined) config.permission = { ...(config.permission ?? {}), mode: patch.permission_mode };
  if (patch.theme !== undefined) config.ui = { ...(config.ui ?? {}), theme: patch.theme };
  return req("/config", { method: "PATCH", body: JSON.stringify({ project_id: projectId, revision: configRevision, config }) }).then(() => undefined);
}

export function listCheckpoints(id: string): Promise<{
  checkpoints: { seq: number; label: string; created_at: string; files: number }[];
}> {
  return req<{ items: Array<{ id: string; label: string; created_at: string; manifest?: { files?: unknown[] } | null }> }>(`/sessions/${encodeURIComponent(id)}/checkpoints`)
    .then((r) => ({ checkpoints: r.items.map((item, seq) => ({ seq, label: item.label, created_at: item.created_at, files: Array.isArray(item.manifest?.files) ? item.manifest.files.length : 0 })) }));
}

export function restoreCheckpoint(
  id: string,
  seq: number,
): Promise<{ restored: number; deleted: number; seq: number }> {
  return req<{ items: Array<{ id: string }> }>(`/sessions/${encodeURIComponent(id)}/checkpoints`).then((r) => {
    const checkpoint = r.items[seq];
    if (!checkpoint) throw new Error("checkpoint does not exist");
    return req<{ restored: boolean }>(`/sessions/${encodeURIComponent(id)}/checkpoints/${encodeURIComponent(checkpoint.id)}/restore`, { method: "POST", body: "{}" })
      .then((result) => ({ restored: result.restored ? 1 : 0, deleted: 0, seq }));
  });
}

export function listSkills(): Promise<{
  skills: DiscoveredSkill[];
}> {
  return req<{ items: Array<{ name: string; description: string; project_id?: string | null; status: string }> }>(`/skills?project_id=${encodeURIComponent(projectId)}`)
    .then((r) => ({ skills: r.items.map((item) => ({ name: item.name, description: item.description, source: item.project_id ?? "runtime", scope: item.status })) }));
}

export interface FileResponse {
  path: string;
  /** "text" | "image" | "binary" — decides how the file can be shown. */
  kind: "text" | "image" | "binary";
  bytes: number;
  editable: boolean;
  /** Text only. */
  content?: string;
  /** Images only: a self-contained data: URL. */
  data_url?: string;
}

export function readFile(path: string): Promise<FileResponse> {
  return req<FileResponse>(`/fs/file?project_id=${encodeURIComponent(projectId)}&path=${encodeURIComponent(path)}`);
}

export function writeFile(path: string, content: string): Promise<unknown> {
  return req(`/fs/file?project_id=${encodeURIComponent(projectId)}`, { method: "PUT", body: JSON.stringify({ project_id: projectId, path, content }) });
}

export function fsTree(limit = 400): Promise<{ files: string[]; truncated: boolean }> {
  return req<{ files: string[]; truncated: boolean }>(`/fs/tree?project_id=${encodeURIComponent(projectId)}&limit=${limit}`);
}

// ---- SSE ---------------------------------------------------------------------

/**
 * Minimal stream handle: callers only ever close(). Implemented over fetch
 * so the bearer token travels in a header — EventSource cannot set headers
 * and the server (correctly) accepts no query-string tokens.
 */
export interface EventStream {
  close(): void;
}

function openSse(
  url: string,
  onEvent: (ev: AgentEvent) => void,
  onError?: () => void,
  sessionId?: string,
): EventStream {
  const ctrl = new AbortController();
  let attempts = 0;

  async function run() {
    for (;;) {
      if (ctrl.signal.aborted) return;
      try {
        const res = await fetch(url, {
          headers: { Authorization: `Bearer ${token}` },
          signal: ctrl.signal,
        });
        if (!res.ok || !res.body) {
          onError?.();
          return;
        }
        attempts = 0;
        const reader = res.body.getReader();
        const decoder = new TextDecoder();
        let buf = "";
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          buf += decoder.decode(value, { stream: true });
          let idx: number;
          while ((idx = buf.indexOf("\n")) >= 0) {
            const line = buf.slice(0, idx).replace(/\r$/, "");
            buf = buf.slice(idx + 1);
            if (line.startsWith("data:")) {
              const data = line.slice(5).trim();
              if (data) {
                try {
                  const value = JSON.parse(data) as unknown;
                  const event = adaptServerEvent(value, sessionId);
                  if (event) onEvent(event);
                } catch {
                  // ignore keep-alive/comment frames
                }
              }
            }
          }
        }
      } catch {
        if (!ctrl.signal.aborted) onError?.();
      }
      // Crash-only channels: reconnect with capped backoff until closed.
      if (ctrl.signal.aborted) return;
      const delay = Math.min(5000, 500 * 2 ** attempts++);
      await new Promise((r) => setTimeout(r, delay));
    }
  }

  void run();
  return { close: () => ctrl.abort() };
}

function adaptServerEvent(value: unknown, sessionId?: string): AgentEvent | null {
  if (!value || typeof value !== "object") return null;
  const event = value as Record<string, unknown>;
  const output = event.RunOutput;
  if (output && typeof output === "object") {
    const item = output as Record<string, unknown>;
    if (sessionId && activeRuns.get(sessionId) !== String(item.run_id ?? "")) return null;
    const delta = typeof item.delta === "string" ? item.delta : "";
    const snapshot = typeof item.snapshot === "string" ? item.snapshot : delta;
    const message: Message = { role: "assistant", content: [{ type: "text", text: snapshot }] };
    const assistant = { content: message.content, stop_reason: "", usage: {}, model: "" };
    return { Stream: { TextDelta: { delta, partial: assistant } } };
  }
  const finished = event.RunFinished;
  if (finished && typeof finished === "object") {
    const item = finished as Record<string, unknown>;
    if (sessionId && activeRuns.get(sessionId) !== String(item.run_id ?? "")) return null;
    const status = String(item.status ?? "failed");
    return { RunFinished: { summary: typeof item.output === "string" ? item.output : status, is_error: status !== "Completed" } };
  }
  return null;
}

export function openEventStream(
  id: string,
  onEvent: (ev: AgentEvent) => void,
  onError?: () => void,
): EventStream {
  return openSse(`${base}/events`, onEvent, onError, id);
}

export function openSideStream(
  id: string,
  onEvent: (ev: AgentEvent) => void,
): EventStream {
  void id;
  return openSse(`${base}/events`, onEvent);
}

export function listTasks(): Promise<{ tasks: import("./types").TaskDef[] }> {
  return req<{ items: Array<{ id: string; spec: any; status: string; updated_at: string }> }>(`/tasks?project_id=${encodeURIComponent(projectId)}`).then((r) => ({ tasks: r.items.map((t) => ({ id: t.id, name: t.spec.name ?? t.id, prompt: t.spec.prompt ?? "", interval_secs: t.spec.interval_secs ?? 0, enabled: t.status === "enabled" || t.spec.enabled !== false, cwd: projectRoot, created_at: t.updated_at, schedule: t.spec.schedule ?? null, script: t.spec.script ?? null, model_pin: t.spec.model_pin ?? null })) }));
}

export interface TaskDraft {
  name: string;
  prompt: string;
  interval_secs: number;
  schedule?: string | null;
  script?: string | null;
  model_pin?: string | null;
}

export function createTask(draft: TaskDraft): Promise<unknown> {
  return req("/tasks", { method: "POST", body: JSON.stringify({ project_id: projectId, spec: draft }) });
}

/**
 * Tri-state optional strings mirror the server: absent key = keep current,
 * explicit null = clear. `JSON.stringify` drops undefined keys, so callers
 * express "keep" by simply not setting the field.
 */
export type TaskPatch = Partial<{
  enabled: boolean;
  name: string;
  prompt: string;
  interval_secs: number;
  schedule: string | null;
  script: string | null;
  model_pin: string | null;
}>;

export function patchTask(id: string, patch: TaskPatch): Promise<import("./types").TaskDef> {
  return req(`/tasks/${encodeURIComponent(id)}`, { method: "PATCH", body: JSON.stringify({ spec: patch, status: patch.enabled === undefined ? undefined : patch.enabled ? "enabled" : "disabled" }) });
}

export function deleteTask(id: string): Promise<unknown> {
  return req(`/tasks/${encodeURIComponent(id)}`, { method: "DELETE" });
}

export function runTaskNow(id: string): Promise<unknown> {
  return req(`/tasks/${encodeURIComponent(id)}`, { method: "PATCH", body: JSON.stringify({ status: "enabled" }) });
}

// ---- personal-os surfaces (docs/design/29-personal-os.md) ---------------------

export interface DoctorCheck {
  label: string;
  ok: boolean;
  detail: string;
}

export interface DoctorLadder {
  legs: string[];
  rendered: string;
  objective: string;
  fallback_legs: number;
  annotations: string[];
}

export interface DoctorReport {
  failures: number;
  checks: DoctorCheck[];
  facts: string[];
  ladder?: DoctorLadder | null;
}

/** `?session=` optionally adds that session's frozen-ladder section. */
export function doctor(session?: string): Promise<DoctorReport> {
  void session;
  return req<{ status: string; details: { [key: string]: unknown } }>("/diagnostics").then((r) => ({ failures: r.status === "ok" ? 0 : 1, checks: [{ label: "Runtime", ok: r.status === "ok", detail: r.status }], facts: Object.entries(r.details).map(([key, value]) => `${key}: ${String(value)}`) }));
}

export interface BackupManifest {
  version: number;
  timestamp: string;
  file_count: number;
  total_bytes: number;
}

export function backupExport(
  destDir: string,
  includeSecrets: boolean,
): Promise<{ manifest: BackupManifest; included_secrets: boolean }> {
  return req<{ file_count: number; total_bytes: number }>("/backup/export", { method: "POST", body: JSON.stringify({ directory: destDir, include_secrets: includeSecrets }) })
    .then((r) => ({ manifest: { version: 1, timestamp: new Date().toISOString(), file_count: r.file_count, total_bytes: r.total_bytes }, included_secrets: includeSecrets }));
}

export interface ImportReportShape {
  copied: number;
  renamed: number;
  skipped: number;
}

export function backupImport(srcDir: string, conflict: "skip" | "rename"): Promise<ImportReportShape> {
  return req<{ file_count: number; renamed: number; skipped: number }>("/backup/import", { method: "POST", body: JSON.stringify({ directory: srcDir, conflict }) })
    .then((r) => ({ copied: r.file_count, renamed: r.renamed, skipped: r.skipped }));
}

// ---- inbox (docs/design/29-personal-os.md P6) ---------------------------------

export interface InboxEntry {
  id: string;
  ts: string;
  /** Server enum tag: task_summary | approval_pending | approval_denied |
   *  budget_alert | digest | heartbeat | proposal_opened (snake_case). */
  kind: string;
  title: string;
  body: string;
  session_id?: string | null;
  task_id?: string | null;
}

export function listInbox(
  limit: number,
  unreadOnly: boolean,
): Promise<{ entries: InboxEntry[]; unread_count: number }> {
  void limit;
  return req<{ items: Array<{ id: string; payload: any; created_at: string; acknowledged_at?: string | null }> }>(`/inbox?unread=${unreadOnly}`).then((r) => ({ entries: r.items.map((e) => ({ id: e.id, ts: e.created_at, kind: e.payload.kind ?? "notice", title: e.payload.title ?? "Runtime notice", body: e.payload.body ?? JSON.stringify(e.payload), session_id: e.payload.session_id ?? null, task_id: e.payload.task_id ?? null })), unread_count: r.items.filter((e) => !e.acknowledged_at).length }));
}

/** Idempotent read-state tombstone; unknown ids come back as a 404 error. */
export function ackInbox(id: string): Promise<{ acked: boolean }> {
  return req(`/inbox/${encodeURIComponent(id)}/ack`, { method: "POST", body: "{}" }).then(() => ({ acked: true }));
}

export function inboxUnreadCount(): Promise<{ count: number }> {
  return listInbox(0, true).then((r) => ({ count: r.unread_count }));
}
