import { host } from "./host";
import type {
  AgentEvent,
  BackendInfo,
  DiffResponse,
  Health,
  Message,
  TranscriptEntryMeta,
  SessionSummary,
  ConfigSnapshot,
  WorkReceipt,
  OutputTimeline,
  PresentationStreamEvent,
  VoiceProvidersResponse,
} from "./types";

/**
 * How this client authenticates, which differs by host and cannot be made
 * uniform (docs/design/48-web-client.md §4.3):
 *
 * - **cookie** (web). The bundle is served by the very server it calls, so
 *   everything is same-origin and an HttpOnly `vak_session` cookie covers
 *   both `fetch` and `EventSource`. No token ever reaches script, and none
 *   ever appears in a URL.
 *
 * - **bearer** (desktop). The webview's origin is the Tauri asset protocol
 *   and the embedded server's is `http://127.0.0.1:<ephemeral>`, so a
 *   cookie set by the latter is third-party to the former and modern
 *   webviews decline to send it. `fetch` therefore carries an
 *   `Authorization` header, and `EventSource` — which cannot set headers at
 *   all — carries `?token=`. That is a real exposure in general and a
 *   non-exposure here: an in-process server on loopback, with a token
 *   minted per boot, no proxy and no access log between the two.
 */
type Auth =
  | { mode: "cookie" }
  | { mode: "bearer"; base: string; token: string };

let auth: Auth = { mode: "cookie" };

/** Origin prefix for API calls: empty on the web (same-origin). */
export function backendUrl(): string {
  return auth.mode === "bearer" ? auth.base : "";
}

/** Token for transports such as WebSocket that cannot set Authorization. */
export function backendToken(): string | undefined {
  return auth.mode === "bearer" ? auth.token : undefined;
}

export async function initBackend(): Promise<BackendInfo> {
  return host.info();
}

export function adoptBackend(info: BackendInfo): void {
  if (info.ready && info.base_url && info.token) {
    auth = { mode: "bearer", base: info.base_url, token: info.token };
  } else if (host.kind === "web") {
    // The web host has no base URL to adopt and never loses its cookie by
    // a workspace switch — it is always same-origin, always cookie.
    auth = { mode: "cookie" };
  } else {
    // Desktop with no live backend: no credentials to speak with yet.
    auth = { mode: "bearer", base: "", token: "" };
  }
}

export function isBackendReady(): boolean {
  return auth.mode === "cookie" || !!auth.base;
}

export function listVoiceProviders(): Promise<VoiceProvidersResponse> {
  return req<VoiceProvidersResponse>("/voice/providers");
}

/** Append `&agent=`/`?agent=` to a URL that may already carry query
 * params — mirrors `listMemory`'s existing `agent` param, threaded through
 * hooks/MCP/plugins/commitments/finops the same way (see commit 15c9c256's
 * memory/proposals precedent and its follow-on config-layer audit). */
function withAgent(url: string, agent?: string): string {
  if (!agent) return url;
  const sep = url.includes("?") ? "&" : "?";
  return `${url}${sep}agent=${encodeURIComponent(agent)}`;
}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...((init?.headers as Record<string, string>) ?? {}),
  };
  if (auth.mode === "bearer") headers.Authorization = `Bearer ${auth.token}`;
  const res = await fetch(`${backendUrl()}${path}`, {
    ...init,
    // Same-origin rather than `include`: this client never talks to a
    // third-party origin, and `include` would attach the session cookie to
    // one if it ever did.
    credentials: "same-origin",
    headers,
  });
  const text = await res.text();
  let parsed: unknown = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = text;
  }
  if (!res.ok) {
    if (res.status === 401) onUnauthorized?.();
    const msg =
      (parsed as { error?: string })?.error ?? `${res.status} ${res.statusText}`;
    throw new Error(msg);
  }
  return parsed as T;
}

/**
 * Called when the server says this client is no longer authenticated.
 *
 * Sessions expire on purpose (`[server] session_ttl_hours`, deliberately
 * short on a public deployment), and without this the expiry surfaces as
 * an unending drip of "401 Unauthorized" toasts from whatever happened to
 * poll next — with no way for the reader to learn that the fix is to sign
 * in again. Registered by App rather than acted on here, because what to
 * DO about it (return to the gate) is the app's decision, not the
 * transport's.
 */
let onUnauthorized: (() => void) | null = null;

export function setUnauthorizedHandler(handler: () => void): void {
  onUnauthorized = handler;
}

