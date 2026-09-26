// Mirrors the serde serialization of vak-server's client-facing ClientEvent
// projection and vak-llm types. Any drift here is a contract bug — the
// JSONL ledger is truth.

export type Role = "user" | "assistant" | "User" | "Assistant";

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

export type OutputRole = "system" | "user" | "assistant" | "tool" | "worker";
export type OutputKind =
  | "message"
  | "information"
  | "approval"
  | "progress"
  | "retry"
  | "error"
  | "outcome"
  | "artifact"
  | "card";
export type OutputStatus =
  | "pending"
  | "running"
  | "succeeded"
  | "failed"
  | "denied"
  | "cancelled"
  | "partial";

export type InlineNode =
  | { type: "text"; text: string }
  | { type: "strong"; content: InlineNode[] }
  | { type: "emphasis"; content: InlineNode[] }
  | { type: "strikethrough"; content: InlineNode[] }
  | { type: "code"; code: string }
  | { type: "link"; label: InlineNode[]; url: string; title?: string | null; safe: boolean }
  | { type: "image"; alt: string; url: string; title?: string | null; safe: boolean }
  | { type: "soft_break" }
  | { type: "hard_break" }
  | { type: "raw_html"; html: string };

export interface ArtifactRef {
  name: string;
  path?: string | null;
  media_type?: string | null;
  description?: string | null;
  /** Observed size; shown only with technical details on. */
  size_bytes?: number | null;
  /** Where the file stands, from the server's durable records; absent when unknown. */
  status?: ArtifactStatus | null;
}

/** A saved draft version and the file's path inside it. */
export interface VersionFile {
  version_id: string;
  path: string;
}

export type ArtifactStatus =
  | { state: "draft"; version: number; saved_as?: VersionFile | null }
  | { state: "accepted"; version: number; saved_as?: VersionFile | null }
  | { state: "in_folder" };

export interface StructuredOutput {
  semantic_type: string;
  schema_version: number;
  skill_id: string;
  skill_version: string;
  payload: Record<string, unknown>;
}

export type DocumentBlock =
  | { type: "heading"; id: string; level: number; content: InlineNode[] }
  | { type: "paragraph"; id: string; content: InlineNode[] }
  | { type: "list"; id: string; ordered: boolean; start?: number | null; items: DocumentBlock[][] }
  | { type: "table"; id: string; alignments: string[]; header: InlineNode[][]; rows: InlineNode[][][] }
  | { type: "quote"; id: string; blocks: DocumentBlock[] }
  | { type: "code"; id: string; language?: string | null; filename?: string | null; content: string }
  | { type: "callout"; id: string; tone: string; title?: string | null; blocks: DocumentBlock[] }
  | { type: "diff"; id: string; content: string }
  | { type: "structured"; id: string; output: StructuredOutput; fallback_markdown: string }
  | { type: "diagram"; id: string; source: string; fallback_markdown: string }
  | { type: "citations"; id: string; items: { label: string; url: string; title?: string | null }[] }
  | { type: "media"; id: string; source: string; alt: string; media_type?: string | null }
  | { type: "artifact_ref"; id: string; artifact: ArtifactRef }
  | { type: "rule"; id: string }
  | { type: "raw_markdown"; id: string; markdown: string; reason: string };

export interface PresentationDocument {
  schema_version: number;
  source_markdown: string;
  blocks: DocumentBlock[];
  coverage: { block_id: string; disposition: "native" | "fallback"; diagnostic?: string | null }[];
  metadata: Record<string, string>;
  diagnostics: string[];
}

export type OutputContent =
  | { type: "document"; document: PresentationDocument }
  | { type: "information"; label: string; detail?: string | null }
  | { type: "approval"; request_id: string; tool: string; args_json: string; reason: string; expires_at?: string | null }
  | { type: "progress"; label: string; detail?: string | null; percent?: number | null }
  | { type: "retry"; attempt: number; delay_ms: number; reason: string }
  | { type: "error"; message: string; source?: string | null; retryable: boolean }
  | { type: "outcome"; summary: string; document?: PresentationDocument | null }
  | { type: "artifact"; artifact: ArtifactRef }
  | { type: "structured"; output: StructuredOutput }
  | { type: "adaptive"; tree: AdaptiveRenderTree; fallback_text: string };

