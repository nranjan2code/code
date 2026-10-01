import { host } from "./host";
import type { PreviewSource } from "./canvasSubject";
import { restartStream, setStreamOpener } from "./streamHub";
import { syntheticMailCalendarEnabled, syntheticMailCalendarRequest } from "./mailCalendarDemo";
import type {
  ClientEvent,
  BackendInfo,
  DiffResponse,
  Health,
  Message,
  TranscriptEntryMeta,
  SessionSummary,
  ConfigSnapshot,
  WorkReceipt,
  OutputTimeline,
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
  const before = auth;
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
  // A different backend numbers its events from scratch, so the stream
  // reconnects to it without the old one's cursor.
  const moved =
    before.mode !== auth.mode ||
    (before.mode === "bearer" && auth.mode === "bearer" && (before.base !== auth.base || before.token !== auth.token));
  if (moved) restartStream(true);
}

export function isBackendReady(): boolean {
  return auth.mode === "cookie" || !!auth.base;
}

export function listVoiceProviders(agent?: string): Promise<VoiceProvidersResponse> {
  return req<VoiceProvidersResponse>(withAgent("/voice/providers", agent));
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

/** A refusal from the server, with its typed `kind` when it has one, so a
 * caller can act on what went wrong without reading the message. */
export class ApiError extends Error {
  constructor(message: string, readonly status: number, readonly kind?: string) {
    super(message);
  }
}

function refusal(res: Response, parsed: unknown): ApiError {
  const body = parsed as { error?: string; kind?: string } | null;
  return new ApiError(body?.error ?? `${res.status} ${res.statusText}`, res.status, body?.kind);
}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  if (syntheticMailCalendarEnabled() && path.startsWith("/mail-calendar/")) {
    try {
      const result = syntheticMailCalendarRequest(path, init);
      if (result !== undefined) return result as T;
      throw new ApiError("This mail and calendar request is not available in Synthetic demo mode", 403, "synthetic_demo_read_only");
    } catch (error) {
      const refusal = error as { message?: string; status?: number; kind?: string };
      throw new ApiError(refusal.message ?? "Synthetic demo request refused", refusal.status ?? 403, refusal.kind ?? "synthetic_demo_read_only");
    }
  }
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
    throw refusal(res, parsed);
  }
  return parsed as T;
}

export type MailCalendarProvider = "google" | "microsoft" | "apple_icloud";
export type MailCalendarAccountStatus = "pending" | "connected" | "connected_unverified" | "reauthentication_required";
export type MailCalendarCapability = "mail_read" | "mail_prepare" | "mail_send" | "calendar_free_busy" | "calendar_read" | "calendar_write";
export interface MailCalendarAccount {
  id: string;
  provider: MailCalendarProvider;
  status: MailCalendarAccountStatus;
  identity_masked: string | null;
  auth_method?: "oauth" | "app_password" | null;
  credential_available: boolean;
  superseded_by_active_link: boolean;
  capabilities: MailCalendarCapability[];
  connected_at: string;
  access_token_expires_at: string | null;
  refresh_token_available: boolean;
  revoked_at: string | null;
}

/** Notify open mail/calendar views after an account-level change completes. */
export function notifyMailCalendarChanged(): void {
  if (typeof window !== "undefined") window.dispatchEvent(new Event("vak:mail-calendar-changed"));
}

export async function listMailCalendarAccounts(agentId: string): Promise<{ accounts: MailCalendarAccount[] }> {
  return req(`/mail-calendar/accounts?agent_id=${encodeURIComponent(agentId)}`);
}

export async function beginMailCalendarOAuth(agentId: string, provider: MailCalendarProvider, capabilities: MailCalendarCapability[]): Promise<{ authorization_url: string }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/oauth`, {
    method: "POST",
    body: JSON.stringify({ provider, capabilities }),
  });
}

export async function connectIcloudAccount(agentId: string, email: string, appSpecificPassword: string, capabilities: MailCalendarCapability[]): Promise<{ connected: boolean }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/icloud`, {
    method: "POST",
    body: JSON.stringify({ email, app_specific_password: appSpecificPassword, capabilities }),
  });
}

export async function connectGoogleAppPassword(agentId: string, email: string, appPassword: string): Promise<{ connected: boolean }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/google-app-password`, {
    method: "POST",
    body: JSON.stringify({ email, app_specific_password: appPassword, capabilities: ["mail_read"] }),
  });
}

export async function connectMicrosoftAppPassword(agentId: string, email: string, appPassword: string): Promise<{ connected: boolean }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/microsoft-app-password`, {
    method: "POST",
    body: JSON.stringify({ email, app_specific_password: appPassword, capabilities: ["mail_read"] }),
  });
}

export async function refreshMailCalendarAccount(agentId: string, accountId: string): Promise<{ account: MailCalendarAccount }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/${encodeURIComponent(accountId)}/refresh`, { method: "POST", body: "{}" });
}

export interface MailCalendarMailPreview {
  provider_id: string;
  thread_id: string | null;
  from: string | null;
  reply_to?: string | null;
  to: string | null;
  cc: string | null;
  subject: string;
  received_at: string | null;
  preview: string;
  body_text: string | null;
  body_status?: "available" | "no_plain_text" | "unavailable";
  has_attachments: boolean;
  attachments?: MailCalendarAttachmentPreview[];
}
export interface MailCalendarThread { provider_id: string; messages: MailCalendarMailPreview[]; next_cursor?: string | null }
export interface MailCalendarAttachmentPreview {
  provider_id: string;
  filename: string;
  mime_type: string | null;
  size_bytes: number;
  previewable: boolean;
}
export interface MailCalendarFolder { provider_id: string; name: string }
export interface MailCalendarEventPreview {
  provider_id: string;
  account_id?: string;
  account_name?: string;
  version: string | null;
  title: string;
  starts_at: string | null;
  ends_at: string | null;
  starts_on: string | null;
  ends_on: string | null;
  all_day: boolean;
  location: string | null;
  description: string | null;
  attendee_count: number;
  recurring: boolean;
  private: boolean;
  can_cancel?: boolean;
}
export interface MailCalendarBusySlot { starts_at: string; ends_at: string }
export interface MailCalendarRoutineRun {
  run_id: string;
  routine_id: string;
  account_id: string;
  session_id: string | null;
  trigger: "manual" | "scheduled";
  status: "running" | "complete" | "failed" | "no_changes" | "interrupted";
  started_at: string;
  finished_at: string | null;
  items_returned: number;
}

export function listMailCalendarRoutineRuns(agentId: string, routineId: string): Promise<{ runs: MailCalendarRoutineRun[] }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/routines/${encodeURIComponent(routineId)}/history`);
}

