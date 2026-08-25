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
  max_turns: number;
  permission_mode: string;
  theme: string;
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

export interface RebuildStats {
  ok: boolean;
  files_scanned?: number;
  entries_indexed?: number;
  fts_rows?: number;
  error?: string;
}
