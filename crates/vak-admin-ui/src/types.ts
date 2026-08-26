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
  max_tokens: number;
  max_turns: number;
  permission_mode: string;
  theme: string;
  subagents: boolean;
  context_window: number;
  route: {
    objective: string;
    fallback_models: string[];
    max_fallbacks: number;
    quality_hints: string[];
  };
  integrations: { mcp_servers: string[]; hooks: number; skills: string[] };
  paths: { project_config: string; global_config?: string | null; sessions_home: string; cwd: string };
  warnings: string[];
}

export interface GatewayStatus {
  enabled: boolean;
  bindings: string[];
  chat_allowlist: string[];
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

export interface ProviderInfo {
  name: string;
  env_var?: string | null;
  requires_key: boolean;
  configured: boolean;
}

export interface ProvidersResponse {
  current: string;
  current_model: string;
  current_configured: boolean;
  providers: ProviderInfo[];
}

export interface McpServerDef {
  command: string;
  args: string[];
  env: Record<string, string>;
  network: boolean;
}

export interface HookConfig {
  event: string;
  matcher?: string | null;
  command: string;
  timeout_ms?: number | null;
  enabled?: boolean;
}

export interface DiscoveredSkill {
  name: string;
  description: string;
  source?: string;
  scope?: string;
}

export interface SkillProposal {
  id: string;
  name: string;
  description: string;
}

export type MemoryScope = "workspace" | "profile";

export interface NoteBlock {
  id: string;
  ts: string;
  kind: string;
  tag: string;
  session_id: string;
  text: string;
  scope?: MemoryScope;
}

export interface TaskDef {
  id: string;
  name: string;
  prompt: string;
  enabled: boolean;
  interval_secs: number;
  schedule?: string | null;
  script?: string | null;
  model_pin?: string | null;
  last_run?: string | null;
  next_run?: string | null;
}

export interface TaskDraft {
  name: string;
  prompt: string;
  interval_secs: number;
  schedule?: string | null;
  script?: string | null;
  model_pin?: string | null;
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

export interface FinopsStatus {
  day_usd: number;
  run_cap_usd?: number | null;
  day_cap_usd?: number | null;
  unknown_rows: number;
  total_rows: number;
  by_provider: { name: string; usd: number; calls: number }[];
  by_model: { name: string; usd: number; calls: number }[];
}

export interface OpsStatusShape {
  gateway: { state: string };
  telegram: { state: string };
  gateway_healthy: boolean;
}

export interface OpsDiagnostics {
  health: {
    status: string;
    provider: string;
    model: string;
    sandbox: string;
    permission_mode: string;
    warnings: string[];
  };
  services: OpsStatusShape;
  gateway: {
    enabled: boolean;
    bindings: { target: string; session_id: string }[];
    approvals: { mode: string; approver?: string | null; pending: number };
  };
  flows: { name: string; runs: number }[];
}

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

export interface BackupManifest {
  version: number;
  timestamp: string;
  file_count: number;
  total_bytes: number;
}

export interface ImportReportShape {
  copied: number;
  renamed: number;
  skipped: number;
}
