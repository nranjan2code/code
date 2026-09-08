export interface SessionListItem {
  session_id: string;
  project_hash: string;
  entry_count: number;
  first_ts: string;
  last_ts: string;
  /** From the shared `archive.json`, not scoped to a workspace. */
  archived: boolean;
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
  posture?: "healthy" | "degraded" | string;
  provider: string;
  model: string;
  provider_source?: string;
  model_source?: string;
  route_revision?: string;
  permission_mode: string;
  approval_mode: "ask" | "approve-safe" | "auto-approve";
  sandbox: string;
  context_window: number;
  cwd: string;
  warnings: string[];
}


/// One bot identity (multi-bot-per-channel). Independent of chat bindings —
/// several chats can share a bot, and a chat's `bot_id` names one of these.
/// The token itself is never part of this shape; it's set/cleared via
/// `PUT/DELETE /gateway/bots/:id/token`.
/// A chat transport, as the server defines it. The UI keeps no list of
/// its own: adding a transport is a server-side edit, and no channel can
/// become the implicit default by being the one a UI hardcoded.
export interface ChatSurface {
  id: string;
  label: string;
}

export interface Bot {
  id: string;
  surface: string;
  label: string;
  token_env: string;
  /// Whether a token is actually set for this bot. The token itself is
  /// never returned once stored.
  token_configured: boolean;
  policy: ChannelPolicy;
  permission_mode: PermissionMode | null;
  route: AllowlistRoute | null;
  workspace: string | null;
  /// `null`/absent = inherit (no voice at this tier); a present object
  /// pins a spoken voice + persona for this bot's replies.
  voice?: VoiceConfig | null;
}

/// A pinned spoken voice + persona for Live API voice synthesis, mirroring
/// `AllowlistRoute`'s "inherit unless overridden" idiom. Both fields are
/// independently optional: a voice name with no persona, or vice versa.
export interface VoiceConfig {
  /// A Gemini Live API prebuilt voice name, e.g. "Kore", "Puck", "Zephyr".
  voice_name?: string | null;
  /// Free-text style directive fed into the synthesis system instruction,
  /// e.g. "warm, upbeat, and enthusiastic".
  persona?: string | null;
}

export interface ConfigInfo {
  provider: string;
  model: string;
  provider_source?: string;
  model_source?: string;
  route_revision?: string;
  max_turns: number;
  permission_mode: string;
  approval_mode: "ask" | "approve-safe" | "auto-approve";
  sandbox: string;
  theme: string;
  subagents: boolean;
  /** Resolved permission rule lists, exactly as the engine evaluates them. */
  permissions?: PermissionRules;
  /** `project_hash` of the workspace this server process is bound to.
   * `/admin/api/sessions` lists sessions across every project the store
   * indexes, but attach/run/steer/cancel/diff/receipts/archive/delete/
   * markdown-export only ever reach a ledger file under this one
   * workspace's directory — a session whose own `project_hash` differs
   * from this can only be read (transcript, checkpoints), never mutated,
   * from this console instance. */
  workspace_project_hash?: string;
  memory?: {
    search_enabled: boolean;
    write_enabled: boolean;
    skill_proposals: boolean;
    reflection: boolean;
  };
}

export interface PermissionRules {
  allow: string[];
  ask: string[];
  deny: string[];
}

/// `GET/PUT /config/permissions`. `effective` is what the engine evaluates
/// after every layer merges; `layer` is only what the selected scope's own
/// file sets, so a reader can tell an inherited rule from one set here.
export interface PermissionRulesView {
  scope: ConfigScope;
  effective: PermissionRules;
  layer: PermissionRules;
}

/// `GET/PUT /gateway/approvals` — whether an `Ask` raised on a chat surface
/// reaches a human, and which chat answers it.
export interface GatewayApprovalPolicy {
  mode: "deny" | "forward";
  approver: string | null;
  timeout_secs: number;
  /// The gateway itself being off makes a forward policy inert.
  enabled?: boolean;
  gateway_enabled?: boolean;
  /// True only when the policy is `forward`, a target is set, AND the
  /// gateway is on — the same condition dispatch checks.
  forwarding: boolean;
  /// Approved chats, as `<surface>:<chat>` delivery addresses.
  candidates?: string[];
}

