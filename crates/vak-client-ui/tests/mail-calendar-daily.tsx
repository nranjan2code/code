import { render } from "solid-js/web";
import DailyMailCalendarViewer from "../src/components/canvas/DailyMailCalendarViewer";
import { pendingSettingsPage, settingsOpen } from "../src/store";
import "../src/styles.css";

const providerTypes = [
  { prefix: "g-account", provider: "google", capabilities: ["mail_read", "calendar_read"] },
  { prefix: "m-account", provider: "microsoft", capabilities: ["mail_read", "calendar_read"] },
  { prefix: "a-account", provider: "apple_icloud", capabilities: ["mail_read", "calendar_free_busy"] },
] as const;
const accounts = providerTypes.flatMap((provider) => Array.from({ length: 3 }, (_, index) => ({
  id: `${provider.prefix}-${index}`,
  provider: provider.provider,
  capabilities: provider.capabilities,
  status: "connected",
  identity_masked: `${provider.provider}-${index}@example.test`,
  credential_available: true,
  superseded_by_active_link: false,
  connected_at: "2026-10-01T00:00:00Z",
  access_token_expires_at: null,
  refresh_token_available: false,
  revoked_at: null,
})));
const requests: string[] = [];
const localDraftWrites: Array<{ path: string; body: Record<string, any> }> = [];
const fixtureErrors: string[] = [];
let fixtureResponses = 0;
const activeAccounts = new Map<string, number>();
let maxConcurrentAccounts = 0;
const now = new Date();
const day = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
const localDayStart = new Date(`${day(now)}T00:00:00`);
const json = (value: unknown) => { fixtureResponses += 1; return new Response(JSON.stringify(value), { headers: { "Content-Type": "application/json" } }); };

window.fetch = async (input, init) => {
  try {
    const url = new URL(String(input), location.origin);
    requests.push(`${url.pathname}${url.search}`);
    if (url.pathname === "/mail-calendar/accounts/fixture-owner/candidates" && init?.method === "POST") {
      const body = JSON.parse(String(init.body ?? "{}")) as Record<string, any>;
      localDraftWrites.push({ path: url.pathname, body });
      return json({ candidate: {
        id: "fixture-local-event-draft", account_id: body.account_id, agent_id: "fixture-owner",
        revision: 1, source_refs: body.source_refs ?? [], action: body.action,
        candidate_digest: "synthetic-fixture-digest", created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(), action_state: null,
      } });
    }
  if (url.pathname === "/mail-calendar/accounts") return json({ accounts });
  const accountId = url.pathname.split("/")[4];
  const account = accounts.find((candidate) => candidate.id === accountId)!;
  activeAccounts.set(accountId, (activeAccounts.get(accountId) ?? 0) + 1);
  maxConcurrentAccounts = Math.max(maxConcurrentAccounts, activeAccounts.size);
  await Promise.resolve();
  try {
    if (url.pathname.endsWith("/mail-preview")) {
      return json({ messages: Array.from({ length: 8 }, (_, index) => ({
      provider_id: `${accountId}-message-${index}`,
      thread_id: account.provider === "apple_icloud" ? null : `${accountId}-thread-${index}`,
      from: `sender-${index}@example.test`, to: "owner@example.test", cc: null,
      subject: `${account.provider} sample mail ${index + 1}`,
      received_at: new Date(Date.now() - index * 60_000).toISOString(),
      preview: `Synthetic preview row ${index + 1}`, body_text: null, body_status: "available", has_attachments: false,
      })) });
    }
    if (url.pathname.endsWith("/calendar-preview")) {
      return json({ events: Array.from({ length: 50 }, (_, index) => {
      const start = new Date(localDayStart.getTime() + (7 * 60 + index * 18) * 60_000);
      const end = new Date(start.getTime() + 30 * 60_000);
      return {
        provider_id: `${accountId}-event-${index}`, title: `${account.provider} sample event ${index + 1}`,
        starts_at: start.toISOString(), ends_at: end.toISOString(), starts_on: null, ends_on: null,
        all_day: false, location: null, description: null, attendee_count: 0, recurring: false, private: false, version: account.provider === "google" ? "fixture-etag" : null,
      };
      }) });
    }
    if (url.pathname.endsWith("/free-busy-preview")) {
      return json({ busy: Array.from({ length: 8 }, (_, index) => ({
      starts_at: new Date(localDayStart.getTime() + (9 * 60 + index * 60) * 60_000).toISOString(),
      ends_at: new Date(localDayStart.getTime() + (9 * 60 + index * 60 + 30) * 60_000).toISOString(),
      })) });
    }
    return json({ error: "Unexpected fixture route" });
  } finally {
    const active = (activeAccounts.get(accountId) ?? 1) - 1;
    if (active === 0) activeAccounts.delete(accountId);
    else activeAccounts.set(accountId, active);
  }
  } catch (cause) {
    fixtureErrors.push(cause instanceof Error ? cause.message : String(cause));
    throw cause;
  }
};