export function listMailCalendarFolders(agentId: string, accountId: string): Promise<{ folders: MailCalendarFolder[] }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/${encodeURIComponent(accountId)}/mail-folders`);
}
export function previewMailCalendarMail(agentId: string, accountId: string, limit = 10, query?: string, folderId?: string, cursor?: string): Promise<{ messages: MailCalendarMailPreview[]; next_cursor?: string | null }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/${encodeURIComponent(accountId)}/mail-preview`, {
    method: "POST", body: JSON.stringify({ limit, ...(query?.trim() ? { query: query.trim() } : {}), ...(folderId ? { folder_id: folderId } : {}), ...(cursor ? { cursor } : {}) }),
  });
}
export function previewMailCalendarMessage(agentId: string, accountId: string, providerId: string): Promise<{ provider_id: string; body_text: string | null; body_status: "available" | "no_plain_text" }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/${encodeURIComponent(accountId)}/message-preview`, {
    method: "POST", body: JSON.stringify({ provider_id: providerId }),
  });
}
export function previewMailCalendarThread(agentId: string, accountId: string, threadId: string, cursor?: string): Promise<MailCalendarThread> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/${encodeURIComponent(accountId)}/thread-preview`, {
    method: "POST", body: JSON.stringify({ thread_id: threadId, ...(cursor ? { cursor } : {}) }),
  });
}
export function previewMailCalendarAttachment(agentId: string, accountId: string, messageId: string, attachmentId: string): Promise<{ filename: string; mime_type: string | null; size_bytes: number; text: string }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/${encodeURIComponent(accountId)}/attachment-preview`, {
    method: "POST", body: JSON.stringify({ message_id: messageId, attachment_id: attachmentId }),
  });
}
export interface MailCalendarSource { provider_id: string; name: string; primary: boolean }
export function listMailCalendarSources(agentId: string, accountId: string): Promise<{ sources: MailCalendarSource[] }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/${encodeURIComponent(accountId)}/calendar-sources`);
}
export function previewMailCalendarEvents(agentId: string, accountId: string, from: string, to: string, limit = 50, calendarId?: string, cursor?: string): Promise<{ events: MailCalendarEventPreview[]; next_cursor?: string | null }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/${encodeURIComponent(accountId)}/calendar-preview`, {
    method: "POST", body: JSON.stringify({ from, to, limit, ...(calendarId ? { calendar_id: calendarId } : {}), ...(cursor ? { cursor } : {}) }),
  });
}
export function previewMailCalendarFreeBusy(agentId: string, accountId: string, from: string, to: string): Promise<{ busy: MailCalendarBusySlot[] }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/${encodeURIComponent(accountId)}/free-busy-preview`, {
    method: "POST", body: JSON.stringify({ from, to }),
  });
}

export type MailCalendarDraftAction =
  | { kind: "send_mail"; draft: { from_alias: string | null; to: Array<{ address: string; display_name: string | null }>; cc: Array<{ address: string; display_name: string | null }>; bcc: Array<{ address: string; display_name: string | null }>; subject: string; body_text: string; attachment_refs: string[]; reply_to_message_id: string | null; reply_to_thread_id: string | null } }
  | { kind: "create_event"; draft: { title: string; description: string; location: string | null; starts_at: string; ends_at: string; time_zone: string; all_day: boolean; attendee_addresses: Array<{ address: string; display_name: string | null }>; recurrence: string | null; occurrence_id: string | null } }
  | { kind: "update_event"; event_id: string; source_version: string; draft: { title: string; description: string; location: string | null; starts_at: string; ends_at: string; time_zone: string; all_day: boolean; attendee_addresses: Array<{ address: string; display_name: string | null }>; recurrence: string | null; occurrence_id: string | null } }
  | { kind: "cancel_event"; event_id: string; source_version: string; occurrence_id: string | null; whole_series: boolean };
