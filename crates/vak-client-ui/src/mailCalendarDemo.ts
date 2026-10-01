/** Local, synthetic mail/calendar data. This module never contacts the API. */
const MODE_KEY = "vak.mail-calendar.synthetic-demo.v1";
const DRAFT_KEY = "vak.mail-calendar.synthetic-drafts.v1";

export function syntheticMailCalendarEnabled(): boolean {
  if (!canOfferSyntheticMailCalendar()) return false;
  try { return window.localStorage.getItem(MODE_KEY) === "on"; } catch { return false; }
}

export function setSyntheticMailCalendarEnabled(enabled: boolean): void {
  if (typeof window === "undefined") return;
  try {
    if (enabled) window.localStorage.setItem(MODE_KEY, "on");
    else window.localStorage.removeItem(MODE_KEY);
  } catch { /* The mode remains off when browser storage is unavailable. */ }
  window.dispatchEvent(new Event("vak:mail-calendar-changed"));
}

export function canOfferSyntheticMailCalendar(): boolean {
  return typeof window !== "undefined" && ["localhost", "127.0.0.1", "::1"].includes(window.location.hostname);
}

const providerNames = ["google", "microsoft", "apple_icloud"] as const;
const capabilities = ["mail_read", "mail_prepare", "calendar_free_busy", "calendar_read"];
function accounts(agentId: string) {
  return providerNames.flatMap((provider, p) => Array.from({ length: 3 }, (_, i) => ({
    id: `demo-${provider}-${i + 1}`, agent_id: agentId, provider, status: "connected", identity_masked: `demo-${i + 1}@example.test`, auth_method: "oauth",
    credential_available: true, superseded_by_active_link: false, capabilities: [...capabilities], connected_at: new Date().toISOString(),
    access_token_expires_at: null, refresh_token_available: false, revoked_at: null, demo: true,
    _group: p,
  })));
}
function drafts(agentId: string): any[] {
  try { return JSON.parse(window.localStorage.getItem(`${DRAFT_KEY}.${agentId}`) ?? "[]"); } catch { return []; }
}
function saveDrafts(agentId: string, items: any[]) {
  window.localStorage.setItem(`${DRAFT_KEY}.${agentId}`, JSON.stringify(items));
}
function dateFor(offset: number, hour = 9) {
  const d = new Date(); d.setDate(d.getDate() + offset); d.setHours(hour, 0, 0, 0); return d.toISOString();
}