/**
 * Authenticated `fetch` returning the raw `Response`, for endpoints whose
 * body is not JSON (markdown export, synthesized speech). Shares one
 * credential path with `req` so a change of auth channel cannot leave a
 * caller behind.
 */
async function authFetch(path: string, init?: RequestInit): Promise<Response> {
  const headers: Record<string, string> = { ...((init?.headers as Record<string, string>) ?? {}) };
  if (auth.mode === "bearer") headers.Authorization = `Bearer ${auth.token}`;
  return fetch(`${backendUrl()}${path}`, { ...init, credentials: "same-origin", headers });
}

/**
 * Open an `EventSource` with whatever this host's auth channel is.
 *
 * On the web the stream is same-origin and the cookie rides along on its
 * own; `withCredentials` is set so a future proxy deployment on a
 * different path cannot silently drop it.
 *
 * On the desktop the token goes in the query string, because `EventSource`
 * cannot carry a header (see `Auth` above) — and `withCredentials` must
 * stay OFF there: the stream is cross-origin, and a credentialed
 * cross-origin request requires `Access-Control-Allow-Credentials` from
 * the server, which this one deliberately does not send. Setting it
 * unconditionally fails every desktop stream before it opens.
 */
function eventSource(path: string): EventSource {
  if (auth.mode === "cookie") {
    return new EventSource(path, { withCredentials: true });
  }
  const join = path.includes("?") ? "&" : "?";
  return new EventSource(
    `${auth.base}${path}${join}token=${encodeURIComponent(auth.token)}`,
  );
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

export function finopsStatus(agent?: string): Promise<FinopsStatus> {
  return req(withAgent("/finops", agent));
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

export function forgetMemory(
  id: string,
  scope: MemoryScope,
  agent?: string,
): Promise<{ forgotten: string; bytes: number }> {
  const params = new URLSearchParams({ scope });
  if (agent) params.set("agent", agent);
  return req(`/memory/${encodeURIComponent(id)}?${params}`, { method: "DELETE" });
}

export function amendMemory(
  id: string,
  scope: MemoryScope,
  text: string,
  agent?: string,
): Promise<{ amended: string }> {
  return req(`/memory/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: JSON.stringify({ text, scope, agent }),
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
  network_allowed: boolean;
  network_denied: boolean;
  network_allow: string[] | null;
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
  layer: "seed" | "shared" | "workspace" | "surface" | "bot" | "chat" | "agent";
  source?: string | null;
  digest: string;
  bytes: number;
}

export function getPromptLayer(scope: "user" | "workspace"): Promise<{ scope: string; path: string; layer: PromptLayerContent }> {
  return req(`/config/prompts?scope=${scope}`);
}

export function putPromptBlock(scope: "user" | "workspace", block: PromptBlock, text: string | null): Promise<unknown> {
  return req("/config/prompts", { method: "PUT", body: JSON.stringify({ scope, block, text }) });
}

export function getPromptEffective(agent?: string): Promise<{ text: string; fingerprint: string; estimated_tokens: number; surface: string; layers: PromptLayerDescriptor[] }> {
  return req(withAgent("/config/prompts/effective", agent));
}

export interface Agent {
  id: string;
  revision: number;
  lifecycle: "active" | "paused" | "archived";
  name: string;
  character: "orb" | "leaf" | "sun" | "wave" | "spark";
  personality: string;
  behaviour: string;
  responsibilities: string;
  instructions: string;
  animation: "subtle" | "expressive" | "off";
  voice: string;
}

export function listAgents(scope?: "user" | "workspace"): Promise<{ agents: Agent[] }> {
  return req(scope ? `/config/agents?scope=${scope}` : "/agents");
}

export function saveAgents(agents: Agent[], scope: "user" | "workspace" = "workspace"): Promise<{ saved: boolean; agents: Agent[] }> {
  return req("/config/agents", { method: "PUT", body: JSON.stringify({ agents, scope }) });
}

export function openAgent(id: string): Promise<{session_id: string; cwd: string; agent: Agent}> {
  return req(`/agents/${encodeURIComponent(id)}/open`, {method: "POST", body: "{}"});
}

export interface AgentTemplate {
  template_id: string;
  domain: string;
  name: string;
  description: string;
  character: "orb" | "leaf" | "sun" | "wave" | "spark";
  personality: string;
  behaviour: string;
  responsibilities: string;
  instructions: string;
  animation: "subtle" | "expressive" | "off";
  voice: string;
}

export function listAgentTemplates(): Promise<{ templates: AgentTemplate[] }> {
  return req("/agents/templates");
}

export function instantiateAgentTemplate(
  template_id: string,
  agent_id: string,
  name?: string,
  scope: "user" | "workspace" = "workspace",
): Promise<{ created: boolean; agent: Agent }> {
  return req("/agents/instantiate", {
    method: "POST",
    body: JSON.stringify({ template_id, agent_id, name, scope }),
  });
}

export function getHooks(agent?: string): Promise<{ hooks: HookConfig[] }> {
  return req(withAgent("/config/hooks", agent));
}

export function putHooks(hooks: HookConfig[], agent?: string): Promise<{ saved: boolean; count: number }> {
  return req("/config/hooks", { method: "PUT", body: JSON.stringify({ hooks, agent }) });
}

export function listMemory(agent?: string): Promise<{ notes: NoteBlock[] }> {
  return req(agent ? `/memory?agent=${encodeURIComponent(agent)}` : "/memory");
}

export function appendMemory(
  scope: MemoryScope,
  text: string,
  kind = "fact",
  tag = "",
  agent?: string,
): Promise<NoteBlock> {
  return req("/memory", {
    method: "POST",
    body: JSON.stringify({ scope, text, kind, tag, agent }),
  });
}

export function listProposals(agent?: string): Promise<{ proposals: SkillProposal[] }> {
  return req(agent ? `/skills/proposals?agent=${encodeURIComponent(agent)}` : "/skills/proposals");
}

export function promoteProposal(id: string, agent?: string): Promise<{ promoted: string }> {
  const suffix = agent ? `?agent=${encodeURIComponent(agent)}` : "";
  return req(`/skills/proposals/${encodeURIComponent(id)}/promote${suffix}`, {
    method: "POST",
    body: "{}",
  });
}

export function rejectProposal(id: string, agent?: string): Promise<{ rejected: string }> {
  const suffix = agent ? `?agent=${encodeURIComponent(agent)}` : "";
  return req(`/skills/proposals/${encodeURIComponent(id)}/reject${suffix}`, {
    method: "POST",
    body: "{}",
  });
}

// ---- sessions ---------------------------------------------------------------

export function listSessions(): Promise<{ sessions: SessionSummary[] }> {
  return req("/sessions");
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
  entries?: TranscriptEntryMeta[];
}> {
  return req(`/sessions/${id}/transcript`);
}

export function sandboxExecutions(id: string): Promise<{ session_id: string; events: Array<Record<string, unknown>> }> {
  return req(`/sessions/${encodeURIComponent(id)}/sandbox/executions`);
}

export function presentation(id: string): Promise<OutputTimeline> {
  return req(`/sessions/${encodeURIComponent(id)}/presentation`);
}

export function result(id: string, resultId: string): Promise<OutputTimeline["items"][number]> {
  return req(`/sessions/${encodeURIComponent(id)}/results/${encodeURIComponent(resultId)}`);
}

export function openPresentationStream(
  id: string,
  onEvent: (event: PresentationStreamEvent) => void,
  onError?: () => void,
): EventSource {
  const es = eventSource(`/sessions/${encodeURIComponent(id)}/presentation/events`);
  es.onmessage = (message) => {
    try {
      const frame = JSON.parse(message.data);
      if (frame?.snapshot?.schema_version !== 2 || frame.snapshot.session_id !== id || !Array.isArray(frame.snapshot.items) || !Array.isArray(frame.snapshot.diagnostics)) {
        throw new Error("Invalid presentation frame");
      }
      onEvent(frame as PresentationStreamEvent);
    } catch {
      onError?.();
    }
  };
  es.onerror = () => onError?.();
  return es;
}

/** Markdown export (shared renderer with the TUI); text, not JSON. */
export async function transcriptMarkdown(id: string): Promise<string> {
  const res = await authFetch(`/sessions/${encodeURIComponent(id)}/transcript.md`);
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  return res.text();
}

/** Interactive HTML Outcome Canvas export. */
export async function transcriptHtml(id: string): Promise<string> {
  const res = await authFetch(`/sessions/${encodeURIComponent(id)}/transcript.md?format=html`);
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  return res.text();
}

/** Render content to interactive living canvas HTML preview. */
export async function previewCanvas(content: string, title?: string): Promise<string> {
  const res = await authFetch("/canvas/preview", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ content, title }),
  });
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  return res.text();
}

export function runPrompt(
  id: string,
  prompt: string,
  goal?: { objective: string; criteria: string[] },
  attachments?: { mime: string; data: string }[],
  requestId?: string,
  routing?: RoutingEnvelope,
): Promise<void> {
  return req(`/sessions/${id}/run`, {
    method: "POST",
    body: JSON.stringify({
      prompt,
      request_id: requestId ?? (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
        ? crypto.randomUUID()
        : `${Date.now()}-${Math.random().toString(36).slice(2)}`),
      routing,
      goal: goal?.objective,
      criteria: goal?.criteria,
      attachments: attachments ?? [],
    }),
  });
}

export type RoutingEnvelope = {
  message_id?: string;
  conversation_id?: string;
  target_work_id?: string;
  target_result_id?: string;
  relation?: "independent" | "follow_up" | "correction" | "status" | "cancel" | "schedule";
  outcome_revision?: number;
  provenance?: string;
};

// ---- workers (attach / steer / stop) ---------------------------------------

export interface ActiveWorker {
  id: string;
  label: string;
  agent_id?: string | null;
  agent_revision?: number | null;
  elapsed_secs: number;
  parent_session_id: string;
}

export function listWorkers(id: string): Promise<{ workers: ActiveWorker[] }> {
  return req(`/sessions/${id}/workers`);
}

export function steerWorker(id: string, child: string, text: string): Promise<void> {
  return req(`/sessions/${id}/workers/${encodeURIComponent(child)}/steer`, {
    method: "POST",
    body: JSON.stringify({ text }),
  });
}

export function stopWorker(
  id: string,
  child: string,
): Promise<void> {
  return req(`/sessions/${id}/workers/${encodeURIComponent(child)}/stop`, { method: "POST" });
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

export type InterventionReceipt = {
  request_id: string;
  decision?: string;
  state?: string;
  reason?: string;
};

export function steer(id: string, text: string, attachments?: { mime: string; data: string }[], requestId?: string, routing?: RoutingEnvelope): Promise<InterventionReceipt> {
  return req(`/sessions/${id}/steering`, {
    method: "POST",
    body: JSON.stringify({
      text,
      attachments: attachments ?? [],
      request_id: requestId ?? (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
        ? crypto.randomUUID()
        : `${Date.now()}-${Math.random().toString(36).slice(2)}`),
      routing,
    }),
  });
}

export function cancelRun(id: string): Promise<void> {
  return req(`/sessions/${id}/cancel`, { method: "POST" });
}

export function pauseRun(id: string): Promise<void> {
  return req(`/sessions/${id}/pause`, { method: "POST" });
}

export function resumeRun(id: string): Promise<void> {
  return req(`/sessions/${id}/resume`, { method: "POST" });
}

export function controlState(id: string): Promise<{ running: boolean; paused: boolean; revision: number }> {
  return req(`/sessions/${id}/control-state`);
}

export function planChange(
  id: string,
  text: string,
  source = "human",
  targetRevision?: number,
): Promise<{ request_id: string; decision: string; revision: number; reason: string }> {
  return req(`/sessions/${id}/plan-change`, {
    method: "POST",
    body: JSON.stringify({ text, source, target_revision: targetRevision }),
  });
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

export function setPermissionMode(mode: string, agent?: string): Promise<void> {
  return req("/config/mode", { method: "POST", body: JSON.stringify({ mode, agent }) });
}

export function getConfig(agent?: string): Promise<ConfigSnapshot> {
  return req(withAgent("/config", agent));
}

/**
 * Subscribe to server-side config/credential changes (docs/design/44-shared-config.md,
 * "Liveness") so a write from another surface (CLI `vak setup`, another
 * client) is reflected here without a manual refresh or app restart.
 *
 * Reuses the admin event hub's existing broadcast stream rather than a
 * dedicated endpoint — every `emit_config_changed(...)` call server-side
 * already fires a `ConfigChanged` event on it. `onChange` is called on
 * every such event; callers decide what to re-fetch.
 */
export function openConfigEvents(onChange: () => void): EventSource {
  const es = eventSource("/admin/api/events");
  es.onmessage = (message) => {
    try {
      const event = JSON.parse(message.data);
      if (event?.type === "ConfigChanged") onChange();
    } catch {
      // ignore malformed/keep-alive frames
    }
  };
  return es;
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
  /// Grants `[plugins] network_allow` for the selected layer. An empty
  /// array clears the layer's grant (deny-by-default); absent leaves the
  /// layer untouched. Non-empty grants are refused for untrusted projects.
  plugins_network_allow?: string[];
}

export function patchConfig(patch: ConfigPatch, agent?: string): Promise<void> {
  return req("/config", { method: "PATCH", body: JSON.stringify({ ...patch, agent }) });
}

export function recordOutcomeReview(
  sessionId: string,
  verdict: "accepted" | "needs_work" | "rejected",
  turn?: number,
  note?: string,
): Promise<{ recorded: boolean }> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/outcome-review`, {
    method: "POST",
    body: JSON.stringify({ verdict, turn, note }),
  });
}

export function patchEvidencePolicy(seconds: number, scope: "user" | "workspace"): Promise<{ saved: boolean; seconds: number }> {
  return req("/config/intent/evidence", { method: "POST", body: JSON.stringify({ seconds, scope }) });
}





// ---- MCP server management ----------------------------------------------------

export interface McpServerDef {
  command: string;
  args: string[];
  env: Record<string, string>;
  network: boolean;
}

export function getMcpServers(agent?: string): Promise<{ servers: Record<string, McpServerDef> }> {
  return req(withAgent("/config/mcp", agent));
}

/** Replaces the whole running table and persists the project config. */
export function putMcpServers(servers: Record<string, McpServerDef>, agent?: string): Promise<{ saved: boolean; count: number }> {
  return req("/config/mcp", { method: "PUT", body: JSON.stringify({ servers, agent }) });
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

export function listCheckpoints(id: string, agent?: string): Promise<{
  checkpoints: { seq: number; label: string; created_at: string; files: number }[];
}> {
  return req(withAgent(`/sessions/${id}/checkpoints`, agent));
}

export function restoreCheckpoint(
  id: string,
  seq: number,
  agent?: string,
): Promise<{ restored: number; deleted: number; seq: number }> {
  return req(withAgent(`/sessions/${id}/checkpoints/${seq}/restore`, agent), { method: "POST" });
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

export function listPlugins(scope?: "user" | "workspace", agent?: string): Promise<{ plugins: InstalledPlugin[] }> {
  return req(withAgent(`/plugins${scope ? `?scope=${scope}` : ""}`, agent));
}

export function listPluginSources(scope?: "user" | "workspace", agent?: string): Promise<{ sources: MarketplaceSource[] }> {
  return req(withAgent(`/plugins/sources${scope ? `?scope=${scope}` : ""}`, agent));
}

export function listPluginCatalog(query = "", scope?: "user" | "workspace", agent?: string): Promise<{ entries: MarketplaceEntry[]; errors: { source_id?: string; error: string }[] }> {
  const params = new URLSearchParams();
  if (query.trim()) params.set("q", query.trim());
  if (scope) params.set("scope", scope);
  const suffix = params.toString() ? `?${params.toString()}` : "";
  return req(withAgent(`/plugins/catalog${suffix}`, agent));
}

export function registerPluginSource(path: string, label: string, signature?: { key_id: string; public_key: string; signature: string }, agent?: string): Promise<MarketplaceSource> {
  return req("/plugins/sources", { method: "POST", body: JSON.stringify({ path, label, trust: "manual-review", agent, ...(signature ?? {}) }) });
}

export function pluginKeyAction(keyId: string, action: "revoke" | "restore", scope: "user" | "workspace", agent?: string): Promise<unknown> {
  return req(withAgent(`/plugins/keys/${encodeURIComponent(keyId)}/${action}?scope=${scope}`, agent), { method: "POST", body: "{}" });
}

export function pluginSourceAction(id: string, action: "enable" | "disable", scope: "user" | "workspace", agent?: string): Promise<MarketplaceSource> {
  return req(withAgent(`/plugins/sources/${encodeURIComponent(id)}/${action}?scope=${scope}`, agent), { method: "POST", body: "{}" });
}

export function installPlugin(path: string, scope: "workspace" | "user", agent?: string): Promise<InstalledPlugin> {
  return req("/plugins/install", { method: "POST", body: JSON.stringify({ path, scope, agent }) });
}

export function updatePlugin(path: string, scope: "workspace" | "user", agent?: string): Promise<InstalledPlugin> {
  return req("/plugins/update", { method: "POST", body: JSON.stringify({ path, scope, agent }) });
}

export function pluginAction(name: string, action: "enable" | "disable" | "rollback", scope: "user" | "workspace", agent?: string): Promise<InstalledPlugin> {
  return req(withAgent(`/plugins/${encodeURIComponent(name)}/${action}?scope=${scope}`, agent), { method: "POST", body: "{}" });
}

export function removePlugin(name: string, scope: "user" | "workspace", agent?: string): Promise<InstalledPlugin> {
  return req(withAgent(`/plugins/${encodeURIComponent(name)}?scope=${scope}`, agent), { method: "DELETE" });
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

/** Authenticated raw bytes for browser-native artifact viewers/downloads. */
export async function readFileRaw(path: string): Promise<string> {
  const response = await authFetch(`/fs/file/raw?path=${encodeURIComponent(path)}`);
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return URL.createObjectURL(await response.blob());
}

export function writeFile(path: string, content: string): Promise<unknown> {
  return req("/fs/file", { method: "PUT", body: JSON.stringify({ path, content }) });
}

export type SandboxCandidate = { candidate_id: string; source_root: string; destination_root: string; files: Array<{ path: string; candidate_hash: string; base_hash?: string; bytes: number }> };
export type SandboxCandidateRecord = { record_id: string; session_id: string; turn_id: string; result_id: string; execution_id: string; environment_id: string; candidate_digest: string; candidate: SandboxCandidate; verified: boolean; updated_at: string };
export type SandboxPromotionRecord = { record_id: string; session_id: string; result_id: string; candidate_digest: string; candidate_id: string; receipt: { verification?: Array<{ path: string; status: string; evidence: string }> }; updated_at: string };
export type SandboxRecord =
  | { kind: "Candidate"; record: SandboxCandidateRecord }
  | { kind: "Promotion"; record: SandboxPromotionRecord }
  | { kind: "Environment"; record: unknown };

export function readSandboxCandidateFile(sessionId: string, candidateId: string, path: string): Promise<FileResponse> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/sandbox/candidates/${encodeURIComponent(candidateId)}/files?path=${encodeURIComponent(path)}`);
}

export async function readSandboxCandidateFileRaw(sessionId: string, candidateId: string, path: string): Promise<string> {
  const response = await authFetch(`/sessions/${encodeURIComponent(sessionId)}/sandbox/candidates/${encodeURIComponent(candidateId)}/files/raw?path=${encodeURIComponent(path)}`);
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return URL.createObjectURL(await response.blob());
}

export function commentOnSandboxCandidate(sessionId: string, candidateId: string, text: string, anchor?: { path?: string; lineStart?: number; lineEnd?: number }): Promise<InterventionReceipt> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/sandbox/candidates/${encodeURIComponent(candidateId)}/comments`, {
    method: "POST",
    body: JSON.stringify({
      text,
      path: anchor?.path,
      line_start: anchor?.lineStart,
      line_end: anchor?.lineEnd,
      request_id: typeof crypto !== "undefined" && typeof crypto.randomUUID === "function" ? crypto.randomUUID() : `${Date.now()}-${Math.random().toString(36).slice(2)}`,
    }),
  });
}

export function listSessionSandboxRecords(sessionId: string): Promise<{ records: SandboxRecord[] }> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/sandbox/records`);
}

export async function exportSandboxCandidate(sessionId: string, executionId: string, source: string, destination = "."):
  Promise<SandboxCandidateRecord> {
  const response = await req<{ kind: "Candidate"; record: SandboxCandidateRecord }>(`/sessions/${encodeURIComponent(sessionId)}/sandbox/candidates`, {
    method: "POST",
    body: JSON.stringify({ execution_id: executionId, source, destination }),
  });
  return response.record;
}

export async function promoteSandboxCandidate(sessionId: string, candidateId: string, files: string[]): Promise<SandboxPromotionRecord> {
  const response = await req<{ kind: "Promotion"; record: SandboxPromotionRecord }>(`/sessions/${encodeURIComponent(sessionId)}/sandbox/promote`, { method: "POST", body: JSON.stringify({ candidate_id: candidateId, files }) });
  return response.record;
}

/**
 * Stop listing a workspace. Sessions, memory, checkpoints, and the
 * project's own settings all survive — re-opening the folder restores it
 * exactly, which is what makes this safe to offer as one click.
 */
export function forgetWorkspace(path: string): Promise<{ forgotten: string }> {
  return req("/workspaces/forget", {
    method: "POST",
    body: JSON.stringify({ path }),
  });
}

/**
 * Directory listing for the workspace picker (docs/design/48-web-client.md
 * §5). Folder names only — never file contents — and rooted server-side at
 * `[server] workspace_roots`, so this cannot be walked into somewhere the
 * operator never authorized.
 *
 * Lives here rather than on the host because it is a plain authenticated
 * route: the desktop's own embedded server answers it identically, which
 * is what lets the picker work as a fallback anywhere a native dialog is
 * unavailable.
 */
export function listDirectory(path?: string): Promise<import("./types").DirListing> {
  const query = path ? `?path=${encodeURIComponent(path)}` : "";
  return req(`/fs/dirs${query}`);
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
  const res = await authFetch("/voice/speak", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
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
  onEvent: (ev: AgentEvent, lastEventId?: string) => void,
  onError?: () => void,
  /** The server lost our place in the replay ring: whatever is on screen
   *  may be missing events, and only a re-read of the durable transcript
   *  can be trusted (docs/design/48-web-client.md §4.4). */
  onResync?: () => void,
  /** Fires when the stream is actually carrying events again. */
  onOpen?: () => void,
  /** Resume cursor for a client-created replacement EventSource. */
  resumeFrom?: string,
): EventSource {
  const path = `/sessions/${encodeURIComponent(id)}/events${resumeFrom ? `?last_event_id=${encodeURIComponent(resumeFrom)}` : ""}`;
  const es = eventSource(path);
  es.onopen = () => onOpen?.();
  // A named event, so it cannot be confused with an agent event that
  // happens to carry a similar shape.
  es.addEventListener("resync", () => onResync?.());
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
        onEvent(parsed, m.lastEventId || undefined);
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
  onError?: () => void,
): EventSource {
  const es = eventSource(`/sessions/${encodeURIComponent(id)}/side/events`);
  es.onmessage = (m) => {
    try {
      onEvent(JSON.parse(m.data) as AgentEvent);
    } catch {
      // ignore keep-alive frames
    }
  };
  es.onerror = () => onError?.();
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
  agent_id?: string | null;
  agent_revision?: number | null;
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

export function retryTaskDelivery(id: string): Promise<{ replayed: number; failed: number }> {
  return req(`/tasks/${encodeURIComponent(id)}/retry-delivery`, { method: "POST", body: "{}" });
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
  result_id?: string | null;
  origin_state?: "available" | "unavailable";
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
  history?: {
    turn_index: number;
    previous_act?: string | null;
    commitment_open: boolean;
  };
}

/** Resolve a prompt without running it. Free tiers only: this costs nothing
 * and dispatches nothing, which is what makes it safe to call while typing. */
export function explainIntent(
  prompt: string,
  sessionId?: string | null,
  signal?: AbortSignal,
): Promise<IntentExplain> {
  const params = new URLSearchParams({ prompt, surface: "desktop" });
  if (sessionId) params.set("session_id", sessionId);
  return req<IntentExplain>(`/intent/explain?${params}`, { signal });
}

// ---- commitments (docs/design/47-commitment-kernel.md) -----------------------
//
// The portfolio previously lived only in the admin console
// (crates/vak-admin-ui/src/Commitments.tsx) — a durable obligation the
// runtime will verify is workspace information, and the workspace client
// had no way to see or close one. This is the same read model, scoped to
// the active workspace client-side (the server itself is not
// workspace-scoped: `spec.cwd` names the workspace a commitment belongs
// to, exactly like a session's own `cwd`).

export type Verdict =
  | "fulfilled" | "partial" | "failed"
  | "abandoned" | "superseded" | "expired" | "unknown";
export type CommitmentPhase =
  | "proposed" | "active" | "suspended" | "blocked" | "satisfying" | "closed";

export interface CriterionState {
  criterion_id: string;
  statement: string;
  required: boolean;
  result?:
    | { kind: "passed"; evidence: string }
    | { kind: "failed"; reason: string }
    | { kind: "unknown"; reason: string }
    | null;
  strength?: Satisfaction | null;
  evaluated_at?: string | null;
}

export interface CommitmentEpisode {
  episode_id: string;
  session_id: string;
  started_at: string;
  ended_at?: string | null;
  advancement?: { kind: "advanced" | "learned" | "blocked" | "stalled" } | null;
  spend_usd: number;
}

export interface Commitment {
  commitment_id: string;
  opened_at: string;
  spec: {
    objective: string;
    reading: IntentReading;
    min_satisfaction: Satisfaction;
    cwd: string;
    economics: {
      lifetime_budget_usd?: number | null;
      expires_at?: string | null;
      review_every_hours?: number | null;
      stall_limit: number;
    };
  };
  phase: CommitmentPhase;
  criteria: CriterionState[];
  episodes: CommitmentEpisode[];
  suspension?: { kind: string } | null;
  blocker?: string | null;
  closure?: {
    verdict: Verdict;
    strength: Satisfaction;
    closed_at: string;
    note: string;
  } | null;
  superseded_by?: string | null;
  spend_usd: number;
  consecutive_stalls: number;
  drift: string[];
  updated_at: string;
}

export interface CommitmentPriority {
  commitment_id: string;
  score: number;
  components: [string, number][];
  withheld?: string | null;
}

export function listCommitments(all = false, agent?: string): Promise<{ commitments: Commitment[]; priorities: CommitmentPriority[] }> {
  return req(withAgent(`/commitments?all=${all}`, agent));
}

export function closeCommitment(id: string, verdict: Verdict, note = "", agent?: string): Promise<unknown> {
  return req(`/commitments/${encodeURIComponent(id)}/close`, {
    method: "POST",
    body: JSON.stringify({ verdict, note, agent }),
  });
}

export interface PresentationLibraryResponse {
  definitions: Array<{ spec: { id: string; revision: number; accepts?: string[] }; origin: { owner: string; plugin_id?: string | null }; enabled: boolean }>;
  activations: Array<{ spec_id: string; revision: number; scope: "user" | "workspace"; owner: string }>;
}

export function listPresentations(): Promise<PresentationLibraryResponse> {
  return req("/presentations");
}

export function getPresentationSpec(id: string, revision: number): Promise<unknown> {
  return req(`/presentations/specs/${encodeURIComponent(id)}/${revision}`);
}

export function exportPresentations(): Promise<unknown> {
  return req("/presentations/export");
}

export function importPresentations(pack: unknown): Promise<unknown> {
  return req("/presentations/import", { method: "POST", body: JSON.stringify(pack) });
}

export function registerPresentations(records: unknown[]): Promise<{ registered: number }> {
  return req("/presentations", {
    method: "POST",
    body: JSON.stringify({ records }),
  });
}

export function revokePresentationPlugin(pluginId: string): Promise<{ removed: number }> {
  return req(`/presentations/plugins/${encodeURIComponent(pluginId)}`, { method: "DELETE" });
}

export function activatePresentation(id: string, revision: number, scope: "user" | "workspace", owner: string): Promise<unknown> {
  return req(`/presentations/${encodeURIComponent(id)}/${revision}/activate`, {
    method: "POST",
    body: JSON.stringify({ scope, owner }),
  });
}

export function activateAllPresentations(scope: "user" | "workspace", owner: string): Promise<{ activated: number }> {
  return req("/presentations/activate-all", {
    method: "POST",
    body: JSON.stringify({ scope, owner }),
  });
}

export function deactivatePresentation(id: string, scope: "user" | "workspace", owner: string): Promise<unknown> {
  return req(`/presentations/${encodeURIComponent(id)}/deactivate`, {
    method: "POST",
    body: JSON.stringify({ scope, owner }),
  });
}

export function deactivateAllPresentations(scope: "user" | "workspace", owner: string): Promise<{ deactivated: number }> {
  return req("/presentations/deactivate-all", {
    method: "POST",
    body: JSON.stringify({ scope, owner }),
  });
}

export function resetPresentation(id: string, scope: "user" | "workspace", owner: string): Promise<unknown> {
  return req(`/presentations/${encodeURIComponent(id)}/reset`, {
    method: "POST",
    body: JSON.stringify({ scope, owner }),
  });
}

export function proposePresentationRevision(request: unknown, proposed: unknown, origin: unknown): Promise<unknown> {
  return req("/presentations/revisions", {
    method: "POST",
    body: JSON.stringify({ request, proposed, origin }),
  });
}

export function proposeSessionPresentationRevision(sessionId: string, request: unknown, proposed: unknown, origin: unknown): Promise<unknown> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/presentation/proposals`, {
    method: "POST",
    body: JSON.stringify({ request, proposed, origin }),
  });
}

// `presentationId` is the `Presentation` ledger entry's own id
// (docs/design/68-context-engine.md §10: "the user dismissed this card" is
// an event about a ledger fact) — the server requires it and 400s without
// one. Read it from `OutputItem.provenance.presentation_id`, never from
// `provenance.entry_id` (that names the containing message, not the card).
export function submitPresentationFeedback(sessionId: string, choice: string, presentationId: string, feedback?: string): Promise<void> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/presentation/feedback`, {
    method: "POST",
    body: JSON.stringify({ choice, feedback, presentation_id: presentationId }),
  });
}

export function selectPresentation(
  sessionId: string,
  specId: string,
  revision: number,
  lifetime: "use_once" | "remember",
  presentationId: string,
  scope?: "user" | "workspace",
  owner?: string,
): Promise<unknown> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/presentation/select`, {
    method: "POST",
    body: JSON.stringify({ spec_id: specId, revision, lifetime, scope, owner, presentation_id: presentationId }),
  });
}

export function selectPresentationForSemantic(sessionId: string, semanticType: string, presentationId: string): Promise<unknown> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/presentation/select`, {
    method: "POST",
    body: JSON.stringify({ spec_id: "", revision: 0, semantic_type: semanticType, lifetime: "use_once", presentation_id: presentationId }),
  });
}
