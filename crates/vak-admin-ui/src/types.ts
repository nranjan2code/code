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
  provider: string;
  model: string;
  permission_mode: string;
  sandbox: string;
  context_window: number;
  cwd: string;
  warnings: string[];
}

/// Credential state for one chat bridge, as `/config` reports it
/// (`chat_surface_status` in vak-server). `configured` is the only thing
/// the server will say about a bot token — the value itself never comes
/// back over the wire.
export interface ChatSurfaceStatus {
  surface: string;
  env_var: string;
  configured: boolean;
  /// Telegram alone has a managed service unit; the others are started by
  /// hand, and the console must not promise otherwise.
  managed_service: boolean;
}

/// One bot identity (multi-bot-per-channel). Independent of chat bindings —
/// several chats can share a bot, and a chat's `bot_id` names one of these.
/// The token itself is never part of this shape; it's set/cleared via
/// `PUT/DELETE /gateway/bots/:id/token`, mirroring `ChatSurfaceStatus`.
export interface Bot {
  id: string;
  surface: string;
  label: string;
  token_env: string;
  policy: ChannelPolicy;
  permission_mode: PermissionMode | null;
  route: AllowlistRoute | null;
  workspace: string | null;
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
  default_route: RouteInfo;
  bindings: GatewayBinding[];
  chat_allowlist: string[];
  chat_allowlist_open: boolean;
  core_pool: CorePoolStatus;
  /// Workspaces vak has session ledgers for, plus the gateway's own cwd —
  /// the options the workspace picker offers before free text.
  known_workspaces: string[];
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
}

export interface SkillItem {
  name: string;
  path?: string;
  description?: string;
  /** Discovery root the skill came from: `<cwd>/.vak/skills` or the user home. */
  scope?: "workspace" | "user";
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

export interface FeedAlertRule {
  id: number;
  name: string;
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