export interface AdaptiveRenderTree {
  schema_version: number;
  spec_id: string;
  revision: number;
  digest: string;
  root: AdaptiveRenderNode;
  accessibility_summary?: string | null;
  coverage: { rendered_paths: string[]; omitted_paths: string[] };
}

export interface AdaptiveRenderNode {
  primitive: string;
  props: Record<string, unknown>;
  children: AdaptiveRenderNode[];
}

export interface OutputAction {
  id: string;
  label: string;
  verb: string;
  data: Record<string, string>;
}

export interface OutputProvenance {
  session_id?: string | null;
  entry_id?: string | null;
  tool_call_id?: string | null;
  source?: string | null;
  /** The `Presentation` ledger entry's own id (docs/design/68-context-engine.md
   * §10) — distinct from `entry_id`, which names the message entry the card's
   * tool call rode in on. `/presentation/feedback` and `/presentation/select`
   * key on this id, not on `entry_id`. `null`/absent for non-card items and
   * for cards not sourced from a written `Presentation` entry. */
  presentation_id?: string | null;
}

/**
 * One entry per transcript message, same order and length: the ledger entry it
 * came from. The chat pairs a turn with the projection by this id, never by
 * position. (The server sends only what a person can see; runtime-authored
 * nudges and derived context blocks are never sent.)
 */
export interface TranscriptEntryMeta {
  entry_id: string;
  author_id?: string | null;
  author_name?: string | null;
  /** Files attached to this message; `block` is the content block that
   *  names each to the model, which the chat draws as the file instead. */
  attachments?: { block: number; path: string; name: string; bytes: number }[];
}

export interface OutputItem {
  id: string;
  timestamp: string;
  turn_id: string;
  role: OutputRole;
  kind: OutputKind;
  status: OutputStatus;
  outcome?: ResultOutcome | null;
  content: OutputContent;
  provenance?: OutputProvenance | null;
  actions: OutputAction[];
  fallback_text: string;
}

export interface ResultOutcome {
  result_id: string;
  status: OutputStatus;
  completion?: string | null;
  evidence_state?: string | null;
  requirement_ids: string[];
  evidence_receipt_ids: string[];
  human_review?: string | null;
}

export interface OutputTimeline {
  schema_version: number;
  session_id: string;
  cursor?: string | null;
  items: OutputItem[];
  diagnostics: string[];
  goal?: GoalState | null;
}

export interface GoalState {
  revision: number;
  objective: string;
  additions: string[];
  superseded_revisions: number[];
  control: "active" | "paused" | "cancelled";
}

export type PresentationDelta =
  | { type: "item_started"; item: OutputItem }
  | { type: "text_delta"; item_id: string; delta: string; text: string }
  | { type: "item_replaced"; item: OutputItem }
  | { type: "item_completed"; item_id: string; status: OutputStatus };

export interface PresentationStreamEvent {
  sequence: number | null;
  /** Present only on a live delta frame; `null` on a snapshot frame. */
  delta: PresentationDelta | null;
  /**
   * Present only on the initial connect frame, run settlement, and an
   * explicit resync — never alongside `delta`. A frame with neither set is
   * not valid; `applyPresentationEvent` treats a present `snapshot` as the
   * sole authority and otherwise applies `delta`.
   */
  snapshot: OutputTimeline | null;
}

export type SandboxEvent =
  | {
      kind: "ExecutionStarted";
      execution_id: string;
      owner_session_id?: string | null;
      tool: string;
      code_preview: string;
      language: string;
      scratch_dir: string;
    }
  | { kind: "Stdout"; execution_id: string; chunk: string }
  | { kind: "Stderr"; execution_id: string; chunk: string }
  | { kind: "OutputTruncated"; execution_id: string }
  | { kind: "PackageInstalled"; execution_id: string; packages: string[] }
  | { kind: "ArtifactGenerated"; execution_id: string; path: string; mime_type: string; size_bytes: number }
  | { kind: "ProcessTelemetry"; execution_id: string; elapsed_ms: number; cpu_percent: number; memory_bytes: number }
  | { kind: "ExecutionFinished"; execution_id: string; exit_code: number; duration_ms: number; artifacts: string[] };