/// Result of answering one gate. `learned_rule` is the spec that was
/// persisted when the operator asked not to be asked again; `learn_error`
/// says why no rule could be derived, which never blocks the approval.
export interface ApprovalAnswer {
  approved: boolean;
  learned_rule: string | null;
  learn_error: string | null;
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
  paused: boolean;
}

export interface CorePoolEntry {
  workspace: string;
  is_default: boolean;
  state: "warm";
  idle_secs: number;
  /// The pool is keyed by (workspace, permission override), so one
  /// workspace can appear more than once with different overrides.
  permission_override: PermissionMode | null;
  effective_permission_mode: PermissionMode;
}

export interface CorePoolStatus {
  max: number;
  idle_secs: number;
  entries: CorePoolEntry[];
}

export interface GatewayStatus {
  enabled: boolean;
  workspace: string;
  canonical_default_workspace: string;
  default_route: RouteInfo;
  bindings: GatewayBinding[];
  chat_allowlist: string[];
  chat_allowlist_open: boolean;
  core_pool: CorePoolStatus;
  /// Workspaces vak has session ledgers for, plus the gateway's own cwd —
  /// the options the workspace picker offers before free text.
  known_workspaces: string[];
  workspace_catalog?: Array<{ path: string; name: string }>;
}

export type AllowlistStatus = "pending" | "allowed" | "denied";

export interface AllowlistRoute {
  provider: string;
  model: string;
}

/// Wire form of `vak_config::PermissionMode` (serde kebab-case).
export type PermissionMode = "read-only" | "workspace-write" | "full-access";

export interface ChannelPolicy {
  tools_allow: string[] | null;
  tools_deny: string[];
  mcp_allow: string[] | null;
  mcp_deny: string[];
  skills_allow: string[] | null;
  skills_deny: string[];
  hooks_allow: string[] | null;
  hooks_deny: string[];
  /** Server-name patterns (`name/*`, same shape as mcp_allow/mcp_deny) for
   * which this channel forces outbound network off, even when the server's
   * own config has it on. Restrictive only — there is no "network_allow";
   * a channel can take network away from a server, never grant it to one
   * the server config itself denies. */
  mcp_network_deny: string[];
}

