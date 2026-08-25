import { invoke } from "@tauri-apps/api/core";
import type {
  AgentEvent,
  BackendInfo,
  DiffResponse,
  Health,
  Message,
  SessionSummary,
  ConfigSnapshot,
  WorkReceipt,
} from "./types";

let base = "";
let token = "";

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
  } else {
    base = "";
    token = "";
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
  telegram: { state: string };
  gateway_healthy: boolean;
}

export function opsStatus(): Promise<OpsStatusShape> {
  return req("/ops/status");
}

export function opsAction(
  service: "gateway" | "telegram",
  action: "start" | "stop" | "restart" | "install" | "uninstall",
): Promise<{ ok: boolean; error?: string }> {
  return req(`/ops/${service}/${action}`, { method: "POST", body: "{}" });
}

export interface OpsDiagnostics {
  health: { status: string; provider: string; model: string; sandbox: string; permission_mode: string; warnings: string[] };
  services: OpsStatusShape;
  gateway: { enabled: boolean; bindings: { target: string; session_id: string }[]; approvals: { mode: string; approver?: string | null; pending: number } };
  flows: { name: string; runs: number }[];
}

export function opsDiagnostics(): Promise<OpsDiagnostics> {
  return req("/ops/diagnostics");
}

export interface FinopsStatus {
  day_usd: number;
  run_cap_usd?: number | null;
  day_cap_usd?: number | null;
  unknown_rows: number;
  total_rows: number;
  by_provider: { name: string; usd: number; calls: number }[];
  by_model: { name: string; usd: number; calls: number }[];
}

export function finopsStatus(): Promise<FinopsStatus> {
  return req("/finops");
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
  return req(`/memory/${encodeURIComponent(id)}?scope=${scope}`, { method: "DELETE" });
}