render(() => <DailyMailCalendarViewer
  subject={{ kind: "daily_mail_calendar", title: "Today", agentId: "fixture-owner" }}
  view={null}
  reloadKey={0}
  selection={null}
  onSelect={() => undefined}
  register={() => undefined}
/>, document.getElementById("root")!);

const check = (ok: unknown, message: string) => { if (!ok) throw new Error(message); return message; };
const waitFor = async (predicate: () => boolean, timeoutMs = 5000) => {
  const deadline = Date.now() + timeoutMs;
  while (!predicate() && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 50));
  return predicate();
};
(window as any).runChecks = async () => {
  const mailRowsLoaded = await waitFor(() => document.querySelectorAll(".daily-mail-calendar-message").length === 72);
  if (!mailRowsLoaded) {
    const mailRows = document.querySelectorAll(".daily-mail-calendar-message").length;
    const mailRequests = requests.filter((path) => path.endsWith("/mail-preview"));
    const warnings = [...document.querySelectorAll(".daily-mail-calendar-warning")].map((node) => node.textContent?.trim()).filter(Boolean);
    throw new Error(`Expected 72 mail rows after the initial read; found ${mailRows}. Loader ${JSON.stringify((window as any).__mailCalendarLoaderTrace ?? null)}. Captured ${requests.length} routes and ${fixtureResponses} responses (${requests.join(", ")}). Fixture errors: ${fixtureErrors.join("; ") || "none"}. View warnings: ${warnings.join("; ") || "none"}.`);
  }
  const initialMailReads = requests.filter((path) => path.endsWith("/mail-preview")).length;
  const initialCalendarReads = requests.filter((path) => path.endsWith("/calendar-preview")).length;
  const initialFreeBusyReads = requests.filter((path) => path.endsWith("/free-busy-preview")).length;
  const allCalendarEventsRendered = document.querySelectorAll(".mail-calendar-grid-event").length === 300;
  const calendarSelector = document.querySelector<HTMLSelectElement>("select[aria-label='Show calendars']");
  if (calendarSelector) {
    calendarSelector.value = "g-account-0";
    calendarSelector.dispatchEvent(new Event("change", { bubbles: true }));
  }
  const focusedCalendarWorks = await waitFor(() => document.querySelectorAll(".mail-calendar-grid-event").length === 50);
  const gridEvents = [...document.querySelectorAll<HTMLElement>(".mail-calendar-grid-event")];
  const overlapLanesWork = gridEvents.length >= 2 && Math.abs(gridEvents[0].getBoundingClientRect().left - gridEvents[1].getBoundingClientRect().left) > 20;
  const eventTitleIsVisible = gridEvents[0]?.textContent?.includes("google sample event 1") ?? false;
  gridEvents[0]?.click();
  const eventSelectionWorks = !!document.querySelector(".daily-mail-calendar-event-detail")?.textContent?.includes("google sample event 1") && !!document.querySelector(".daily-mail-calendar-event-detail")?.textContent?.includes("Draft an update");
  [...document.querySelectorAll<HTMLButtonElement>(".daily-mail-calendar-event-detail button")]
    .find((button) => button.textContent?.includes("Draft an update"))?.click();
  const localDraftActionWorks = await waitFor(() => localDraftWrites.length === 1 && settingsOpen());
  const draft = localDraftWrites[0]?.body;
  const clickedEventStaysLocal = localDraftActionWorks
    && draft?.account_id === "g-account-0"
    && draft?.source_refs?.[0]?.item_id === "g-account-0-event-0"
    && draft?.action?.kind === "update_event"
    && pendingSettingsPage() === "mail-calendar"
    && !requests.some((path) => /\/candidates\/[^/]+\/(send|create-event|update-event|cancel-event)$/.test(path));
  document.querySelector<HTMLButtonElement>(".daily-mail-calendar-heading button")?.click();
  const manualRefreshWorks = await waitFor(() => requests.filter((path) => path.endsWith("/mail-preview")).length === 18 && document.querySelectorAll(".daily-mail-calendar-message").length === 72);
  window.dispatchEvent(new Event("vak:mail-calendar-changed"));
  const changeRefreshWorks = await waitFor(() => requests.filter((path) => path.endsWith("/mail-preview")).length === 27);
  const passed = [
    check(requests.some((path) => path === "/mail-calendar/accounts?agent_id=fixture-owner"), "Account inventory is scoped to this Agent"),
    check(initialMailReads === 9, "Recent mail is read once for each connected account"),
    check(initialCalendarReads === 6 && initialFreeBusyReads === 3, "Calendar details and free/busy use the exact provider grants across repeated accounts"),
    check(document.querySelectorAll(".daily-mail-calendar-message").length === 72, "The daily view renders eight recent rows for each of nine connected accounts"),
    check(allCalendarEventsRendered, "The daily view renders bounded 50-event batches on a time-based calendar grid"),
    check(focusedCalendarWorks, "The calendar selector focuses the timeline to one account without losing the all-calendar view"),
    check(overlapLanesWork && eventTitleIsVisible, "Concurrent events receive stable separate lanes with visible titles"),
    check(eventSelectionWorks, "Selecting a supported event opens its details and a local-draft next action"),
    check(clickedEventStaysLocal, "Draft an update saves only an Agent-scoped local candidate and opens the mail/calendar workspace without a provider effect"),
    check(manualRefreshWorks && document.body.textContent?.includes("Auto-refreshes every 5 minutes while open"), "Manual refresh reloads the bounded sources and the view explains its refresh cadence"),
    check(changeRefreshWorks, "Account changes refresh the open Today view immediately"),
    check(!!document.querySelector("select[aria-label='Show calendars']") && document.querySelectorAll(".mail-calendar-time-labels > div").length < 25, "The timeline can focus one account and fits its time scale to events"),
    check(document.querySelectorAll(".daily-mail-calendar-busy span").length === 24, "All three free/busy-only accounts render intervals without event details"),
    check(document.body.textContent?.includes("google sample event 1") && document.body.textContent.includes("microsoft sample mail 8") && document.body.textContent.includes("apple_icloud-2@example.test"), "Provider rows retain their visible source identity"),
    check(!document.body.textContent?.includes("apple_icloud sample event"), "Free/busy rows do not disclose event titles"),
    check(maxConcurrentAccounts <= 2 && maxConcurrentAccounts === 2, "Provider reads for many accounts run in batches of at most two accounts"),
  ];
  (window as any).__passed = passed;
  return passed;
};

if (new URLSearchParams(location.search).has("run")) {
  (window as any).runChecks().then((passed: string[]) => {
    const report = document.createElement("pre");
    report.id = "fixture-report";
    report.setAttribute("role", "status");
    report.textContent = `${passed.length} checks passed\n${passed.join("\n")}`;
    document.body.append(report);
  }).catch((error: unknown) => {
    const report = document.createElement("pre");
    report.id = "fixture-report";
    report.setAttribute("role", "alert");
    report.textContent = `Failed: ${error instanceof Error ? error.message : String(error)}`;
    document.body.append(report);
  });
}