export interface AllowlistEntry {
  key: string;
  status: AllowlistStatus;
  workspace: string | null;
  route: AllowlistRoute | null;
  /// What the operator pinned, if anything. `null` = inherit the workspace.
  permission_mode: PermissionMode | null;
  /// The target workspace's own configured mode — the ceiling an override
  /// is capped to. `null` when the entry names no workspace.
  workspace_permission_mode: PermissionMode | null;
  /// What the channel actually gets: min(pin, workspace mode).
  effective_permission_mode: PermissionMode | null;
  /// True when the pin asked for more than the workspace allows and was
  /// reduced — the console must not show it as a live grant.
  permission_capped: boolean;
  policy: ChannelPolicy;
  /// Which `Bot` (if any) this chat is bound to, when its surface has more
  /// than one configured.
  bot_id: string | null;
  /// `false` = this chat ignores its bot's policy/permission_mode/route
  /// tier entirely and resolves purely against the workspace — the
  /// explicit "break inheritance" switch. Meaningless when `bot_id` is null.
  inherit_bot_policy: boolean;
  /// `null`/absent = inherit this chat's bot's voice (or the bot's own
  /// inherit chain); a present object pins a voice + persona for this chat.
  voice?: VoiceConfig | null;
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
  pool_env_var?: string | null;
  pool_size?: number;
  credential_ids?: string[];
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

/// Matches `finops_status`'s actual JSON exactly (`vak-server/src/lib.rs`)
/// — this used to name fields (`total_spend_usd`, `budget_admission`, …)
/// the backend never sent, so every reader of it always saw `undefined`.
export interface FinOpsRollupEntry {
  name: string;
  usd: number;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
}

export interface FinOpsDailyPoint {
  date: string;
  usd: number;
}

export interface FinOpsAlertRow {
  kind: string;
  ts: string;
  level: "eighty" | "full";
  day_total_usd: number;
  session_id: string;
}

export interface FinOpsStatus {
  day_usd: number;
  day_input_tokens: number;
  day_output_tokens: number;
  day_cache_read_tokens: number;
  activity: { kind: string; name: string; plugin?: string | null; calls: number; successes: number; duration_ms: number }[];
  run_cap_usd: number | null;
  day_cap_usd: number | null;
  unknown_rows: number;
  total_rows: number;
  by_provider: FinOpsRollupEntry[];
  by_model: FinOpsRollupEntry[];
  daily: FinOpsDailyPoint[];
  recent_alerts: FinOpsAlertRow[];
}

export interface OpsStatus {
  gateway: { state: string };
  bridges: { state: string };
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

export interface OperationsSnapshot {
  generated_at: string;
  server: {
    pid: number;
    version: string;
    uptime_secs: number;
    cwd: string;
    posture: "healthy" | "degraded" | string;
  };
  health: HealthInfo & {
    checks: Array<{ label: string; status: "pass" | "fail" | string; detail: string }>;
    facts: string[];
    failures: number;
  };
  services: OpsStatus;
  gateway: {
    enabled: boolean;
    approvals: { pending: number; mode: string; approver: string | null };
    workspace_catalog?: Array<{ path: string; name: string }>;
    bindings: Array<{
      target: string;
      session_id: string | null;
      workspace: string | null;
      provider: string | null;
      model: string | null;
      route_revision: string | null;
    }>;
  };
  pool: {
    max: number;
    idle_secs: number;
    entries: CorePoolEntry[];
  };
  runs: Array<{
    session_id: string;
    workspace: string;
    state: "running" | "waiting_approval" | string;
    pending_approvals: Array<{ id: string; tool: string; reason: string; requested_at: string }>;
  }>;
  tasks: Array<TaskItem & { cwd?: string; next_fire?: string | null; running?: boolean }>;
  outbox: {
    pending: number;
    dead_letter: number;
    error?: string | null;
    records: Array<{
      job_id: string;
      target: string;
      kind: string;
      state: "pending" | "delivered" | "dead_letter" | string;
      attempts: number;
      created_at_ms: number;
      updated_at_ms: number;
      last_error: string | null;
    }>;
  };
  security: SecurityEvent[];
  incidents: Array<{
    id: string;
    fingerprint?: string;
    severity: "critical" | "warning" | "info" | string;
    status?: "investigating" | "resolved" | string;
    source: string;
    title: string;
    detail: string;
    first_seen?: string;
    last_seen?: string;
    occurrences?: number;
    workspace?: string | null;
    evidence?: string[];
    resolution?: string | null;
  }>;
  actions: Array<{
    receipt_id: string;
    service: string;
    action: string;
    requested_at: string;
    completed_at: string;
    succeeded: boolean;
    verification: { status: string; before: string; after: string; detail: string };
    persisted: boolean;
  }>;
  ops_port: number;
  bus: BusStatus | null;
}

export interface BusStatus {
  workspace_id: string;
  backend: "nats" | "memory";
  connected: boolean;
  encrypted: boolean;
  metrics: {
    published_count: number;
    received_count: number;
    bytes_published: number;
    bytes_received: number;
    dead_letter_count: number;
    active_queue_lag: number;
  };
}

export interface BusConfig {
  nats_url: string | null;
  encrypted: boolean;
  runtime: BusStatus | null;
}

export type SandboxRecordKind = "environment" | "candidate" | "promotion";

export interface SandboxEnvironmentRecord {
  kind: "environment";
  record_id: string;
  environment_id: string;
  state: string;
  plan: {
    id: string;
    outcome_revision: number;
    backend: string;
    image?: string | null;
    network_policy: string;
    input_root: string;
    task_root: string;
  };
  updated_at: string;
  detail?: string | null;
}

export interface SandboxCandidateRecord {
  kind: "candidate";
  record_id: string;
  environment_id: string;
  candidate: {
    candidate_id: string;
    source_root: string;
    destination_root: string;
    files: Array<{ path: string; candidate_hash: string; base_hash?: string | null; bytes: number }>;
  };
  verified: boolean;
  updated_at: string;
}

export interface SandboxPromotionRecord {
  kind: "promotion";
  record_id: string;
  candidate_id: string;
  receipt: {
    candidate_id: string;
    applied: string[];
    before_hashes: Array<[string, string | null]>;
    after_hashes: Array<[string, string]>;
    verification: Array<{ path: string; status: string; evidence: string }>;
  };
  updated_at: string;
}

export type SandboxRecord =
  | SandboxEnvironmentRecord
  | SandboxCandidateRecord
  | SandboxPromotionRecord;

export interface SandboxRecordsResponse {
  records: SandboxRecord[];
}

export interface CandidateManifest {
  candidate_id: string;
  source_root: string;
  destination_root: string;
  files: CandidateFile[];
}

export interface CandidateFile {
  path: string;
  candidate_hash: string;
  base_hash?: string | null;
  bytes: number;
}

export interface PromotionReceipt {
  candidate_id: string;
  applied: string[];
  before_hashes: Array<[string, string | null]>;
  after_hashes: Array<[string, string]>;
  verification: VerificationResult[];
}

export interface VerificationResult {
  path: string;
  status: string;
  evidence: string;
}

export interface SandboxExecutionEvent {
  event: string;
  ts: string;
  [key: string]: unknown;
}

export interface SandboxExecutionsResponse {
  session_id: string;
  events: SandboxExecutionEvent[];
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

export type ConfigScope = "user" | "project";

// Prompt layers (docs/design/45-prompt-layers.md). Only these three blocks
// are editable; the capability contract, the Surface line, and the skill/MCP
// inventories are code-owned and describe the interface as it actually is.
export type PromptBlock = "identity" | "operating-rules" | "guardrails" | "surface-note";

export interface PromptLayerContent {
  identity?: string | null;
  operating_rules?: string | null;
  guardrails?: string[];
  surface_notes?: string[];
}

export interface PromptLayerResponse {
  scope: ConfigScope;
  path: string;
  layer: PromptLayerContent;
}

export interface PromptLayerDescriptor {
  block: PromptBlock;
  layer: "seed" | "shared" | "project" | "surface" | "bot" | "chat" | "agent";
  source?: string | null;
  digest: string;
  bytes: number;
}

export interface PromptEffective {
  text: string;
  fingerprint: string;
  estimated_tokens: number;
  surface: string;
  layers: PromptLayerDescriptor[];
}

export interface ConfigLayer {
  scope: ConfigScope;
  path: string;
  provider?: string | null;
  model?: string | null;
  max_tokens?: number | null;
  max_turns?: number | null;
  permission_mode?: string | null;
  approval_mode?: string | null;
  profile?: string | null;
  subagents?: boolean | null;
  theme?: string | null;
  permissions: { allow: string[]; ask: string[]; deny: string[] };
  memory: {
    search_enabled?: boolean | null;
    write_enabled?: boolean | null;
    reflection?: boolean | null;
    skill_proposals?: boolean | null;
  };
  work: {
    enabled?: boolean | null;
    default_mode?: string | null;
    max_items?: number | null;
    max_revisions?: number | null;
    max_parallel?: number | null;
    confirmation?: string | null;
  };
  counts: { mcp: number; hooks: number };
  capabilities: {
    inherit_mcp?: boolean | null;
    inherit_hooks?: boolean | null;
    inherit_skills?: boolean | null;
    inherit_plugins?: boolean | null;
  };
}

export interface IntegrationStatus {
  id: string;
  label: string;
  description: string;
  command: string;
  args: string[];
  network: boolean;
  env_var?: string | null;
  key_required: boolean;
  documentation_url: string;
  scope: ConfigScope;
  configured_here: boolean;
  inherited: boolean;
  effective: boolean;
  key_here: boolean;
  key_inherited: boolean;
  key_effective: boolean;
}

export interface TavilyStatus {
  enabled: boolean;
  key_present: boolean;
  network: boolean;
  env_var: string;
}

export interface HookConfig {
  event: string;
  matcher?: string | null;
  command: string;
  timeout_ms: number;
  enabled: boolean;
  failure_mode?: "open" | "closed";
}

export interface SkillItem {
  name: string;
  path?: string;
  description?: string;
  /** Discovery root the skill came from: `<cwd>/.vak/skills` or the user home. */
  scope?: "workspace" | "user";
  provenance?: string | null;
  shadowed?: boolean;
}

export interface PluginItem {
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

export interface WorkProjection {
  contract: {
    contract_id: string;
    objective: string;
    assumptions: Array<{ assumption_id: string; text: string; requires_confirmation: boolean; resolution?: string }>;
  };
  status: string;
  items: Record<string, { status: string; attempt: number; blocker?: string; evidence: unknown[] }>;
  criteria: Record<string, unknown>;
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

export interface ActiveSubagent {
  id: string;
  label: string;
  elapsed_secs: number;
  parent_session_id: string;
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
  id?: string;
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
  scope?: "global" | "workspace";
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
  source_id?: string;
  scope?: string;
  workspace_id?: string;
  security_status?: string;
  security_detail?: string;
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
  quarantined_items?: number;
  sources?: Array<{
    name: string;
    type: string;
    item_count: number;
    last_item?: string;
  }>;
}

export interface ConfiguredFeedSource {
  id: string;
  name: string;
  source_type: string;
  url?: string;
  trust: string;
  enabled: boolean;
  check_interval?: string;
  scope?: string;
  workspace_id?: string;
  security_status?: string;
  last_started_at?: string;
  next_due_at?: string;
  last_status?: string;
  last_error?: string;
}

export interface FeedAlertRule {
  id: number;
  name: string;
  scope?: string;
  match_config?: {
    keywords?: string[];
    tags?: string[];
    sources?: string[];
  };
  action?: string;
  deliver_to?: string;
  hook_command?: string;
  cooldown_minutes?: number;
  enabled?: boolean;
}

export interface FeedRun {
  id: number;
  run_key: string;
  scope: string;
  workspace_id: string;
  source_id?: string;
  status: string;
  started_at: string;
  finished_at?: string;
  sources_seen: number;
  sources_succeeded: number;
  items_seen: number;
  items_added: number;
  items_quarantined: number;
  error?: string;
}

export interface FeedQuarantineItem {
  id: number;
  title: string;
  url: string;
  security_status: string;
  security_detail?: string;
  ingested_at?: string;
  source_name?: string;
  scope?: string;
  workspace_id?: string;
}

/// The derived setup projection (`GET /onboarding`). Mirrors
/// `vak_core::onboarding` exactly: readiness is derived on every read, so
/// this is never cached and never trusted from a stored value.
export interface StepFailure {
  what: string;
  preserved: string;
  repair: string;
  detail?: string | null;
}

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

// --- Intent kernel + commitments (docs/design/47-commitment-kernel.md) ------

export type Act =
  | "converse" | "answer" | "locate" | "analyze" | "author"
  | "modify" | "operate" | "verify" | "orchestrate" | "govern";
export type Horizon = "immediate" | "turn" | "session" | "durable";
export type Stakes = "inert" | "reversible" | "costly" | "irreversible";
export type EvidenceAxis = "none" | "cited" | "verified" | "audited";
export type Clarity = "clear" | "underspecified" | "ambiguous";
export type Attendance = "interactive" | "supervised" | "unattended";
/** Ordered weakest to strongest; the whole "is it done" question lives here. */
export type Satisfaction = "asserted" | "cited" | "observed" | "attested";
export type Verdict =
  | "fulfilled" | "partial" | "failed"
  | "abandoned" | "superseded" | "expired" | "unknown";
export type Phase =
  | "proposed" | "active" | "suspended" | "blocked" | "satisfying" | "closed";

export interface Confidences {
  act: number;
  horizon: number;
  stakes: number;
  evidence: number;
}

export interface Reading {
  act: Act;
  horizon: Horizon;
  stakes: Stakes;
  evidence: EvidenceAxis;
  clarity: Clarity;
  attendance: Attendance;
  domains: string[];
  alternate_acts?: Act[];
  confidence: number;
  axis_confidence: Confidences;
}

export interface IntentSignal {
  kind: string;
  name: string;
  weight: number;
  detail: string;
}

export interface Provenance {
  tier: "declared" | "signals" | "local-model" | "cloud-model" | "general";
  resolver_version: number;
  signals: IntentSignal[];
  model?: string | null;
  prompt_digest?: string | null;
  /** False means a model decided it; never claim replay fidelity you lack. */
  reproducible: boolean;
  escalation_note?: string | null;
}

export interface Posture {
  managed: boolean;
  open_commitment: boolean;
  checkpoint_before_effect: boolean;
  hil: "interrupt" | "envelope" | "review" | "defer";
  gate_fallback: "deny" | "defer";
  clarify: "proceed" | "state-assumption" | "ask";
  delivery: {
    shape: string;
    cadence: "live" | "on-completion" | "digest";
    urgency: "interrupt" | "notify" | "quiet";
  };
  demand: {
    reasoning_required: boolean;
    evidence_required: boolean;
    structured_output: boolean;
  };
  stop: "message" | "inspection" | "effect" | "verification";
  context: "minimal" | "recall" | "working" | "full";
  note?: string | null;
}

export interface Engagement {
  limits: {
    capabilities: { kind: "all" } | { kind: "only"; names: string[] };
    ladder_limit?: number | null;
    required_modalities: string[];
    spend_ceiling_usd?: number | null;
    approval_ceiling: "ask" | "approve-safe" | "auto-approve";
    permission_ceiling: "read-only" | "workspace-write" | "full-access";
    subagent_budget?: number | null;
    max_turns?: number | null;
    min_satisfaction: Satisfaction;
  };
  posture: Posture;
}

export interface IntentExplain {
  reading: Reading;
  engagement: Engagement;
  provenance: Provenance;
  /** Human-readable list of what this narrows versus doing nothing. */
  narrows: string[];
  escalation_recommended?: string | null;
  model_visible?: string | null;
}

export interface IntentPolicy {
  intent: {
    enabled: boolean;
    accept_confidence: number;
    provisional_confidence: number;
    slice_capabilities: boolean;
    posture: boolean;
    escalate: string;
    max_classify_usd: number;
    autonomy: string;
  };
  commitment: {
    enabled: boolean;
    lifetime_budget_usd?: number | null;
    stall_limit: number;
    review_every_hours?: number | null;
    default_ttl_days?: number | null;
  };
}

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
    reading: Reading;
    min_satisfaction: Satisfaction;
    economics: {
      lifetime_budget_usd?: number | null;
      expires_at?: string | null;
      review_every_hours?: number | null;
      stall_limit: number;
    };
  };
  phase: Phase;
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

/** Why the scheduler ranked a commitment where it did. */
export interface CommitmentPriority {
  commitment_id: string;
  score: number;
  components: [string, number][];
  /** Present when the commitment cannot be worked now, with the reason. */
  withheld?: string | null;
}

export interface CommitmentList {
  commitments: Commitment[];
  priorities: CommitmentPriority[];
}