/**
 * Mirrors the serde serialization of `vak_server::client_events::RunOutcome`
 * — the run's outcome reduced to the handful of states a person may see.
 * Never carries raw provider/internal error text; see `ClientEvent.RunFinished`.
 */
export type RunOutcome = "Completed" | "Stopped" | "MaxTurns" | "Failed";

/**
 * Mirrors the serde serialization of `vak_server::client_events::ClientEvent`
 * — the one server-side projection of `vak_agent::AgentEvent` that
 * `/sessions/:id/events` and `/sessions/:id/side/events` are allowed to
 * send. Internal bookkeeping (retry attempts/reasons, route fallback,
 * context compaction, worker token counts, raw error text) never crosses
 * this boundary; see docs/design/30-output-engineering.md and AGENTS.md
 * ("Runtime-authored traffic is typed, never sniffed").
 */
export type ClientEvent =
  | "StreamOpened"
  | { TurnStart: { turn: number } }
  | { TextDelta: { delta: string } }
  | { ThinkingDelta: { delta: string } }
  | { ToolCallStart: { id: string; name: string; args_json: string } }
  | {
      ToolCallEnd: { id: string; name: string; is_error: boolean; result_preview: string | null };
    }
  | { ApprovalRequested: { id: string; tool: string; args_json: string; reason: string } }
  | { WorkerStarted: { label: string } }
  | { WorkerToolCall: { label: string; name: string; is_error: boolean } }
  | { WorkerFinished: { label: string; is_error: boolean; elapsed_ms: number } }
  /** A long provider-side backoff is happening. No attempt count, delay or
   *  raw reason rides along — the header shows a neutral "Retrying" state. */
  | "Retrying"
  | { Sandbox: SandboxEvent }
  /** The runtime sent a text answer back for a redo; the client drops the
   *  discarded draft bubble for this turn. */
  | { DraftDiscarded: { turn: number } }
  | { RunFinished: { outcome: RunOutcome; message: string } };

export interface SessionSummary {
  agent?: { id: string; name: string; revision: number; character?: string; animation?: "subtle" | "expressive" | "off"; voice?: string } | null;
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
  pool_env_var?: string | null;
  pool_size?: number;
  credential_ids?: string[];
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
  /** Absent on the web host: the API is same-origin there, and auth rides
   *  an HttpOnly cookie rather than a token the client ever holds. */
  base_url?: string;
  token?: string;
  cwd?: string;
  boot_error?: string;
  recent_workspaces: string[];
  /** Web host only: whether a real PTY is reachable ([server.web] terminal). */
  terminal?: boolean;
  version?: string;
}

/** What a folder would ask for, read as text and never through the config
 *  loader — describing a project's privileged settings must not activate
 *  them (docs/design/46 Step 2). */
export interface WorkspaceReview {
  path: string;
  git: boolean;
  requests_privilege: boolean;
  privileges: string[];
  trusted: boolean;
}

/** One entry in the server-side directory browser (web host). */
export interface DirEntry {
  name: string;
  path: string;
  git: boolean;
}

export interface DirListing {
  path: string;
  parent?: string | null;
  entries: DirEntry[];
}

