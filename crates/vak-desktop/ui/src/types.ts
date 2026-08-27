// Mirrors the serde serialization of vak-agent's AgentEvent and vak-llm
// types. Any drift here is a contract bug — the JSONL ledger is truth.

export type Role = "User" | "Assistant";

export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "thinking"; text: string; signature?: string }
  | { type: "tool_use"; id: string; name: string; input: unknown }
  | {
      type: "tool_result";
      tool_use_id: string;
      content: string;
      is_error?: boolean;
    };

export interface Usage {
  input_tokens?: number;
  output_tokens?: number;
  cache_read_input_tokens?: number | null;
  cache_creation_input_tokens?: number | null;
}

export interface AssistantMessage {
  content: ContentBlock[];
  stop_reason: string;
  usage: Usage;
  model: string;
}

export interface Message {
  role: Role;
  content: ContentBlock[];
}

type StreamEvent =
  | { Start: { partial: AssistantMessage } }
  | { TextDelta: { delta: string; partial: AssistantMessage } }
  | { ThinkingDelta: { delta: string; partial: AssistantMessage } }
  | {
      ToolUseStart: { index: number; id: string; name: string; partial: AssistantMessage };
    }
  | { ToolInputDelta: { index: number; delta: string; partial: AssistantMessage } }
  | { End: { message: AssistantMessage } };

export type AgentEvent =
  | { TurnStart: { turn: number } }
  | { Stream: StreamEvent }
  | { ToolCallStart: { id: string; name: string; args_json: string } }
  | {
      ToolCallEnd: { id: string; name: string; is_error: boolean; result_preview: string | null };
    }
  | { TurnEnd: { usage: Usage } }
  | { StopHookContinuation: { reason: string } }
  | { RetryScheduled: { attempt: number; delay_ms: number; reason: string } }
  | {
      RouteFallback: {
        to_provider: string;
        to_model: string;
      };
    }
  | { ContextCompacting: { estimated_tokens: number } }
  | {
      ContextCompacted: { before_tokens: number; after_tokens: number; summarized_messages: number };
    }
  | { StreamOpened: Record<string, never> }
  | { ApprovalRequested: { id: string; tool: string; args_json: string; reason: string } }
  | { SubagentStarted: { label: string } }
  | { SubagentToolCall: { label: string; name: string; is_error: boolean } }
  | { SubagentUsage: { label: string; input_tokens: number; output_tokens: number } }
  | { SubagentFinished: { label: string; is_error: boolean; elapsed_ms: number } }
  | { RunFinished: { summary: string; is_error: boolean } };

export interface SessionSummary {
  session_id: string;
  cwd?: string;
  created_at?: string | null;
  updated_at?: string | null;
  entries?: number;
  title?: string | null;
  running?: boolean;
  archived?: boolean;
}

export interface CheckpointInfo {
  seq: number;
  label: string;
  created_at: string;
  files: number;
}

export interface SkillInfo {
  name: string;
  description: string;
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

export interface Health {
  status: string;
  provider: string;
  model: string;
  permission_mode: "ReadOnly" | "WorkspaceWrite" | "FullAccess";
  sandbox: string;
  context_window: number;
  cwd: string;
  warnings: unknown[];
}

export interface BackendInfo {
  ready: boolean;
  base_url?: string;
  token?: string;
  cwd?: string;
  boot_error?: string;
  recent_projects: string[];
}

export interface ConfigSnapshot {
  provider: string;
  model: string;
  provider_source?: string;
  model_source?: string;
  max_tokens: number;
  max_turns: number;
  permission_mode: "ReadOnly" | "WorkspaceWrite" | "FullAccess";
  subagents: boolean;
  max_retries: number;
  retry_base_backoff_ms: number;
  request_timeout_secs: number;
  run_retry_attempts: number;
  run_retry_base_backoff_ms: number;
  circuit_breaker_threshold: number;
  circuit_breaker_cooldown_secs: number;
  context_window: number;
  theme: string;
  bell: boolean;
  stop_policy: { enabled: boolean; marker_gate: boolean; verify_gate: boolean; max_blocks: number };
  route: {
    objective: string;
    fallback_models: string[];
    max_fallbacks: number;
    quality_hints: string[];
  };
  integrations: { mcp_servers: string[]; hooks: number; skills: string[] };
  telegram: { env_var: string; configured: boolean };
  paths: { project_config: string; global_config?: string | null; sessions_home: string; cwd: string };
  warnings: string[];
}

export interface DiffResponse {
  // Absent on the error payload (e.g. the workspace is not a git repo), so
  // these are optional: the server returns { error } alone in that case.
  root?: string;
  diff?: string;
  staged_diff?: string;
  status?: string;
  error?: string;
}

export interface PrCheck {
  name?: string;
  status?: string;
  conclusion?: string | null;
}

export interface PrStatus {
  branch: string;
  pr: {
    number: number;
    title: string;
    url: string;
    state: string;
    mergeable: string;
  } | null;
  reason?: "no_pr" | "gh_unavailable";
  error?: string;
  checks?: PrCheck[];
  summary?: { pass: number; fail: number; pending: number };
}

export interface TaskDef {
  id: string;
  name: string;
  prompt: string;
  interval_secs: number;
  enabled: boolean;
  cwd: string;
  created_at: string;
  last_run_at?: string | null;
  last_session_id?: string | null;
  last_summary?: string | null;
  last_wt?: { path: string; branch: string } | null;
  deliver_to?: string | null;
  /** 5-field cron (`m h dom mon dow`, local time); replaces interval ticks. */
  schedule?: string | null;
  /** Watchdog shell one-liner; XOR with prompt (docs/design/29 P2). */
  script?: string | null;
  /** Pinned model id; a pinned task never escalates. */
  model_pin?: string | null;
}

export interface OpsServiceState {
  state: string;
}

export interface OpsStatus {
  gateway: OpsServiceState;
  telegram: OpsServiceState;
  gateway_healthy: boolean;
}

// Dispatch forensics (docs/design/27 Phases A+B+R). Mirrors the serde
// serialization of vak_llm::work — the JSONL ledger is truth.
export type WorkPurpose = "execute" | "summarize" | "verify";
export type AttemptReason = "initial" | "retry" | "route_fallback" | "endurance_retry";
export type FailureDomain =
  | "account"
  | "provider"
  | "model"
  | "request"
  | "network"
  | "deadline"
  | "unknown";
export type Settlement = "ok" | "failed" | "cancelled" | "unknown";

export interface DispatchAttempt {
  ordinal: number;
  reason: AttemptReason;
  domain: FailureDomain;
  settlement: Settlement;
  latency_ms: number;
  usage?: Usage | null;
  error?: string | null;
  provider?: string | null;
  model?: string | null;
}

export interface WorkReceipt {
  purpose: WorkPurpose;
  provider: string;
  model: string;
  winning_attempt?: number | null;
  attempts: DispatchAttempt[];
}
