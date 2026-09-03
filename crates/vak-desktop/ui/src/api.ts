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
  OutputTimeline,
  PresentationStreamEvent,
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
  bridges: { state: string };
  gateway_healthy: boolean;
}

export function opsStatus(): Promise<OpsStatusShape> {
  return req("/ops/status");
}

export function opsAction(
  service: "gateway" | "bridges",
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
  provenance?: string | null;
  shadowed?: boolean;
}

export interface CustomCommand { name: string; description: string; source?: string; }

export function listCommands(): Promise<{ commands: CustomCommand[] }> {
  return req("/commands");
}

export interface InstalledPlugin {
  name: string;
  version: string;
  digest: string;
  description: string;
  format: string;
  scope: "workspace" | "user";
  enabled: boolean;
  trace_id: string;
  capabilities: Record<string, unknown>;
  warnings: string[];
}

export interface MarketplaceSource {
  id: string;
  label: string;
  root: string;
  format: string;
  catalog_digest: string;
  trace_id: string;
  trust: string;
  enabled: boolean;
  registered_at_unix: number;
  signature?: { algorithm: string; key_id: string; public_key: string; signature: string; verified: boolean; revoked: boolean } | null;
}

export interface MarketplaceEntry {
  source_id: string;
  source_label: string;
  source_enabled: boolean;
  source_scope: "user" | "workspace";
  catalog_digest: string;
  name: string;
  version?: string | null;
  description?: string | null;
  license?: string | null;
}

export interface HookConfig {
  event: string;
  matcher?: string | null;
  command: string;
  timeout_ms?: number | null;
  enabled?: boolean;
  failure_mode?: "open" | "closed";
}

// Prompt layers (docs/design/45-prompt-layers.md). Same endpoints the admin
// console uses — doc 44 requires Desktop and Admin to exercise identical
// scope contracts, so this is one API, not a parallel one.
export type PromptBlock = "identity" | "operating-rules" | "guardrails" | "surface-note";

export interface PromptLayerContent {
  identity?: string | null;
  operating_rules?: string | null;
  guardrails?: string[];
  surface_notes?: string[];
}

export interface PromptLayerDescriptor {
  block: PromptBlock;
  layer: "seed" | "shared" | "project" | "surface" | "bot" | "chat" | "agent";
  source?: string | null;
  digest: string;
  bytes: number;
}

export function getPromptLayer(scope: "user" | "project"): Promise<{ scope: string; path: string; layer: PromptLayerContent }> {
  return req(`/config/prompts?scope=${scope}`);
}

export function putPromptBlock(scope: "user" | "project", block: PromptBlock, text: string | null): Promise<unknown> {
  return req("/config/prompts", { method: "PUT", body: JSON.stringify({ scope, block, text }) });
}