export function amendMemory(id: string, scope: MemoryScope, text: string): Promise<{ amended: string }> {
  return req(`/memory/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: JSON.stringify({ text, scope }),
  });
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
  const params = new URLSearchParams({ q, limit: String(limit), all: String(all) });
  return req(`/search?${params.toString()}`);
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

export interface HookConfig {
  event: string;
  matcher?: string | null;
  command: string;
  timeout_ms?: number | null;
  enabled?: boolean;
}

export function getHooks(): Promise<{ hooks: HookConfig[] }> {
  return req("/config/hooks");
}

export function putHooks(hooks: HookConfig[]): Promise<{ saved: boolean; count: number }> {
  return req("/config/hooks", { method: "PUT", body: JSON.stringify({ hooks }) });
}

export function listMemory(): Promise<{ notes: NoteBlock[] }> {
  return req("/memory");
}

export function listProposals(): Promise<{ proposals: SkillProposal[] }> {
  return req("/skills/proposals");
}

export function promoteProposal(id: string): Promise<{ promoted: string }> {
  return req(`/skills/proposals/${encodeURIComponent(id)}/promote`, { method: "POST", body: "{}" });
}

export function rejectProposal(id: string): Promise<{ rejected: string }> {
  return req(`/skills/proposals/${encodeURIComponent(id)}/reject`, { method: "POST", body: "{}" });
}

// ---- sessions ---------------------------------------------------------------

export function listSessions(): Promise<{ sessions: SessionSummary[] }> {
  return req("/sessions");
}

export function createSession(): Promise<{ session_id: string }> {
  return req("/sessions", { method: "POST", body: "{}" });
}

export function attachSession(id: string): Promise<{ session_id: string }> {
  return req(`/sessions/${id}/attach`, {
    method: "POST",
    body: JSON.stringify({ session_id: id }),
  });
}

export function transcript(id: string): Promise<{
  count: number;
  usage: Record<string, number>;
  messages: Message[];
}> {
  return req(`/sessions/${id}/transcript`);
}

/** Markdown export (shared renderer with the TUI); text, not JSON. */
export async function transcriptMarkdown(id: string): Promise<string> {
  const res = await fetch(`${base}/sessions/${encodeURIComponent(id)}/transcript.md`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  return res.text();
}

export function runPrompt(
  id: string,
  prompt: string,
  goal?: { objective: string; criteria: string[] },
  attachments?: { mime: string; data: string }[],
): Promise<void> {
  return req(`/sessions/${id}/run`, {
    method: "POST",
    body: JSON.stringify({
      prompt,
      goal: goal?.objective,
      criteria: goal?.criteria,
      attachments: attachments ?? [],
    }),
  });
}

// ---- subagents (attach / steer / stop) ---------------------------------------

export interface ActiveSubagent {
  id: string;
  label: string;
  elapsed_secs: number;
  parent_session_id: string;
}

export function listSubagents(id: string): Promise<{ subagents: ActiveSubagent[] }> {
  return req(`/sessions/${id}/subagents`);
}

export function steerSubagent(id: string, child: string, text: string): Promise<void> {
  return req(`/sessions/${id}/subagents/${encodeURIComponent(child)}/steer`, {
    method: "POST",
    body: JSON.stringify({ text }),
  });
}

export function stopSubagent(
  id: string,
  child: string,
): Promise<void> {
  return req(`/sessions/${id}/subagents/${encodeURIComponent(child)}/stop`, { method: "POST" });
}

/// Dispatch forensics (docs/design/27 Phases A+B+R): per-dispatch receipts
/// with the full frozen-ladder attempt ledger.
export function receipts(id: string): Promise<WorkReceipt[]> {
  return req(`/sessions/${id}/receipts`);
}

export function steer(id: string, text: string, attachments?: { mime: string; data: string }[]): Promise<void> {
  return req(`/sessions/${id}/steering`, {
    method: "POST",
    body: JSON.stringify({ text, attachments: attachments ?? [] }),
  });
}

export function cancelRun(id: string): Promise<void> {
  return req(`/sessions/${id}/cancel`, { method: "POST" });
}

export function runSide(id: string, question: string): Promise<void> {
  return req(`/sessions/${id}/side`, {
    method: "POST",
    body: JSON.stringify({ question }),
  });
}

export function cancelSide(id: string): Promise<void> {
  return req(`/sessions/${id}/side/cancel`, { method: "POST" });
}

export function answerApproval(
  id: string,
  requestId: string,
  approve: boolean,
): Promise<unknown> {
  return req(`/sessions/${id}/approvals/${requestId}`, {
    method: "POST",
    body: JSON.stringify({ approve }),
  });
}

export function health(): Promise<Health> {
  // /health is intentionally open; still send the header for consistency.
  return req("/health");
}

export function setPermissionMode(mode: string): Promise<void> {
  return req("/config/mode", { method: "POST", body: JSON.stringify({ mode }) });
}

export function getConfig(): Promise<ConfigSnapshot> {
  return req("/config");
}

export function listProviders(): Promise<import("./types").ProvidersResponse> {
  return req("/providers");
}

/** Live model list for one provider, discovered from its API. */
export function discoverModels(provider: string): Promise<{ provider: string; models: string[] }> {
  return req(`/providers/${encodeURIComponent(provider)}/models`);
}

export function putProviderKey(
  provider: string,
  key: string,
): Promise<{ provider: string; env_var: string; configured: boolean }> {
  return req("/config/key", {
    method: "PUT",
    body: JSON.stringify({ provider, key }),
  });
}

/** Revoke a provider key stored on this device. */
export function removeProviderKey(
  provider: string,
): Promise<{ provider: string; env_var: string; configured: boolean; shadowed_by_env: boolean }> {
  return req("/config/key", {
    method: "DELETE",
    body: JSON.stringify({ provider }),
  });
}

export function patchConfig(patch: { provider?: string; model?: string; max_turns?: number; permission_mode?: string; theme?: string }): Promise<void> {
  return req("/config", { method: "PATCH", body: JSON.stringify(patch) });
}

// ---- MCP server management ----------------------------------------------------

export interface McpServerDef {
  command: string;
  args: string[];
  env: Record<string, string>;
  network: boolean;
}

export function getMcpServers(): Promise<{ servers: Record<string, McpServerDef> }> {
  return req("/config/mcp");
}

/** Replaces the whole running table and persists the project config. */
export function putMcpServers(servers: Record<string, McpServerDef>): Promise<{ saved: boolean; count: number }> {
  return req("/config/mcp", { method: "PUT", body: JSON.stringify({ servers }) });
}

export function readDiff(id: string): Promise<DiffResponse> {
  return req(`/sessions/${id}/diff`);
}

export function listCheckpoints(id: string): Promise<{
  checkpoints: { seq: number; label: string; created_at: string; files: number }[];
}> {
  return req(`/sessions/${id}/checkpoints`);
}

export function restoreCheckpoint(
  id: string,
  seq: number,
): Promise<{ restored: number; deleted: number; seq: number }> {
  return req(`/sessions/${id}/checkpoints/${seq}/restore`, { method: "POST" });
}

export function setArchived(id: string, archived: boolean): Promise<{ archived: boolean }> {
  return req(`/sessions/${id}/archive`, {
    method: "POST",
    body: JSON.stringify({ archived }),
  });
}

export function deleteSession(id: string): Promise<{ deleted: string }> {
  return req(`/sessions/${id}`, { method: "DELETE" });
}

export function deleteAllArchived(): Promise<{ deleted: number }> {
  return req("/sessions/archived", { method: "DELETE" });
}

export function listSkills(): Promise<{
  skills: DiscoveredSkill[];
}> {
  return req("/skills");
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
  return req(`/fs/file?path=${encodeURIComponent(path)}`);
}

export function writeFile(path: string, content: string): Promise<unknown> {
  return req("/fs/file", { method: "PUT", body: JSON.stringify({ path, content }) });
}

export function fsTree(limit = 400): Promise<{ files: string[]; truncated: boolean }> {
  return req(`/fs/tree?limit=${limit}`);
}

export function startBestOfN(
  anchorId: string,
  prompt: string,
  n: number,
): Promise<{ runs: { session_id: string; branch: string; path: string }[] }> {
  return req(`/sessions/${anchorId}/bestofn`, {
    method: "POST",
    body: JSON.stringify({ prompt, n }),
  });
}

export function keepRun(childId: string): Promise<{ kept: string }> {
  return req(`/sessions/${childId}/keep`, { method: "POST" });
}

export function discardRun(childId: string): Promise<{ discarded: string }> {
  return req(`/sessions/${childId}/discard`, { method: "POST" });
}

export function getPr(id: string): Promise<import("./types").PrStatus> {
  return req(`/sessions/${id}/pr`);
}

export function mergePr(
  id: string,
  number: number,
  method: "squash" | "merge" | "rebase" = "squash",
): Promise<{ merging: number }> {
  return req(`/sessions/${id}/pr/merge`, {
    method: "POST",
    body: JSON.stringify({ number, method }),
  });
}

// ---- SSE ---------------------------------------------------------------------

export function openEventStream(
  id: string,
  onEvent: (ev: AgentEvent) => void,
  onError?: () => void,
): EventSource {
  const es = new EventSource(`${base}/sessions/${id}/events?token=${encodeURIComponent(token)}`);
  es.onmessage = (m) => {
    try {
      onEvent(JSON.parse(m.data) as AgentEvent);
    } catch {
      // ignore keep-alive/comment frames
    }
  };
  es.onerror = () => onError?.();
  return es;
}

export function openSideStream(
  id: string,
  onEvent: (ev: AgentEvent) => void,
): EventSource {
  const es = new EventSource(
    `${base}/sessions/${id}/side/events?token=${encodeURIComponent(token)}`,
  );
  es.onmessage = (m) => {
    try {
      onEvent(JSON.parse(m.data) as AgentEvent);
    } catch {
      // ignore keep-alive frames
    }
  };
  return es;
}

export function listTasks(): Promise<{ tasks: import("./types").TaskDef[] }> {
  return req("/tasks");
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
  return req("/tasks", { method: "POST", body: JSON.stringify(draft) });
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
  return req(`/tasks/${id}`, { method: "PATCH", body: JSON.stringify(patch) });
}

export function deleteTask(id: string): Promise<unknown> {
  return req(`/tasks/${id}`, { method: "DELETE" });
}

export function runTaskNow(id: string): Promise<unknown> {
  return req(`/tasks/${id}/run-now`, { method: "POST" });
}

export function getLaunch(id: string): Promise<{
  servers: { name: string; cmd: string; args: string[]; port: number | null; running: boolean }[];
  error?: string;
}> {
  return req(`/sessions/${id}/launch`);
}

export function startLaunch(id: string, name: string): Promise<{ started: boolean; listening: boolean; error?: string }> {
  return req(`/sessions/${id}/launch/start`, { method: "POST", body: JSON.stringify({ name }) });
}

export function stopLaunch(id: string, name: string): Promise<unknown> {
  return req(`/sessions/${id}/launch/stop`, { method: "POST", body: JSON.stringify({ name }) });
}

export function launchLogs(id: string, name: string): Promise<{ lines: string[] }> {
  return req(`/sessions/${id}/launch/logs?name=${encodeURIComponent(name)}`);
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
  const q = session ? `?session=${encodeURIComponent(session)}` : "";
  return req(`/doctor${q}`);
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
  return req("/backup/export", {
    method: "POST",
    body: JSON.stringify({ dest_dir: destDir, include_secrets: includeSecrets }),
  });
}

export interface ImportReportShape {
  copied: number;
  renamed: number;
  skipped: number;
}

export function backupImport(srcDir: string, conflict: "skip" | "rename"): Promise<ImportReportShape> {
  return req("/backup/import", {
    method: "POST",
    body: JSON.stringify({ src_dir: srcDir, conflict }),
  });
}

export interface DigestModelRollup {
  rows: number;
  usd: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
}

export interface DigestReport {
  days: number;
  since?: string | null;
  total_usd: number;
  unpriced_rows: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  dispatches: number;
  by_model: Record<string, DigestModelRollup>;
  per_day: { day: string; usd: number; unpriced_rows: number }[];
  distinct_sessions: string[];
  memory_notes_appended: number;
  skill_proposals_opened: number;
}

export function digest(days: number): Promise<DigestReport> {
  return req(`/digest?days=${days}`);
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
  const params = new URLSearchParams({ limit: String(limit), unread: String(unreadOnly) });
  return req(`/inbox?${params.toString()}`);
}

/** Idempotent read-state tombstone; unknown ids come back as a 404 error. */
export function ackInbox(id: string): Promise<{ acked: boolean }> {
  return req(`/inbox/${encodeURIComponent(id)}/ack`, { method: "POST", body: "{}" });
}

export function inboxUnreadCount(): Promise<{ count: number }> {
  return req("/inbox/unread_count");
}