/** Returns undefined when a path is not part of the mail/calendar API. */
export function syntheticMailCalendarRequest(path: string, init?: RequestInit): unknown | undefined {
  if (!syntheticMailCalendarEnabled() || !path.startsWith("/mail-calendar/")) return undefined;
  const url = new URL(path, "http://localhost");
  const parts = url.pathname.split("/").filter(Boolean).map(decodeURIComponent);
  const agentId = url.searchParams.get("agent_id") ?? parts[2] ?? "vak";
  const method = init?.method ?? "GET";
  const body = (() => { try { return JSON.parse(String(init?.body ?? "{}")); } catch { return {}; } })();
  const fail = (message: string): never => { throw Object.assign(new Error(message), { status: 403, kind: "synthetic_demo_read_only" }); };
  const accts = accounts(agentId);
  const account = accts.find((item) => item.id === parts[3]);
  if (parts[1] === "accounts" && parts.length === 2 && method === "GET") return { accounts: accts.map(({ _group, ...item }) => item) };
  if (parts[1] === "accounts" && parts.length === 4 && parts[3] === "candidates" && method === "GET") return { candidates: drafts(agentId) };
  if (parts[1] === "accounts" && parts.length === 4 && parts[3] === "candidates" && method === "POST") {
    const items = drafts(agentId);
    const index = items.findIndex((item) => item.id === body.candidate_id);
    const prior = index < 0 ? undefined : items[index];
    const candidate = { id: prior?.id ?? `demo-draft-${crypto.randomUUID()}`, account_id: body.account_id, agent_id: agentId, audience_id: `owner:${agentId}`, source_refs: body.source_refs ?? [], action: body.action, revision: (prior?.revision ?? 0) + 1, created_at: prior?.created_at ?? new Date().toISOString() };
    if (index < 0) items.unshift(candidate); else items[index] = candidate;
    saveDrafts(agentId, items); return { candidate };
  }
  if (parts[1] === "accounts" && parts.length === 5 && parts[3] === "candidates" && parts[4] && method === "DELETE") {
    saveDrafts(agentId, drafts(agentId).filter((item) => item.id !== parts[4])); return { deleted: true };
  }
  if (parts[1] === "accounts" && ["send", "create-event", "update-event", "cancel-event"].includes(parts.at(-1) ?? "")) return fail("Provider actions are disabled in Synthetic demo mode. Your draft stays local.");
  if (parts[1] === "accounts" && parts[3] === "routines") return { runs: [] };
  if (parts[1] !== "accounts" || !account) return fail("This synthetic demo request is read-only.");
  const endpoint = parts[4];
  if (endpoint === "mail-folders") return { folders: [{ provider_id: "inbox", name: "Inbox" }, { provider_id: "important", name: "Important" }, { provider_id: "archive", name: "Archive" }] };
  if (endpoint === "calendar-sources") return { sources: [{ provider_id: `primary-${account.id}`, name: "Personal", primary: true }, { provider_id: `shared-${account.id}`, name: "Shared", primary: false }] };
  if (endpoint === "mail-preview") {
    const count = Math.max(1, Math.min(Number(body.limit ?? 10), 30));
    const messages = Array.from({ length: count }, (_, i) => ({ provider_id: `demo-message-${account.id}-${i}`, thread_id: `demo-thread-${account.id}-${i}`, from: [`Maya Chen <maya@example.test>`, `Sam Patel <sam@example.test>`, `Updates <updates@example.test>`][i % 3], to: account.identity_masked, cc: null, subject: [`Project check-in · ${i + 1}`, "Your weekly summary", "Schedule update"][i % 3], received_at: dateFor(-i, 8 + i % 8), preview: "Synthetic sample message for preview and regression testing. No real account is connected.", body_text: "Hello! This is synthetic demonstration content. It contains no real personal or provider data.", body_status: "available", has_attachments: i % 5 === 0, attachments: [] }));
    return { messages: body.query ? messages.filter((message) => `${message.subject} ${message.from}`.toLowerCase().includes(String(body.query).toLowerCase())) : messages };
  }
  if (endpoint === "message-preview") return { provider_id: body.provider_id, body_text: "Synthetic demo message body. No provider connection is used.", body_status: "available" };
  if (endpoint === "thread-preview") return { provider_id: body.thread_id, messages: [{ provider_id: `${body.thread_id}-1`, thread_id: body.thread_id, from: "Maya Chen <maya@example.test>", to: account.identity_masked, cc: null, subject: "Synthetic conversation", received_at: dateFor(-1), preview: "Synthetic reply", body_text: "This conversation is sample content for safe testing.", body_status: "available", has_attachments: true, attachments: [{ provider_id: "demo-attachment-1", filename: "sample-notes.txt", mime_type: "text/plain", size_bytes: 96, previewable: true }] }] };
  if (endpoint === "attachment-preview") return { filename: "sample-notes.txt", mime_type: "text/plain", size_bytes: 96, text: "Synthetic attachment preview. This file is generated locally for demonstration." };
  if (endpoint === "calendar-preview") {
    const events = Array.from({ length: Math.max(1, Math.min(Number(body.limit ?? 50), 50)) }, (_, i) => ({ provider_id: `demo-event-${account.id}-${i}`, account_id: account.id, account_name: account.identity_masked, version: `demo-v${i + 1}`, title: ["Focus time", "Design review", "Lunch break", "Planning session"][i % 4], starts_at: dateFor(i % 10, 8 + (i * 2) % 10), ends_at: dateFor(i % 10, 9 + (i * 2) % 10), starts_on: null, ends_on: null, all_day: false, location: i % 2 ? "Video call" : null, description: "Synthetic calendar event for preview and regression testing.", attendee_count: 0, recurring: false, private: false, can_cancel: true }));
    const from = Date.parse(body.from ?? ""); const to = Date.parse(body.to ?? "");
    return { events: events.filter((event) => (!Number.isFinite(from) || Date.parse(event.starts_at!) >= from) && (!Number.isFinite(to) || Date.parse(event.starts_at!) < to)) };
  }
  if (endpoint === "free-busy-preview") return { busy: Array.from({ length: 5 }, (_, i) => ({ starts_at: dateFor(i, 10 + i), ends_at: dateFor(i, 11 + i) })) };
  if (parts.at(-1) === "refresh") return { account };
  return fail("This synthetic demo request is read-only.");
}
