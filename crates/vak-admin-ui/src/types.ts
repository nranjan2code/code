export interface SessionListItem {
  session_id: string;
  project_hash: string;
  entry_count: number;
  first_ts: string;
  last_ts: string;
}

export interface TranscriptEntry {
  entry_id: string;
  ts: string;
  kind: string;
  role: string | null;
  tool_name: string | null;
  is_error: boolean;
  content: string;
}

export interface SearchHit {
  entry_id: string;
  session_id: string;
  project_hash: string;
  ts: string;
  kind: string;
  role: string | null;
  provider: string | null;
  model: string | null;
  tool_name: string | null;
  score: number;
  snippet: string;
}

export interface SecurityEvent {
  ts: string;
  kind: string;
  label: string;
  detail: string;
  ip: string | null;
}

export interface HealthInfo {
  status: string;
  provider: string;
  model: string;
  permission_mode: string;
  sandbox: string;
  context_window: number;
  cwd: string;
  warnings: string[];
}

export interface ConfigInfo {
  provider: string;
  model: string;
  provider_source?: string;
  model_source?: string;
  route_revision?: string;
  max_turns: number;
  permission_mode: string;
  theme: string;
}

export interface RouteInfo {
  provider: string;
  model: string;
  provider_source?: string;
  model_source?: string;
  source?: string;
  revision: string;
}

export interface GatewayBinding {
  target: string;
  session_id: string | null;
  workspace: string;
  configured_workspace: string | null;
  override: { provider: string; model: string } | null;
  effective_route: RouteInfo;
  session_contract: {
    provider: string;
    model: string;
    workspace: string;
    app_version: string;
  } | null;
  stale: boolean;
  stale_reasons: string[];
}

export interface GatewayStatus {
  enabled: boolean;
  workspace: string;
  default_route: RouteInfo;
  bindings: GatewayBinding[];
  chat_allowlist: string[];
  chat_allowlist_open: boolean;
}

export type AllowlistStatus = "pending" | "allowed" | "denied";

export interface AllowlistRoute {
  provider: string;
  model: string;
}

export interface AllowlistEntry {
  key: string;
  status: AllowlistStatus;
  workspace: string | null;
  route: AllowlistRoute | null;
  added_at: string;
  added_by: string;
  first_seen_text: string | null;
}

export type SystemEvent =
  | { type: "Agent"; data: { summary: string; detail?: string } }
  | { type: "SessionCreated"; data: { session_id: string; project_hash: string } }
  | { type: "SessionEntryAppended"; data: { session_id: string; entry_id: string; kind: string } }
  | { type: "ConfigChanged"; data: { label: string; detail: string } }
  | { type: "GatewayInbound"; data: { surface: string; who: string; preview: string } }
  | { type: "ApprovalRequested"; data: { id: string; session_id: string; tool: string; reason: string } }
  | { type: "ApprovalGranted"; data: { id: string; tool: string } }
  | { type: "ApprovalDenied"; data: { id: string; tool: string } }
  | { type: "SecurityEvent"; data: { kind: string; label: string } }
  | { type: "ProviderError"; data: { provider: string; model: string; error: string } }
  | { type: "RateLimit"; data: { provider: string; retry_after_secs: number | null } }
  | { type: "Heartbeat" }
  | { type: "Lagged"; data: { missed: number } };

export interface PendingApproval {
  session_id: string;
  request_id: string;
  tool: string;
  args_json: string;
  reason: string;
  requested_at: string;
}

export interface InboxEntry {
  id: string;
  ts: string;
  kind: string;
  title: string;
  body: string;
  session_id?: string | null;
}

export interface BestOfNRun {
  session_id: string;
  repo: string;
  branch: string;
}

export interface RebuildStats {
  ok: boolean;
  files_scanned?: number;
  entries_indexed?: number;
  fts_rows?: number;
  error?: string;
}

export interface ProviderSummary {
  name: string;
  env_var: string;
  requires_key: boolean;
  configured: boolean;
}

export interface ProviderListResponse {
  current: string;
  current_model: string;
  current_configured: boolean;
  providers: ProviderSummary[];
}

export interface DiscoveredModelsResponse {
  provider: string;
  models: string[];
  error?: string;
}

export interface FinOpsStatus {
  total_spend_usd?: number;
  total_cost?: number;
  total_input_tokens?: number;
  total_output_tokens?: number;
  budget_cap_usd?: number | null;
  budget_admission?: string;
  budget_exhausted?: boolean;
  alert_rows?: Array<{ ts: string; level: string; message: string }>;
  currency?: string;
}

export interface OpsStatus {
  gateway: { state: string };
  telegram: { state: string };
  gateway_healthy: boolean;
}

export interface OpsDiagnostics {
  health: HealthInfo;
  services: OpsStatus;
  gateway: {
    enabled: boolean;
    bindings: Array<{ target: string; session_id: string }>;
    approvals: { mode: string; approver: string | null; pending: number };
  };
  flows: Array<{ name: string; runs: number }>;
}

export interface McpServerConfig {
  command: string;
  args?: string[];
  env?: Record<string, string>;
  network?: boolean;
}

export interface McpListResponse {
  servers: Record<string, McpServerConfig>;
}

export interface HookConfig {
  event: string;
  matcher?: string | null;
  command: string;
  timeout_ms: number;
  enabled: boolean;
}

export interface SkillItem {
  name: string;
  path?: string;
  description?: string;
}

export interface SkillProposal {
  id: string;
  name: string;
  description: string;
}

export interface TaskItem {
  id: string;
  name: string;
  prompt?: string;
  interval_secs?: number;
  enabled: boolean;
  schedule?: string | null;
  script?: string | null;
  model_pin?: string | null;
  created_at?: string;
  last_run_at?: string | null;
  last_session_id?: string | null;
  last_summary?: string | null;
}

export interface MemoryItem {
  id: string;
  ts: string;
  scope: "workspace" | "profile" | string;
  kind: string;
  tag?: string;
  text: string;
  session_id?: string;
}

export interface WorkReceipt {
  step?: number;
  provider?: string;
  model?: string;
  input_tokens?: number;
  output_tokens?: number;
  cost_usd?: number;
  latency_ms?: number;
  ts?: string;
}

export interface SessionDiff {
  diff: string;
}

export interface SessionCheckpoint {
  seq: number;
  ts: string;
  commit_hash?: string;
  message?: string;
}