export interface ConfigSnapshot {
  voice?: { enabled: boolean; provider?: string | null; transcription_model?: string | null; synthesis_model?: string | null; max_session_secs: number; max_concurrent: number; max_audio_bytes: number; source?: string };
  provider: string;
  model: string;
  provider_source?: string;
  model_source?: string;
  max_tokens: number;
  max_turns: number;
  intent_evidence_max_age_secs?: number;
  permission_mode: "ReadOnly" | "WorkspaceWrite" | "FullAccess";
  /// How an `Ask` decision is resolved. Separate from the permission mode:
  /// it never widens the boundary or switches off the sandbox, it only
  /// decides who answers the gate.
  approval_mode: "ask" | "approve-safe" | "auto-approve";
  sandbox: string;
  /// The rule lists the engine evaluates, effective across all layers.
  permissions: { allow: string[]; ask: string[]; deny: string[] };
  workers: boolean;
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
  /// Transports a bot can be created on. A surface is not a credential
  /// slot (AGENTS.md invariant 23) — bot tokens live on bots, and the
  /// admin console reports them.
  surfaces: string[];
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
  last_result_id?: string | null;
  last_run_status?: string | null;
  last_delivery_state?: string | null;
  last_wt?: { path: string; branch: string } | null;
  deliver_to?: string | null;
  /** 5-field cron (`m h dom mon dow`, local time); replaces interval ticks. */
  schedule?: string | null;
  /** Watchdog shell one-liner; XOR with prompt (docs/design/29 P2). */
  script?: string | null;
  /** Pinned model id; a pinned task never escalates. */
  model_pin?: string | null;
  agent_id?: string | null;
  agent_revision?: number | null;
  next_run_at?: string | null;
  timezone?: string | null;
}

export interface OpsServiceState {
  state: string;
}

export interface OpsStatus {
  gateway: OpsServiceState;
  bridges: OpsServiceState;
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

export interface WorkProjection {
  contract: { contract_id: string; objective: string };
  status: string;
  items: Record<string, { status: string; attempt: number; blocker?: string; evidence: unknown[] }>;
  criteria: Record<string, unknown>;
}

export interface FeedSourceType {
  id: string;
  name: string;
  icon: string;
  description: string;
  fetcher: string;
  default_interval: string;
}

export interface FeedSource {
  name: string;
  type: string;
  url?: string;
  driver?: string;
  channel_id?: string;
  variant?: string;
  tags?: string[];
  trust?: string;
  enabled?: boolean;
  interval?: string;
}

export interface FeedItem {
  id: number;
  feed_id: number;
  title: string;
  url: string;
  author?: string;
  summary: string;
  content?: string;
  published_at?: string;
  ingested_at?: string;
  tags?: string[];
  source_trust?: string;
  word_count?: number;
  source_name?: string;
  source_type?: string;
}

export interface FeedSearchResult {
  url: string;
  title: string;
  content: string;
  score: number;
  published_date?: string;
  source_name?: string;
  source_type?: string;
  tags?: string[];
  highlights?: string[];
  evidence?: {
    excerpts?: Array<{ text: string; relevance: number }>;
    source_trust?: string;
    freshness_hours?: number;
    corroboration_count?: number;
  };
}

export interface FeedSearchResponse {
  query: string;
  answer?: string;
  follow_up_questions?: string[];
  results: FeedSearchResult[];
  meta: {
    total_results: number;
    returned_results: number;
    search_time_ms: number;
    sources_searched?: string[];
    deduplicated?: number;
  };
}

export interface FeedStats {
  total_items: number;
  total_feeds: number;
  active_feeds: number;
  total_alerts: number;
  last_ingest?: string;
  items_today: number;
  sources?: Array<{
    name: string;
    type: string;
    item_count: number;
    last_item?: string;
  }>;
}

/// The derived setup projection (`GET /onboarding`), mirroring
/// `vak_core::onboarding`. Never cached: readiness is derived on every
/// read, so a key removed elsewhere makes setup incomplete again here.
export type StepState =
  | { state: "satisfied"; detail: string; provenance?: string | null }
  | { state: "incomplete"; what: string; preserved: string; repair: string; detail?: string | null }
  | { state: "not_applicable"; reason: string };

export interface OnboardingState {
  install: StepState;
  dependencies: StepState;
  workspace: StepState;
  trust: StepState;
  provider: StepState;
  route: StepState;
  permission: StepState;
  sandbox: StepState;
  capabilities: StepState;
  integrations: StepState;
  channels: StepState;
  services: StepState;
  first_result: StepState;
  core_ready: boolean;
  unattended_ready: boolean;
}

export interface VoiceProviderDescriptor {
  name: "gemini" | "openai" | "local";
  formats: Array<"pcm16" | "ogg_opus" | "mp3" | "wav">;
  credential_vars: string[];
  configured: boolean;
  readiness: { ready: boolean; detail: string } | null;
}

export interface VoiceProvidersResponse {
  providers: VoiceProviderDescriptor[];
}
