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
  | { RunFinished: { summary: string } };

export interface SessionSummary {
  session_id: string;
  created_at?: string | null;
  updated_at?: string | null;
  entries?: number;
  title?: string | null;
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
}

export interface DiffResponse {
  root: string;
  diff: string;
  staged_diff: string;
  status: string;
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
}