export interface MailCalendarCandidate {
  id: string;
  account_id: string;
  agent_id: string;
  audience_id: string;
  source_refs: Array<{ item_id: string; version: string | null; label: string | null }>;
  action: MailCalendarDraftAction;
  revision: number;
  created_at: string;
  candidate_digest?: string;
  action_state?: "prepared" | "awaiting_approval" | "dispatching" | "provider_accepted" | "confirmed" | "failed" | "unknown" | "cancelled" | "expired";
}
export interface MailCalendarReviewContext {
  candidate_id: string;
  revision: number;
  sender: string;
  source_from: string | null;
  source_reply_to: string | null;
}
export function listMailCalendarCandidates(agentId: string): Promise<{ candidates: MailCalendarCandidate[] }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/candidates`);
}
export function getMailCalendarReviewContext(agentId: string, candidateId: string, expectedRevision: number, candidateDigest: string): Promise<MailCalendarReviewContext> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/candidates/${encodeURIComponent(candidateId)}/review-context`, {
    method: "POST", body: JSON.stringify({ expected_revision: expectedRevision, candidate_digest: candidateDigest }),
  });
}
export function saveMailCalendarCandidate(agentId: string, payload: { account_id: string; candidate_id?: string; expected_revision?: number; source_refs?: MailCalendarCandidate["source_refs"]; action: MailCalendarDraftAction }): Promise<{ candidate: MailCalendarCandidate }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/candidates`, { method: "POST", body: JSON.stringify(payload) });
}
export function deleteMailCalendarCandidate(agentId: string, candidateId: string, expectedRevision: number): Promise<{ deleted: boolean }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/candidates/${encodeURIComponent(candidateId)}`, { method: "DELETE", body: JSON.stringify({ expected_revision: expectedRevision }) });
}
export function sendMailCalendarCandidate(agentId: string, candidateId: string, expectedRevision: number, candidateDigest: string): Promise<{ receipt?: { state: "provider_accepted" | "failed" | "unknown" | "dispatching"; provider_item_id?: string | null; detail_code?: string | null }; state?: "dispatching" }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/candidates/${encodeURIComponent(candidateId)}/send`, {
    method: "POST",
    body: JSON.stringify({ expected_revision: expectedRevision, candidate_digest: candidateDigest, confirm: true }),
  });
}
export function createMailCalendarEventCandidate(agentId: string, candidateId: string, expectedRevision: number, candidateDigest: string): Promise<{ receipt?: { state: "provider_accepted" | "failed" | "unknown" | "dispatching"; provider_item_id?: string | null; detail_code?: string | null }; state?: "dispatching" }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/candidates/${encodeURIComponent(candidateId)}/create-event`, {
    method: "POST",
    body: JSON.stringify({ expected_revision: expectedRevision, candidate_digest: candidateDigest, confirm: true }),
  });
}
export function updateMailCalendarEventCandidate(agentId: string, candidateId: string, expectedRevision: number, candidateDigest: string): Promise<{ receipt?: { state: "provider_accepted" | "failed" | "unknown" | "dispatching"; provider_item_id?: string | null; detail_code?: string | null }; state?: "dispatching" }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/candidates/${encodeURIComponent(candidateId)}/update-event`, {
    method: "POST",
    body: JSON.stringify({ expected_revision: expectedRevision, candidate_digest: candidateDigest, confirm: true }),
  });
}
export function cancelMailCalendarEventCandidate(agentId: string, candidateId: string, expectedRevision: number, candidateDigest: string): Promise<{ receipt?: { state: "provider_accepted" | "failed" | "unknown" | "dispatching"; provider_item_id?: string | null; detail_code?: string | null }; state?: "dispatching" }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/candidates/${encodeURIComponent(candidateId)}/cancel-event`, {
    method: "POST",
    body: JSON.stringify({ expected_revision: expectedRevision, candidate_digest: candidateDigest, confirm: true }),
  });
}
export function reconcileMailCalendarEventCandidate(agentId: string, candidateId: string, expectedRevision: number, candidateDigest: string): Promise<{ matched: boolean; state?: string; receipt?: { state: string; provider_item_id?: string | null; detail_code?: string | null }; message?: string }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/candidates/${encodeURIComponent(candidateId)}/reconcile-event`, {
    method: "POST", body: JSON.stringify({ expected_revision: expectedRevision, candidate_digest: candidateDigest }),
  });
}
export function reconcileMailCalendarMailCandidate(agentId: string, candidateId: string, expectedRevision: number, candidateDigest: string): Promise<{ matched: boolean; state?: string; receipt?: { state: string; provider_item_id?: string | null; detail_code?: string | null }; message?: string }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/candidates/${encodeURIComponent(candidateId)}/reconcile-mail`, {
    method: "POST", body: JSON.stringify({ expected_revision: expectedRevision, candidate_digest: candidateDigest }),
  });
}