export function getPromptEffective(): Promise<{ text: string; fingerprint: string; estimated_tokens: number; surface: string; layers: PromptLayerDescriptor[] }> {
  return req("/config/prompts/effective");
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

export function appendMemory(scope: MemoryScope, text: string, kind = "fact", tag = ""): Promise<NoteBlock> {
  return req("/memory", {
    method: "POST",
    body: JSON.stringify({ scope, text, kind, tag }),
  });
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

export function presentation(id: string): Promise<OutputTimeline> {
  return req(`/sessions/${encodeURIComponent(id)}/presentation`);
}

export function openPresentationStream(
  id: string,
  onEvent: (event: PresentationStreamEvent) => void,
  onError?: () => void,
): EventSource {
  const es = new EventSource(`${base}/sessions/${encodeURIComponent(id)}/presentation/events?token=${encodeURIComponent(token)}`);
  es.onmessage = (message) => {
    try {
      onEvent(JSON.parse(message.data) as PresentationStreamEvent);
    } catch {
      // Ignore keep-alive and malformed frames; the legacy stream remains the fallback.
    }
  };
  es.onerror = () => onError?.();
  return es;
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

export function work(id: string): Promise<import("./types").WorkProjection | null> {
  return req(`/sessions/${encodeURIComponent(id)}/work`);
}

export function workCommand(
  id: string,
  command: Record<string, unknown>,
): Promise<import("./types").WorkProjection> {
  return req(`/sessions/${encodeURIComponent(id)}/work`, {
    method: "POST",
    body: JSON.stringify(command),
  });
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

export interface ApprovalAnswer {
  approved: boolean;
  /// The rule that was persisted, when the answer asked not to be asked
  /// again. Null when nothing was remembered.
  learned_rule: string | null;
  /// Why no rule could be derived. Never blocks the approval: some calls
  /// (opaque shell, a one-off URL) have no shape that generalizes safely.
  learn_error: string | null;
}

export function answerApproval(
  id: string,
  requestId: string,
  approve: boolean,
  remember = false,
): Promise<ApprovalAnswer> {
  return req(`/sessions/${id}/approvals/${requestId}`, {
    method: "POST",
    body: JSON.stringify({ approve, remember }),
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

export interface ConfigPatch {
  provider?: string;
  model?: string;
  max_turns?: number;
  permission_mode?: string;
  /// "ask" | "approve-safe" | "auto-approve". The server has accepted this
  /// since approval modes shipped; the desktop just never sent it, so the
  /// one control deciding how gates resolve was browser-only.
  approval_mode?: string;
  theme?: string;
}

export function patchConfig(patch: ConfigPatch): Promise<void> {
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

/** User-scope inventory inherited by every project unless that project overrides it. */
export function getGlobalMcpServers(): Promise<{ scope: "user"; path: string; servers: Record<string, McpServerDef> }> {
  return req("/config/mcp/global");
}

export function putGlobalMcpServers(servers: Record<string, McpServerDef>): Promise<{ saved: boolean; scope: "user"; count: number }> {
  return req("/config/mcp/global", { method: "PUT", body: JSON.stringify({ servers }) });
}

export function patchGlobalConfig(body: ConfigPatch): Promise<void> {
  return req("/config/global", { method: "PATCH", body: JSON.stringify(body) });
}

export function getGlobalHooks(): Promise<{ scope: "user"; hooks: HookConfig[] }> {
  return req("/config/hooks/global");
}

export function putGlobalHooks(hooks: HookConfig[]): Promise<{ saved: boolean; scope: "user"; count: number }> {
  return req("/config/hooks/global", { method: "PUT", body: JSON.stringify({ hooks }) });
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

export function listPlugins(scope?: "user" | "workspace"): Promise<{ plugins: InstalledPlugin[] }> {
  return req(`/plugins${scope ? `?scope=${scope}` : ""}`);
}

export function listPluginSources(scope?: "user" | "workspace"): Promise<{ sources: MarketplaceSource[] }> {
  return req(`/plugins/sources${scope ? `?scope=${scope}` : ""}`);
}

export function listPluginCatalog(query = "", scope?: "user" | "workspace"): Promise<{ entries: MarketplaceEntry[]; errors: { source_id?: string; error: string }[] }> {
  const params = new URLSearchParams();
  if (query.trim()) params.set("q", query.trim());
  if (scope) params.set("scope", scope);
  const suffix = params.toString() ? `?${params.toString()}` : "";
  return req(`/plugins/catalog${suffix}`);
}

export function registerPluginSource(path: string, label: string, signature?: { key_id: string; public_key: string; signature: string }): Promise<MarketplaceSource> {
  return req("/plugins/sources", { method: "POST", body: JSON.stringify({ path, label, trust: "manual-review", ...(signature ?? {}) }) });
}

export function pluginKeyAction(keyId: string, action: "revoke" | "restore", scope: "user" | "workspace"): Promise<unknown> {
  return req(`/plugins/keys/${encodeURIComponent(keyId)}/${action}?scope=${scope}`, { method: "POST", body: "{}" });
}

export function pluginSourceAction(id: string, action: "enable" | "disable", scope: "user" | "workspace"): Promise<MarketplaceSource> {
  return req(`/plugins/sources/${encodeURIComponent(id)}/${action}?scope=${scope}`, { method: "POST", body: "{}" });
}

export function installPlugin(path: string, scope: "workspace" | "user"): Promise<InstalledPlugin> {
  return req("/plugins/install", { method: "POST", body: JSON.stringify({ path, scope }) });
}

export function updatePlugin(path: string, scope: "workspace" | "user"): Promise<InstalledPlugin> {
  return req("/plugins/update", { method: "POST", body: JSON.stringify({ path, scope }) });
}

export function pluginAction(name: string, action: "enable" | "disable" | "rollback", scope: "user" | "workspace"): Promise<InstalledPlugin> {
  return req(`/plugins/${encodeURIComponent(name)}/${action}?scope=${scope}`, { method: "POST", body: "{}" });
}

export function removePlugin(name: string, scope: "user" | "workspace"): Promise<InstalledPlugin> {
  return req(`/plugins/${encodeURIComponent(name)}?scope=${scope}`, { method: "DELETE" });
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

// ---- voice (docs/design: Voice & Personality for vak) -------------------------

/** POSTs to /voice/speak and returns the raw audio/wav bytes as a Blob.
 * Unlike req<T>(), the success body is audio, not JSON — only the error
 * path parses JSON, mirroring req()'s error-shape handling. */
export async function speak(
  text: string,
  opts?: { voiceName?: string; persona?: string },
): Promise<Blob> {
  const voice_override =
    opts?.voiceName || opts?.persona
      ? { voice_name: opts?.voiceName || undefined, persona: opts?.persona || undefined }
      : undefined;
  const res = await fetch(`${base}/voice/speak`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({ text, voice_override }),
  });
  if (!res.ok) {
    const text = await res.text();
    let parsed: unknown = null;
    try {
      parsed = text ? JSON.parse(text) : null;
    } catch {
      parsed = text;
    }
    const msg =
      (parsed as { error?: string })?.error ?? `${res.status} ${res.statusText}`;
    throw new Error(msg);
  }
  return res.blob();
}

// ---- SSE ---------------------------------------------------------------------

export function openEventStream(
  id: string,
  onEvent: (ev: AgentEvent) => void,
  onError?: () => void,
): EventSource {
  const es = new EventSource(`${base}/sessions/${id}/events?token=${encodeURIComponent(token)}`);
  es.onmessage = (m) => {
    // Only the parse is allowed to fail silently -- a genuine keep-alive
    // or comment frame is not valid JSON, and that is expected. `onEvent`
    // runs outside this catch on purpose: it previously shared the same
    // try/catch, so a bug in the caller's own handling of a successfully
    // parsed event -- including RunFinished, the one that clears
    // "running" state and reveals the reply -- vanished with no trace.
    // The backend would complete a turn and durably log it correctly
    // while the UI stayed on "Working" forever with nothing in the
    // console to explain why, because nothing had actually failed at
    // the connection level: the exception was caught and discarded.
    let parsed: AgentEvent;
    try {
      parsed = JSON.parse(m.data) as AgentEvent;
    } catch {
      return; // not JSON: a keep-alive or comment frame, not an error
    }
      try {
        onEvent(parsed);
      } catch (eventErr) {
        // Report and move on rather than either vanish (the defect this
        // replaces) or take the whole stream down over one bad event.
        console.error("vak: error handling agent event", parsed, eventErr);
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

// ---- feeds -----------------------------------------------------------

import type { FeedSourceType, FeedStats, FeedSearchResponse, FeedItem } from "./types";

export function feedSourceTypes(): Promise<{ source_types: FeedSourceType[] }> {
  return req("/feeds/sources");
}

export function feedStats(): Promise<FeedStats> {
  return req("/feeds/stats");
}

export function feedSearch(params: {
  q: string;
  tags?: string;
  since?: string;
  limit?: number;
  source?: string;
}): Promise<FeedSearchResponse> {
  const qs = new URLSearchParams({ q: params.q });
  if (params.tags) qs.set("tags", params.tags);
  if (params.since) qs.set("since", params.since);
  if (params.limit) qs.set("limit", String(params.limit));
  if (params.source) qs.set("source", params.source);
  return req(`/feeds/search?${qs.toString()}`);
}

export function feedItems(params?: {
  limit?: number;
  source?: string;
}): Promise<{ items: FeedItem[] }> {
  const qs = new URLSearchParams();
  if (params?.limit) qs.set("limit", String(params.limit));
  if (params?.source) qs.set("source", params.source);
  const q = qs.toString();
  return req(`/feeds/items${q ? `?${q}` : ""}`);
}

export function feedIngest(): Promise<{
  sources_ingested: number;
  new_items: number;
  alerts_fired: number;
  errors: number;
}> {
  return req("/feeds/ingest", { method: "POST", body: "{}" });
}

export function feedDeleteSource(name: string): Promise<{ status: string }> {
  return req(`/feeds/sources/${encodeURIComponent(name)}`, { method: "DELETE" });
}

import type { OnboardingState } from "./types";

/// The derived setup projection. Shared with the web wizard and the CLI:
/// one definition of "ready", never a per-surface guess.
export function onboarding(): Promise<OnboardingState> {
  return req<OnboardingState>("/onboarding");
}

// ---- intent kernel (docs/design/47-commitment-kernel.md) ---------------------

export type Satisfaction = "asserted" | "cited" | "observed" | "attested";

export interface IntentReading {
  act: string;
  horizon: string;
  stakes: string;
  evidence: string;
  clarity: string;
  attendance: string;
  domains: string[];
  confidence: number;
  axis_confidence: { act: number; horizon: number; stakes: number; evidence: number };
}

export interface IntentSignal {
  kind: string;
  name: string;
  weight: number;
  detail: string;
}

export interface IntentExplain {
  reading: IntentReading;
  engagement: {
    limits: {
      capabilities: { kind: "all" } | { kind: "only"; names: string[] };
      approval_ceiling: "ask" | "approve-safe" | "auto-approve";
      min_satisfaction: Satisfaction;
      ladder_limit?: number | null;
      max_turns?: number | null;
    };
    posture: {
      managed: boolean;
      open_commitment: boolean;
      checkpoint_before_effect: boolean;
      hil: "interrupt" | "envelope" | "review" | "defer";
      clarify: "proceed" | "state-assumption" | "ask";
      stop: string;
      context: string;
      delivery: { shape: string; cadence: string; urgency: string };
      note?: string | null;
    };
  };
  provenance: {
    tier: string;
    reproducible: boolean;
    signals: IntentSignal[];
    escalation_note?: string | null;
  };
  narrows: string[];
  model_visible?: string | null;
}

/** Resolve a prompt without running it. Free tiers only: this costs nothing
 * and dispatches nothing, which is what makes it safe to call while typing. */
export function explainIntent(
  prompt: string,
  signal?: AbortSignal,
): Promise<IntentExplain> {
  const params = new URLSearchParams({ prompt, surface: "desktop" });
  return req<IntentExplain>(`/intent/explain?${params}`, { signal });
}