export async function disconnectMailCalendarAccount(agentId: string, accountId: string): Promise<{ disconnected: boolean; already_disconnected: boolean; provider_grant_revoked: boolean; provider_revocation: "confirmed" | "unsupported" | "unconfirmed" | "not_retried"; content_erased: boolean }> {
  return req(`/mail-calendar/accounts/${encodeURIComponent(agentId)}/${encodeURIComponent(accountId)}/disconnect`, { method: "POST", body: "{}" });
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
 * Open the stream hub's `EventSource` (streamHub.ts) with whatever this
 * host's auth channel is. Nothing else in the client opens one.
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

setStreamOpener(eventSource);

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
  path?: string;
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
  character: "vak" | "mira" | "moss" | "nori" | "pip" | "lumi" | "tavi" | "beni";
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

export function openAgent(id: string, createNew = false): Promise<{session_id: string; cwd: string; agent: Agent}> {
  return req(`/agents/${encodeURIComponent(id)}/open`, {method: "POST", body: JSON.stringify({create_new: createNew})});
}

export interface AgentTemplate {
  template_id: string;
  domain: string;
  name: string;
  description: string;
  character: "vak" | "mira" | "moss" | "nori" | "pip" | "lumi" | "tavi" | "beni";
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

export interface PreviewOrigin {
  id: string;
  url: string;
  origin: string;
}

/**
 * Opens a preview origin for a page and the files it loads. `null` when the
 * server cannot offer one because it is not on this computer (409); the page
 * is then shown as a single document.
 */
export async function openPreview(source: PreviewSource): Promise<PreviewOrigin | null> {
  try {
    return await req<PreviewOrigin>("/previews", { method: "POST", body: JSON.stringify(source) });
  } catch (cause) {
    if (cause instanceof ApiError && cause.status === 409) return null;
    throw cause;
  }
}

/** Ends a preview origin. Closing one that is already gone is not a failure. */
export async function closePreview(id: string): Promise<void> {
  try {
    await authFetch(`/previews/${encodeURIComponent(id)}`, { method: "DELETE" });
  } catch {
    /* The server stops it by age. */
  }
}

/** The name this client reaches the server by. */
export function backendHostname(): string {
  try {
    return new URL(backendUrl() || window.location.href).hostname;
  } catch {
    return window.location.hostname;
  }
}

export interface RunAdmission {
  request_id: string | null;
  state: "started" | "queued" | "duplicate";
}

/** A file saved in the workspace inbox (docs/design/72, "File in"). */
export interface InboxFile {
  /** Workspace-relative, under `inbox/`. */
  path: string;
  name: string;
  bytes: number;
}

/** What a message carries besides its text: images the model sees, and
 *  files it is told about by path (their bytes never reach it). */
export interface Attachments {
  images: { mime: string; data: string }[];
  files: InboxFile[];
}

/** Saves a dropped or picked file to the workspace inbox. */
export async function uploadToInbox(file: File): Promise<InboxFile> {
  const response = await authFetch(`/fs/inbox?name=${encodeURIComponent(file.name)}`, {
    method: "POST",
    headers: { "Content-Type": "application/octet-stream" },
    body: file,
  });
  if (!response.ok) {
    const detail = await response.json().catch(() => null) as { error?: string } | null;
    throw new Error(detail?.error ?? `${response.status} ${response.statusText}`);
  }
  return response.json() as Promise<InboxFile>;
}

export function runPrompt(
  id: string,
  prompt: string,
  goal?: { objective: string; criteria: string[] },
  attachments?: Attachments,
  requestId?: string,
  routing?: RoutingEnvelope,
): Promise<RunAdmission> {
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
      attachments: attachments?.images ?? [],
      files: attachments?.files.map((file) => file.path) ?? [],
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
  /** `"steering_queued"` when a run is already busy; `"started"` when it
   *  lands on an idle session and starts a run itself. */
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

export interface PendingApproval {
  id: string;
  tool: string;
  args_json: string;
  reason: string;
  requested_at: string;
}

export function pendingApprovals(id: string): Promise<{ approvals: PendingApproval[] }> {
  return req(`/sessions/${encodeURIComponent(id)}/approvals`);
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

export function listProviders(): Promise<import("./types").ProvidersResponse> {
  return req("/providers");
}

/** The name people know a provider by, from the server's listing; an id the
 * listing does not hold shows as itself. */
export function providerLabel(list: readonly import("./types").ProviderInfo[] | undefined, id: string): string {
  return list?.find((p) => p.name === id)?.label ?? id;
}

/** Live model list for one provider, discovered from its API. */
export interface DiscoveredModels {
  provider: string;
  models: string[];
  capabilities?: Record<string, string[]>;
}

export function discoverModels(provider: string, agent?: string): Promise<DiscoveredModels> {
  return req(withAgent(`/providers/${encodeURIComponent(provider)}/models`, agent));
}

export function putProviderKey(
  provider: string,
  key: string,
  scope: "user" | "workspace" = "user",
  agent?: string,
): Promise<{ provider: string; env_var: string; configured: boolean }> {
  return req("/config/key", {
    method: "PUT",
    body: JSON.stringify({ provider, key, scope, agent }),
  });
}

/** Revoke a provider key stored on this device. */
export function removeProviderKey(
  provider: string,
  scope: "user" | "workspace" = "user",
  agent?: string,
): Promise<{ provider: string; env_var: string; configured: boolean; shadowed_by_env: boolean }> {
  return req("/config/key", {
    method: "DELETE",
    body: JSON.stringify({ provider, scope, agent }),
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
  memory_search_enabled?: boolean;
  memory_write_enabled?: boolean;
  memory_reflection?: boolean;
  memory_skill_proposals?: boolean;
  theme?: string;
  inherit_mcp?: boolean;
  inherit_hooks?: boolean;
  inherit_skills?: boolean;
  inherit_commands?: boolean;
  inherit_plugins?: boolean;
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

export function getGlobalRoute(): Promise<{ provider?: string | null; model?: string | null }> {
  return req("/config/global");
}

export function getPrivacyConfigLayer(scope: "user" | "workspace", agent?: string): Promise<{
  permission_mode?: string | null;
  approval_mode?: string | null;
  permissions?: { allow?: string[]; ask?: string[]; deny?: string[] };
  memory?: { search_enabled?: boolean | null; write_enabled?: boolean | null; reflection?: boolean | null; skill_proposals?: boolean | null };
}> {
  const path = scope === "user" ? "/config/global" : "/config/workspace";
  return req(withAgent(path, agent));
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

/** Moves an archived task to the trash, which hides it everywhere,
 * search included. Nothing is erased; `restoreFromTrash` brings it back. */
export function trashSession(id: string): Promise<{ trashed: string }> {
  return req(`/sessions/${id}`, { method: "DELETE" });
}

export function trashAllArchived(): Promise<{ trashed: number }> {
  return req("/sessions/archived", { method: "DELETE" });
}

export function listTrash(): Promise<{ sessions: SessionSummary[] }> {
  return req("/sessions?trash=true");
}

export function restoreFromTrash(id: string): Promise<{ restored: string }> {
  return req(`/sessions/${id}/restore`, { method: "POST" });
}

export function listSkills(agent?: string): Promise<{
  skills: DiscoveredSkill[];
}> {
  return req(withAgent("/skills", agent));
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

export function registerPluginSource(path: string, label: string, scope: "user" | "workspace", signature?: { key_id: string; public_key: string; signature: string }, agent?: string): Promise<MarketplaceSource> {
  return req("/plugins/sources", { method: "POST", body: JSON.stringify({ path, label, scope, trust: "manual-review", agent, ...(signature ?? {}) }) });
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

export function installCatalogPlugin(entry: MarketplaceEntry, scope: "workspace" | "user", agent?: string, update = false): Promise<InstalledPlugin> {
  return req("/plugins/catalog/install", { method: "POST", body: JSON.stringify({ source_id: entry.source_id, source_scope: entry.source_scope, name: entry.name, scope, agent, update }) });
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

export function readExecutionArtifact(sessionId: string, executionId: string, path: string): Promise<FileResponse> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/sandbox/executions/${encodeURIComponent(executionId)}/artifact?path=${encodeURIComponent(path)}`);
}

export async function readExecutionArtifactRaw(sessionId: string, executionId: string, path: string): Promise<string> {
  const response = await authFetch(`/sessions/${encodeURIComponent(sessionId)}/sandbox/executions/${encodeURIComponent(executionId)}/artifact/raw?path=${encodeURIComponent(path)}`);
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return URL.createObjectURL(await response.blob());
}

/** Authenticated raw bytes for browser-native artifact viewers/downloads. */
export async function readFileRaw(path: string): Promise<string> {
  const response = await authFetch(`/fs/file/raw?path=${encodeURIComponent(path)}`);
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return URL.createObjectURL(await response.blob());
}

/** A workspace file's bytes and type, for saving a copy. */
export async function readFileBytes(path: string): Promise<{ bytes: Uint8Array<ArrayBuffer>; mime: string }> {
  const response = await authFetch(`/fs/file/raw?path=${encodeURIComponent(path)}`);
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return {
    bytes: new Uint8Array(await response.arrayBuffer()),
    mime: response.headers.get("Content-Type") ?? "application/octet-stream",
  };
}

export async function readExecutionArtifactBytes(sessionId: string, executionId: string, path: string): Promise<{ bytes: Uint8Array<ArrayBuffer>; mime: string }> {
  const response = await authFetch(`/sessions/${encodeURIComponent(sessionId)}/sandbox/executions/${encodeURIComponent(executionId)}/artifact/raw?path=${encodeURIComponent(path)}`);
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return { bytes: new Uint8Array(await response.arrayBuffer()), mime: response.headers.get("Content-Type") ?? "application/octet-stream" };
}

export async function readSandboxCandidateFileBytes(sessionId: string, candidateId: string, path: string): Promise<{ bytes: Uint8Array<ArrayBuffer>; mime: string }> {
  const response = await authFetch(`/sessions/${encodeURIComponent(sessionId)}/sandbox/candidates/${encodeURIComponent(candidateId)}/files/raw?path=${encodeURIComponent(path)}`);
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return { bytes: new Uint8Array(await response.arrayBuffer()), mime: response.headers.get("Content-Type") ?? "application/octet-stream" };
}

export function writeFile(path: string, content: string): Promise<unknown> {
  return req("/fs/file", { method: "PUT", body: JSON.stringify({ path, content }) });
}

export type WorkspaceCheckPlan = { id: string; label: string; command: string };
export type SandboxCandidate = { candidate_id: string; source_root: string; destination_root: string; files: Array<{ path: string; candidate_hash: string; base_hash?: string; bytes: number; operation?: "Upsert" | "Delete" }>; target_checks?: Array<{ verifier: string; path: string }>; workspace_checks?: WorkspaceCheckPlan[] };
export type SandboxCandidateRecord = { record_id: string; session_id: string; turn_id: string; result_id: string; execution_id: string; environment_id: string; candidate_digest: string; candidate: SandboxCandidate; verified: boolean; draft_checks?: Array<{ verifier: string; path: string; status: string; evidence: string }>; updated_at: string; parent_candidate_id?: string; revision_session_id?: string; narrowed?: { path: string; keep: string[] } };
export type SandboxPromotionRecord = { record_id: string; session_id: string; result_id: string; candidate_digest: string; candidate_id: string; receipt: { verification?: Array<{ path: string; status: string; evidence: string }>; deleted?: string[]; integration?: { applied_state_digest: string; workspace_state_status: string; target_checks_status: string; evidence: string; target_checks?: Array<{ verifier: string; path: string; status: string; evidence: string }> } }; workspace_checks?: WorkspaceCheckPlan[]; updated_at: string };
export type SandboxPromotionUndoRecord = { record_id: string; session_id: string; candidate_id: string; receipt: { restored: string[]; verification: Array<{ path: string; status: string; evidence: string }> }; updated_at: string };
export type SandboxWorkspaceCheckRecord = { record_id: string; session_id: string; candidate_id: string; applied_state_digest: string; check: WorkspaceCheckPlan; status: "passed" | "failed"; evidence: string; updated_at: string };
export type SandboxCandidateRevisionRecord = { record_id: string; revision_id: string; session_id: string; parent_candidate_id: string; comment_id: string; child_session_id: string; status: "Running" | "Completed" | "Failed"; candidate_id?: string; detail?: string; updated_at: string };
export type SandboxPreviewPreparationRecord = { record_id: string; session_id: string; result_id: string; candidate_id: string; candidate_digest: string; environment_id: string; state: "Planned" | "Preparing" | "Ready" | "Running" | "Stopped" | "Failed" | "Expired"; command: string; evidence: string; updated_at: string };
export type SandboxRecord =
  | { kind: "Candidate"; record: SandboxCandidateRecord }
  | { kind: "Promotion"; record: SandboxPromotionRecord }
  | { kind: "PromotionUndo"; record: SandboxPromotionUndoRecord }
  | { kind: "WorkspaceCheck"; record: SandboxWorkspaceCheckRecord }
  | { kind: "Environment"; record: unknown }
  | { kind: "PreviewPreparation"; record: SandboxPreviewPreparationRecord }
  | { kind: "CandidateRevision"; record: SandboxCandidateRevisionRecord };

export function readSandboxCandidateFile(sessionId: string, candidateId: string, path: string): Promise<FileResponse> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/sandbox/candidates/${encodeURIComponent(candidateId)}/files?path=${encodeURIComponent(path)}`);
}

export type OfficeChange = {
  section: string;
  anchor: string;
  kind: "added" | "removed" | "changed" | "moved";
  before?: string | null;
  after?: string | null;
};

/** One change a person can keep or leave out: one edit the Agent made, or
 *  one cell of it. `requires` names choices it cannot be kept without. */
export type OfficeChoice = {
  id: string;
  label: string;
  requires: string[];
  changes: OfficeChange[];
};

/** What accepting does beyond the visible changes: to digital signatures
 *  and sensitivity labels. `warning` when it removes or weakens one. */
export type OfficeImpact = { kind: "signature" | "label" | "recalculation" | "revisions"; message: string; warning: boolean };

export type OfficeReview = {
  path: string;
  compared_with: "workspace" | "nothing (new file)";
  summary: string[];
  changes: OfficeChange[];
  flags: string[];
  impact?: OfficeImpact[];
  /** Present when the draft's recorded edits reproduce it exactly. */
  choices?: OfficeChoice[];
  /** Why the draft can only be accepted or rejected whole. */
  choices_unavailable?: string;
  /** Set on a version that keeps some of an earlier draft's changes. */
  narrowed_from?: { candidate_id: string; keep: string[] };
};

/** The semantic change list for an Office file in a draft, and the changes
 *  a person can choose among, computed by the server in its document
 *  worker (docs/design/72, P3). */
export function readSandboxCandidateOfficeReview(sessionId: string, candidateId: string, path: string): Promise<OfficeReview> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/sandbox/candidates/${encodeURIComponent(candidateId)}/office-review?path=${encodeURIComponent(path)}`);
}

/** A new version of the draft that keeps only `keep` (choice ids). The full
 *  draft stays; the new version is reviewed and accepted like any other. */
export async function narrowSandboxCandidateOffice(sessionId: string, candidateId: string, path: string, keep: string[]): Promise<SandboxCandidateRecord> {
  const response = await req<{ kind: "Candidate"; record: SandboxCandidateRecord }>(`/sessions/${encodeURIComponent(sessionId)}/sandbox/candidates/${encodeURIComponent(candidateId)}/office-narrow`, {
    method: "POST",
    body: JSON.stringify({ path, keep }),
  });
  return response.record;
}

/** One addressable piece of an Office file, as the reader projects it:
 *  `anchor` is what a citation, an op or a comment names. `cells` holds a
 *  sheet row's cells as [address, shown value]. */
export type OfficeUnit = {
  anchor: string;
  kind: "heading" | "paragraph" | "table_row" | "comment" | "sheet_row" | "defined_name" | "slide" | "shape" | "notes" | "page" | "image";
  level: number;
  text: string;
  labels: string[];
  cells?: [string, string][];
  /** Word/PowerPoint table row cells, preserving cell text and anchors. */
  row_cells?: [string, string][][];
};

export type OfficeMediaPreview = {
  alt_text: string;
  object_id: string;
  mime_type: "image/png" | "image/jpeg";
  data_url: string;
  cell?: string;
  offset_x_px?: number;
  offset_y_px?: number;
  end_cell?: string;
  end_offset_x_px?: number;
  end_offset_y_px?: number;
  width_px?: number;
  height_px?: number;
};

export type OfficeCellStyle = {
  fill_color?: string;
  font_color?: string;
  bold: boolean;
  italic: boolean;
  number_format?: string;
  horizontal_alignment?: string;
  vertical_alignment?: string;
  wrap_text?: boolean;
};

export type OfficeTableStyleRange = {
  sheet_anchor: string;
  range: string;
  style_name: string | null;
  show_row_stripes: boolean;
  show_column_stripes: boolean;
};

export type OfficeMergedCellRange = { sheet_anchor: string; range: string };

export type OfficeOutlineEntry = { anchor: string; title: string; level: number; first_unit: number; units: number };

/** One page of what the Canvas draws of an Office file (docs/design/72, P4). */
export type OfficeProjection = {
  path: string;
  sha256: string;
  vocabulary: "word" | "excel" | "power_point" | "visio" | "pdf";
  kind: string;
  extension: string;
  macro_enabled: boolean;
  strict: boolean;
  title: string | null;
  stats: [string, number][];
  flags: string[];
  sensitivity_labels: string[];
  outline: OfficeOutlineEntry[];
  total_units: number;
  from: number;
  next: number | null;
  units: OfficeUnit[];
  /** Package-scoped visual image IDs keyed by the stable public anchor. */
  image_object_ids?: Record<string, string>;
  /** Bounded previews returned by the document worker for the Canvas. */
  media?: OfficeMediaPreview[];
  /** Presentation-only cell styles; excluded from the RAG extraction view. */
  cell_styles?: Record<string, OfficeCellStyle>;
  table_styles?: OfficeTableStyleRange[];
  merged_ranges?: OfficeMergedCellRange[];
  /** Canvas-formatted values; `units` retains the stored values for RAG. */
  display_values?: Record<string, string>;
  sheet_geometry?: { default_column_widths: Record<string, number>; default_row_heights: Record<string, number>; column_widths: Record<string, number>; row_heights: Record<string, number>; merged_ranges: OfficeMergedCellRange[] };
  not_read: string[];
  /** The unit a cited anchor named, when the page was asked for `at` one. */
  focus?: string;
};

export type OfficeStructure = {
  path: string;
  sha256: string;
  main_part: string;
  parts: { name: string; content_type: string | null; size: number }[];
  relationships: { source: string; id: string; kind: string; target: string; external: boolean }[];
  untyped_parts: string[];
};

/** Where an Office file lives: the workspace, or a saved candidate. */
export type OfficeSource = { path: string; sessionId?: string; candidateId?: string; executionId?: string; token?: string };

export type OfficeEditOp =
  | { op: "replace_paragraph_text"; anchor: string; text: string }
  | { op: "set_cells"; sheet: string; cells: Record<string, string | number | boolean> }
  | { op: "set_placeholder_text"; anchor: string; text: string };

export type OfficeWorkspaceRevision = { candidate_id: string; parent_candidate_id: string; merge_parent_candidate_id?: string | null; branch_id: string; author_id: string; author_name: string; created_at: string; ops: OfficeEditOp[] };
export type OfficeWorkspaceBranch = { branch_id: string; name: string; base_candidate_id: string; head_candidate_id: string; shared: boolean; archived: boolean };
export type OfficeWorkspace = { schema: number; room_id: string; session_id: string; path: string; created_at: string; branches: OfficeWorkspaceBranch[]; revisions: OfficeWorkspaceRevision[] };

async function officeReq<T>(source: OfficeSource, url: string, init?: RequestInit): Promise<T> {
  if (!source.token) return req<T>(url, init);
  const headers: Record<string, string> = { "Content-Type": "application/json", ...((init?.headers as Record<string, string>) ?? {}), Authorization: `Bearer ${source.token}` };
  const response = await fetch(url, { ...init, headers, credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer" });
  const text = await response.text();
  let parsed: { error?: string } = {};
  try { parsed = text ? JSON.parse(text) as { error?: string } : {}; } catch { parsed = {}; }
  if (!response.ok) throw new ApiError(parsed.error ?? (response.status === 401 || response.status === 403 ? "This invitation has expired or no longer permits this action." : `Could not open the Office workspace (${response.status}).`), response.status);
  return (text ? JSON.parse(text) : null) as T;
}

function officeUrl(source: OfficeSource, query: string): string {
  const path = `path=${encodeURIComponent(source.path)}&${query}`;
  if (source.candidateId && source.sessionId)
    return `/sessions/${encodeURIComponent(source.sessionId)}/sandbox/candidates/${encodeURIComponent(source.candidateId)}/office?${path}`;
  if (source.executionId && source.sessionId)
    return `/sessions/${encodeURIComponent(source.sessionId)}/sandbox/executions/${encodeURIComponent(source.executionId)}/artifact/office?${path}`;
  return `/fs/office?${path}`;
}

/** A page of an Office file: from unit `from`, or, given `at`, from the unit
 *  that cited anchor names (`focus` is then that unit's anchor). */
export function readOfficeProjection(source: OfficeSource, from = 0, at?: string): Promise<OfficeProjection> {
  return officeReq(source, officeUrl(source, at ? `at=${encodeURIComponent(at)}` : `from=${from}`));
}

export function readOfficeStructure(source: OfficeSource): Promise<OfficeStructure> {
  return officeReq(source, officeUrl(source, "view=structure"));
}

/** What a file card says about an Office file: its kind, counts and flags,
 *  with no content (`units` is empty). A card is drawn again on every
 *  timeline update, so one answer per path serves every card for a short
 *  while instead of one worker read per render. `null` when it cannot be
 *  read. */
export function readOfficeFacts(source: OfficeSource): Promise<OfficeProjection | null> {
  const key = `${source.sessionId ?? ""}:${source.candidateId ?? ""}:${source.executionId ?? ""}:${source.path}`;
  const cached = officeFactsCache.get(key);
  if (cached && Date.now() - cached.at < OFFICE_FACTS_TTL_MS) return cached.facts;
  const facts = req<OfficeProjection>(officeUrl(source, "view=facts")).catch(() => null);
  officeFactsCache.set(key, { at: Date.now(), facts });
  return facts;
}

export function createOfficeWorkspace(sessionId: string, candidateId: string, path: string): Promise<OfficeWorkspace> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/office-workspaces`, { method: "POST", body: JSON.stringify({ candidate_id: candidateId, path }) });
}

export function listOfficeWorkspaces(sessionId: string): Promise<{ workspaces: OfficeWorkspace[] }> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/office-workspaces`);
}

export function getOfficeWorkspaces(source: OfficeSource): Promise<{ workspaces: OfficeWorkspace[] }> {
  if (!source.sessionId) return Promise.resolve({ workspaces: [] });
  return officeReq(source, `/sessions/${encodeURIComponent(source.sessionId)}/office-workspaces`);
}

export function mutateOfficeWorkspace(sessionId: string, roomId: string, action: unknown): Promise<{ workspace: OfficeWorkspace; candidate?: SandboxCandidateRecord; error?: string }> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/office-workspaces/${encodeURIComponent(roomId)}`, { method: "POST", body: JSON.stringify(action) });
}

export function mutateOfficeWorkspaceFromSource(source: OfficeSource, roomId: string, action: unknown): Promise<{ workspace?: OfficeWorkspace; workspaces?: OfficeWorkspace[]; candidate?: SandboxCandidateRecord }> {
  if (!source.sessionId) return Promise.reject(new Error("Office workspace has no conversation."));
  return officeReq(source, `/sessions/${encodeURIComponent(source.sessionId)}/office-workspaces/${encodeURIComponent(roomId)}`, { method: "POST", body: JSON.stringify(action) });
}

export function setOfficeFocus(source: OfficeSource, roomId: string, anchor: string | null, active = true): Promise<{ ok: boolean }> {
  if (!source.sessionId) return Promise.resolve({ ok: false });
  return officeReq(source, `/sessions/${encodeURIComponent(source.sessionId)}/office-workspaces/${encodeURIComponent(roomId)}/presence`, { method: "POST", body: JSON.stringify({ anchor, active }) });
}

const OFFICE_FACTS_TTL_MS = 30_000;
const officeFactsCache = new Map<string, { at: number; facts: Promise<OfficeProjection | null> }>();

export async function readSandboxCandidateFileRaw(sessionId: string, candidateId: string, path: string): Promise<string> {
  const response = await authFetch(`/sessions/${encodeURIComponent(sessionId)}/sandbox/candidates/${encodeURIComponent(candidateId)}/files/raw?path=${encodeURIComponent(path)}`);
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return URL.createObjectURL(await response.blob());
}

/** `anchor.anchor` points into an Office file (`Budget!B4`, `p:1A2B3C4D`,
 *  `slide:256/shape:3`); line numbers are for text files only. */
export function commentOnSandboxCandidate(sessionId: string, candidateId: string, text: string, anchor?: { path?: string; lineStart?: number; lineEnd?: number; anchor?: string }): Promise<{ comment_id: string; intervention: boolean }> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/sandbox/candidates/${encodeURIComponent(candidateId)}/comments`, {
    method: "POST",
    body: JSON.stringify({
      text,
      path: anchor?.path,
      line_start: anchor?.lineStart,
      line_end: anchor?.lineEnd,
      anchor: anchor?.anchor,
      request_id: typeof crypto !== "undefined" && typeof crypto.randomUUID === "function" ? crypto.randomUUID() : `${Date.now()}-${Math.random().toString(36).slice(2)}`,
    }),
  });
}

export type SandboxCandidateComment = { comment_id: string; actor_id: string; actor_name?: string; text: string; path?: string; line_start?: number; line_end?: number; anchor?: string; created_at?: string };

export function listSandboxCandidateComments(sessionId: string, candidateId: string): Promise<{ comments: SandboxCandidateComment[] }> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/sandbox/candidates/${encodeURIComponent(candidateId)}/comments`);
}

export function requestRevisionFromCandidateComment(sessionId: string, candidateId: string, commentId: string): Promise<InterventionReceipt> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/sandbox/candidates/${encodeURIComponent(candidateId)}/comments/${encodeURIComponent(commentId)}/request-revision`, { method: "POST" });
}

export function listSessionSandboxRecords(sessionId: string): Promise<{ records: SandboxRecord[] }> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/sandbox/records`);
}

export type CoworkingInvitation = {
  grant_id: string;
  principal_id: string;
  display_name: string;
  conversation_id: string;
  audience_id: string;
  capabilities: string[];
  created_at: string;
  expires_at: string;
  status: "active" | "expired" | "revoked";
  revoked_at?: string;
};

export type CoworkingParticipant = { principal_id: string; display_name: string; office_room_id?: string | null; office_anchor?: string | null };

export function coworkingPresence(sessionId: string): Promise<{ participants: CoworkingParticipant[] }> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/coworking/presence`);
}

export function listCoworkingInvitations(sessionId: string): Promise<{ invitations: CoworkingInvitation[] }> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/coworking/invitations`);
}

export function createCoworkingInvitation(sessionId: string, displayName: string, expiresInHours: number, canComment = false, canMessage = false, canEdit = false): Promise<{ invitation: CoworkingInvitation; token: string }> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/coworking/invitations`, {
    method: "POST",
    body: JSON.stringify({ display_name: displayName, expires_in_hours: expiresInHours, can_comment: canComment, can_message: canMessage, can_edit: canEdit }),
  });
}

export function delegateCoworkingApproval(sessionId: string, requestId: string, grantId: string): Promise<{ delegated_to: string }> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/coworking/approvals/${encodeURIComponent(requestId)}/delegate`, {
    method: "POST", body: JSON.stringify({ grant_id: grantId }),
  });
}

export function revokeCoworkingInvitation(sessionId: string, grantId: string): Promise<void> {
  return req(`/sessions/${encodeURIComponent(sessionId)}/coworking/invitations/${encodeURIComponent(grantId)}/revoke`, {
    method: "POST",
    body: "{}",
  });
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

export async function runSandboxWorkspaceCheck(sessionId: string, candidateId: string, checkId: string): Promise<SandboxWorkspaceCheckRecord> {
  const response = await req<{ kind: "WorkspaceCheck"; record: SandboxWorkspaceCheckRecord }>(`/sessions/${encodeURIComponent(sessionId)}/sandbox/promotions/${encodeURIComponent(candidateId)}/checks`, { method: "POST", body: JSON.stringify({ check_id: checkId }) });
  return response.record;
}

export async function undoSandboxPromotion(sessionId: string, candidateId: string): Promise<SandboxPromotionUndoRecord> {
  const response = await req<{ kind: "PromotionUndo"; record: SandboxPromotionUndoRecord }>(`/sessions/${encodeURIComponent(sessionId)}/sandbox/promotions/${encodeURIComponent(candidateId)}/undo`, { method: "POST" });
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
  opts?: { voiceName?: string; persona?: string; provider?: string; transcriptionModel?: string; synthesisModel?: string; sessionId?: string },
): Promise<Blob> {
  const voice_override =
    opts?.voiceName || opts?.persona || opts?.provider || opts?.transcriptionModel || opts?.synthesisModel
      ? {
          voice_name: opts?.voiceName || undefined,
          persona: opts?.persona || undefined,
          provider: opts?.provider || undefined,
          transcription_model: opts?.transcriptionModel || undefined,
          synthesis_model: opts?.synthesisModel || undefined,
        }
      : undefined;
  const res = await authFetch("/voice/speak", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text, voice_override, session_id: opts?.sessionId }),
  });
  if (!res.ok) {
    const text = await res.text();
    let parsed: unknown = null;
    try {
      parsed = text ? JSON.parse(text) : null;
    } catch {
      parsed = text;
    }
    throw refusal(res, parsed);
  }
  return res.blob();
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
  mail_calendar_scope?: {
    routine_id?: string;
    account_id: string;
    mail_folder_id?: string | null;
    calendar_source_id?: string | null;
    operations: Array<"recent_mail" | "mail_thread" | "calendar_events" | "free_busy">;
    max_items: number;
    watch_new_mail: boolean;
    read_commitments: boolean;
    calendar_event_trigger?: {
      boundary: "start" | "end";
      /** Positive means before the boundary; negative means after it. */
      offset_minutes: number;
      max_lateness_minutes: number;
    } | null;
  } | null;
  timezone?: string | null;
  deliver_to?: string | null;
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

export function getLaunch(id: string, candidateId?: string): Promise<{
  servers: {
    name: string;
    cmd: string;
    args: string[];
    port: number | null;
    running: boolean;
    available: boolean;
    availability: "ready" | "needs_setup" | "needs_preparation" | "port_in_use";
    unavailable_reason?: string;
  }[];
  error?: string;
}> {
  const query = candidateId ? `?candidate_id=${encodeURIComponent(candidateId)}` : "";
  return req(`/sessions/${id}/launch${query}`);
}

export function startLaunch(id: string, name: string, candidateId?: string): Promise<{ started: boolean; listening: boolean; error?: string }> {
  return req(`/sessions/${id}/launch/start`, { method: "POST", body: JSON.stringify({ name, candidate_id: candidateId }) });
}

export function prepareLaunch(id: string, name: string, candidateId: string): Promise<{ prepared: boolean; evidence?: string }> {
  return req(`/sessions/${id}/launch/prepare`, { method: "POST", body: JSON.stringify({ name, candidate_id: candidateId }) });
}

export function stopLaunch(id: string, name: string, candidateId?: string): Promise<unknown> {
  return req(`/sessions/${id}/launch/stop`, { method: "POST", body: JSON.stringify({ name, candidate_id: candidateId }) });
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
   *  budget_alert | digest | heartbeat | proposal_opened | routine_failed
   *  (snake_case). */
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
