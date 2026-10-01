import { render } from "solid-js/web";
import DailyMailCalendarViewer from "../src/components/canvas/DailyMailCalendarViewer";
import { setSyntheticMailCalendarEnabled, syntheticMailCalendarEnabled } from "../src/mailCalendarDemo";
import { canvasSubject, settingsOpen } from "../src/store";
import type { TaskDef } from "../src/types";
import "../src/styles.css";

const restoreSyntheticDemo = syntheticMailCalendarEnabled();
setSyntheticMailCalendarEnabled(false);

const providerTypes = [
  { prefix: "g-account", provider: "google", capabilities: ["mail_read", "mail_send", "calendar_read"] },
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
const routines: TaskDef[] = Array.from({ length: 19 }, (_, index) => ({
  id: `fixture-routine-${index}`, name: `Synthetic daily brief ${index + 1}`, prompt: "Synthetic routine",
  interval_secs: 0, enabled: index % 4 !== 0, cwd: "/tmp/fixture", created_at: "2026-10-01T00:00:00Z",
  agent_id: "fixture-owner", next_run_at: "2026-10-02T08:00:00Z", last_run_at: null,
  mail_calendar_scope: { routine_id: `fixture-routine-${index}`, account_id: "g-account-0", operations: ["recent_mail"], max_items: 10, watch_new_mail: false },
}));
routines.push({ ...routines[0], id: "other-agent-routine", name: "Must not appear", agent_id: "different-agent" });
routines.push({ ...routines[0], id: "non-mail-routine", name: "Must also not appear", mail_calendar_scope: null });
routines[1].last_run_status = "working";
routines[1].last_session_id = "fixture-active-session";
const requests: string[] = [];
const localDraftWrites: Array<{ path: string; body: Record<string, any> }> = [];
const savedCandidates: Array<Record<string, any>> = [];
let candidateId = 0;
const calendarReadRanges: Array<{ from: string; to: string; cursor?: string }> = [];
const mailPreviewReads: Array<{ accountId: string; query?: string; folder_id?: string }> = [];
const fixtureErrors: string[] = [];
let fixtureResponses = 0;
let inventoryGate: Promise<void> | null = null;
let releaseInventoryGate: (() => void) | null = null;
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
    if (url.pathname === "/agents") return json({ agents: [{ id: "fixture-owner", name: "Fixture owner", revision: 7 }] });
    if (url.pathname === "/tasks" && (!init?.method || init.method === "GET")) return json({ tasks: routines });
    if (url.pathname === "/tasks" && init?.method === "POST") {
      const body = JSON.parse(String(init.body ?? "{}")) as Record<string, any>;
      const task: TaskDef = {
        id: "fixture-created-routine", name: body.name, prompt: body.prompt, agent_id: body.agent_id,
        agent_revision: body.agent_revision, enabled: false, interval_secs: body.interval_secs,
        schedule: body.schedule ?? null, timezone: body.timezone, cwd: "/tmp/fixture", created_at: new Date().toISOString(),
        mail_calendar_scope: { routine_id: "fixture-created-routine", ...body.mail_calendar_scope },
      };
      routines.push(task);
      return json({ task });
    }
    if (url.pathname.startsWith("/tasks/") && init?.method === "PATCH") {
      const id = url.pathname.split("/").at(-1)!;
      const index = routines.findIndex((task) => task.id === id);
      if (index >= 0) routines[index] = { ...routines[index], ...JSON.parse(String(init.body ?? "{}")) };
      return json(routines[index]);
    }
    if (url.pathname.endsWith("/run-now") && init?.method === "POST") return json({ started: true });
    if (url.pathname === "/sessions/fixture-active-session/cancel" && init?.method === "POST") return json({ cancelled: true });
    if (url.pathname.startsWith("/mail-calendar/accounts/fixture-owner/routines/") && url.pathname.endsWith("/history")) return json({ runs: [{ run_id: "fixture-routine-run", routine_id: "fixture-routine-0", account_id: "g-account-0", session_id: "fixture-routine-session", trigger: "manual", status: "complete", started_at: new Date().toISOString(), finished_at: new Date().toISOString(), items_returned: 4 }] });
    if (url.pathname.startsWith("/tasks/") && init?.method === "DELETE") {
      const id = url.pathname.split("/").at(-1)!;
      const index = routines.findIndex((task) => task.id === id);
      if (index >= 0) routines.splice(index, 1);
      return json({ deleted: index >= 0 });
    }
    if (url.pathname.endsWith("/mail-folders")) {
      const accountId = url.pathname.split("/")[4];
      return json({ folders: [{ provider_id: `${accountId}-inbox`, name: "Inbox" }, { provider_id: `${accountId}-projects`, name: "Projects" }] });
    }
    if (url.pathname.endsWith("/calendar-sources")) return json({ sources: [{ provider_id: "g-account-0-primary", name: "Personal", primary: true }] });
    if (url.pathname === "/mail-calendar/accounts/fixture-owner/candidates" && init?.method === "GET") return json({ candidates: savedCandidates });
    if (url.pathname === "/mail-calendar/accounts/fixture-owner/candidates" && init?.method === "POST") {
      const body = JSON.parse(String(init.body ?? "{}")) as Record<string, any>;
      localDraftWrites.push({ path: url.pathname, body });
      const candidate = {
        id: `fixture-local-draft-${++candidateId}`, account_id: body.account_id, agent_id: "fixture-owner",
        revision: 1, source_refs: body.source_refs ?? [], action: body.action,
        candidate_digest: "synthetic-fixture-digest", created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(), action_state: null,
      };
      savedCandidates.unshift(candidate);
      return json({ candidate });
    }
  if (url.pathname === "/mail-calendar/accounts") {
    if (inventoryGate) {
      const gate = inventoryGate;
      inventoryGate = null;
      await gate;
    }
    return json({ accounts });
  }
  const accountId = url.pathname.split("/")[4];
  const account = accounts.find((candidate) => candidate.id === accountId)!;
  activeAccounts.set(accountId, (activeAccounts.get(accountId) ?? 0) + 1);
  maxConcurrentAccounts = Math.max(maxConcurrentAccounts, activeAccounts.size);
  await Promise.resolve();
  try {
    if (url.pathname.endsWith("/message-preview")) return json({ body_text: "Synthetic Apple message body", body_status: "available" });
    if (url.pathname.endsWith("/attachment-preview")) return json({ filename: "agenda.txt", mime_type: "text/plain", size_bytes: 24, text: "Synthetic safe attachment text" });
    if (url.pathname.endsWith("/mail-preview")) {
      const body = JSON.parse(String(init?.body ?? "{}")) as { query?: string; folder_id?: string; cursor?: string };
      mailPreviewReads.push({ accountId, ...body });
      const offset = body.cursor ? 8 : 0;
      return json({ messages: Array.from({ length: 8 }, (_, pageIndex) => {
        const index = offset + pageIndex;
        return {
          provider_id: `${accountId}-message-${index}`,
          thread_id: account.provider === "apple_icloud" ? null : `${accountId}-thread-${index}`,
          from: `sender-${index}@example.test`, to: "owner@example.test", cc: null,
          subject: `${account.provider} sample mail ${index + 1}`,
          received_at: new Date(Date.now() - index * 60_000).toISOString(),
          preview: `Synthetic preview row ${index + 1}`, body_text: null, body_status: "available", has_attachments: false,
        };
      }), next_cursor: body.cursor ? null : "fixture-mail-page-2" });
    }
    if (url.pathname.endsWith("/thread-preview")) {
      const body = JSON.parse(String(init?.body ?? "{}")) as { thread_id?: string; cursor?: string };
      const page = body.cursor ? 2 : 1;
      const messages = page === 1
        ? [{ provider_id: `${accountId}-thread-message-1`, thread_id: body.thread_id, from: "sender@example.test", to: "owner@example.test", cc: null, subject: "Synthetic conversation", received_at: "2026-10-01T10:00:00Z", preview: "First page preview", body_text: "First page full message", body_status: "available", has_attachments: true, attachments: [{ provider_id: "fixture-attachment", filename: "agenda.txt", mime_type: "text/plain", size_bytes: 24, previewable: true }] }]
        : [
            { provider_id: `${accountId}-thread-message-1`, thread_id: body.thread_id, from: "sender@example.test", to: "owner@example.test", cc: null, subject: "Synthetic conversation", received_at: "2026-10-01T10:00:00Z", preview: "Repeated first message", body_text: "Duplicate must collapse", body_status: "available", has_attachments: false },
            { provider_id: `${accountId}-thread-message-2`, thread_id: body.thread_id, from: "owner@example.test", to: "sender@example.test", cc: null, subject: "Re: Synthetic conversation", received_at: "2026-10-01T10:05:00Z", preview: "Second page preview", body_text: "Second page full message", body_status: "available", has_attachments: false },
          ];
      return json({ provider_id: body.thread_id, messages, next_cursor: page === 1 ? "fixture-page-2" : null });
    }
    if (url.pathname.endsWith("/calendar-preview")) {
      const range = JSON.parse(String(init?.body ?? "{}")) as { from: string; to: string; cursor?: string };
      calendarReadRanges.push(range);
      const offset = range.cursor ? 50 : 0;
      return json({ events: Array.from({ length: range.cursor ? 10 : 50 }, (_, pageIndex) => {
      const index = offset + pageIndex;
      const start = new Date(localDayStart.getTime() + (7 * 60 + index * 18) * 60_000);
      const end = new Date(start.getTime() + 30 * 60_000);
      return {
        provider_id: `${accountId}-event-${index}`, title: `${account.provider} sample event ${index + 1}`,
        starts_at: start.toISOString(), ends_at: end.toISOString(), starts_on: null, ends_on: null,
        all_day: false, location: null, description: null, attendee_count: 0, recurring: false, private: false, version: account.provider === "google" ? "fixture-etag" : null,
      };
      }), next_cursor: range.cursor ? null : "fixture-calendar-page-2" });
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

(window as any).__holdNextAccountInventory = () => {
  inventoryGate = new Promise<void>((resolve) => { releaseInventoryGate = resolve; });
};
(window as any).__releaseAccountInventory = () => {
  releaseInventoryGate?.();
  releaseInventoryGate = null;
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
  const mailRowsLoaded = await waitFor(() => document.querySelectorAll(".daily-mail-calendar-message").length === 12);
  if (!mailRowsLoaded) {
    const mailRows = document.querySelectorAll(".daily-mail-calendar-message").length;
    const mailRequests = requests.filter((path) => path.endsWith("/mail-preview"));
    const warnings = [...document.querySelectorAll(".daily-mail-calendar-warning")].map((node) => node.textContent?.trim()).filter(Boolean);
    throw new Error(`Expected 12 visible mail rows after the initial read; found ${mailRows}. Loader ${JSON.stringify((window as any).__mailCalendarLoaderTrace ?? null)}. Captured ${requests.length} routes and ${fixtureResponses} responses (${requests.join(", ")}). Fixture errors: ${fixtureErrors.join("; ") || "none"}. View warnings: ${warnings.join("; ") || "none"}.`);
  }
  const initialMailReads = requests.filter((path) => path.endsWith("/mail-preview")).length;
  const routineRows = document.querySelectorAll(".daily-mail-calendar-routine");
  const routinePagingStartsCorrectly = routineRows.length === 8
    && document.querySelector("nav[aria-label='Routine pages']")?.textContent?.includes("Page 1 of 3") === true
    && !document.body.textContent?.includes("Must not appear");
  document.querySelector<HTMLButtonElement>("nav[aria-label='Routine pages'] button:last-child")?.click();
  const routineNextPageWorks = await waitFor(() => document.querySelectorAll(".daily-mail-calendar-routine").length === 8
    && document.querySelector("nav[aria-label='Routine pages']")?.textContent?.includes("Page 2 of 3") === true);
  document.querySelector<HTMLButtonElement>(".daily-mail-calendar-routine button")?.click();
  const openedRoutine = canvasSubject();
  const routineCanvasHandoffWorks = openedRoutine?.kind === "automation" && openedRoutine.taskId === "fixture-routine-8";
  document.querySelector<HTMLButtonElement>("nav[aria-label='Routine pages'] button:first-child")?.click();
  document.querySelector<HTMLDetailsElement>(".mail-calendar-routine-create")?.querySelector("summary")?.click();
  const routineFormLoaded = await waitFor(() => document.querySelector<HTMLSelectElement>("select[aria-label='Routine account']")?.value === "g-account-0"
    && document.querySelector<HTMLSelectElement>("select[aria-label='Routine calendar source']")?.value === "g-account-0-primary");
  const routineNameField = document.querySelector<HTMLInputElement>(".mail-calendar-routine-create input");
  if (routineNameField) {
    const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(routineNameField), "value")?.set;
    setter?.call(routineNameField, "Prepare the morning brief");
    routineNameField.dispatchEvent(new Event("input", { bubbles: true }));
  }
  const routineBudget = document.querySelector<HTMLSelectElement>("select[aria-label='Routine result budget']");
  if (routineBudget) {
    routineBudget.value = "5";
    routineBudget.dispatchEvent(new Event("change", { bubbles: true }));
  }
  document.querySelector<HTMLButtonElement>(".mail-calendar-routine-create button")?.click();
  const routineCreationStaysAgentScoped = await waitFor(() => routines.some((task) => task.id === "fixture-created-routine"));
  const createdRoutine = routines.find((task) => task.id === "fixture-created-routine");
  document.querySelector<HTMLButtonElement>(".mail-calendar-routine-workspace > .daily-mail-calendar-calendar-heading button")?.click();
  const pauseAllStopsActiveRun = await waitFor(() => routines[1].enabled === false && requests.includes("/sessions/fixture-active-session/cancel"));
  const firstRoutine = () => document.querySelector<HTMLElement>(".daily-mail-calendar-routine");
  firstRoutine()?.querySelector<HTMLDetailsElement>(".mail-calendar-routine-history")?.querySelector("summary")?.click();
  const routineHistoryLoadsInCanvas = await waitFor(() => requests.includes("/mail-calendar/accounts/fixture-owner/routines/fixture-routine-0/history")
    && firstRoutine()?.querySelector(".mail-calendar-routine-history li") !== null
    && firstRoutine()?.querySelector(".mail-calendar-routine-history li")?.textContent?.includes("4 results used") === true);
  const action = (label: string) => [...(firstRoutine()?.querySelectorAll<HTMLButtonElement>(".settings-actions button") ?? [])].find((button) => button.textContent?.trim() === label);
  action("Preview run")?.click();
  const routinePreviewRunWorks = await waitFor(() => requests.includes("/tasks/fixture-routine-0/run-now"));
  const routineResumeReady = await waitFor(() => action("Resume") !== undefined && !action("Resume")!.disabled);
  action("Resume")?.click();
  const routineResumeWorks = await waitFor(() => routines.find((task) => task.id === "fixture-routine-0")?.enabled === true);
  const routinePauseReady = await waitFor(() => action("Pause") !== undefined && !action("Pause")!.disabled);
  action("Pause")?.click();
  const routinePauseWorks = await waitFor(() => routines.find((task) => task.id === "fixture-routine-0")?.enabled === false);
  const routinePauseSettled = await waitFor(() => action("Resume") !== undefined && !action("Resume")!.disabled);
  [...(firstRoutine()?.querySelectorAll<HTMLButtonElement>(".settings-actions button") ?? [])].find((button) => button.textContent?.trim() === "Delete")?.click();
  const confirmDeleteButton = [...(firstRoutine()?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((button) => button.textContent?.trim() === "Confirm delete");
  confirmDeleteButton?.click();
  const routineDeletionWorks = await waitFor(() => !routines.some((task) => task.id === "fixture-routine-0"));
  const initialCalendarReads = requests.filter((path) => path.endsWith("/calendar-preview")).length;
  const initialFreeBusyReads = requests.filter((path) => path.endsWith("/free-busy-preview")).length;
  document.querySelector<HTMLButtonElement>(".daily-mail-calendar-message button")?.click();
  const conversationOpened = await waitFor(() => document.querySelectorAll(".daily-mail-calendar-conversation-message").length === 1);
  const conversation = document.querySelector(".daily-mail-calendar-conversation");
  [...(conversation?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((button) => button.textContent?.includes("Preview attachment"))?.click();
  const attachmentPreviewWorks = await waitFor(() => conversation?.textContent?.includes("Synthetic safe attachment text") === true
    && requests.some((path) => path.endsWith("/attachment-preview")));
  [...(conversation?.querySelectorAll<HTMLButtonElement>("button") ?? [])]
    .find((button) => button.textContent?.includes("Load more messages"))?.click();
  const conversationPagingWorks = await waitFor(() => document.querySelectorAll(".daily-mail-calendar-conversation-message").length === 2);
  const threadRows = [...document.querySelectorAll(".daily-mail-calendar-conversation-message")];
  const conversationDedupesIds = threadRows.length === 2 && threadRows.some((row) => row.textContent?.includes("Second page full message")) && !threadRows.some((row) => row.textContent?.includes("Duplicate must collapse"));
  const replyButton = [...(conversation?.querySelectorAll<HTMLButtonElement>("button") ?? [])]
    .find((button) => button.textContent?.includes("Draft reply in Canvas"));
  replyButton?.click();
  const conversationHandoffWorks = await waitFor(() => localDraftWrites.some(({ body }) => body.action?.kind === "send_mail"
    && body.action.draft.reply_to_message_id === "g-account-0-thread-message-1"));
  const renderedMailRows = document.querySelectorAll(".daily-mail-calendar-message").length;
  const mailPager = document.querySelector<HTMLElement>("nav[aria-label='Recent email pages']");
  const mailPagingStartsCorrectly = mailPager?.textContent?.includes("Page 1 of 6") === true && renderedMailRows === 12;
  const nextMailPage = () => mailPager?.querySelector<HTMLButtonElement>("button:last-child")?.click();
  const previousMailPage = () => mailPager?.querySelector<HTMLButtonElement>("button:first-child")?.click();
  nextMailPage(); nextMailPage();
  const microsoftMailPageWorks = await waitFor(() => mailPager?.textContent?.includes("Page 3 of 6") === true
    && document.body.textContent?.includes("microsoft sample mail 8") === true);
  nextMailPage(); nextMailPage(); nextMailPage();
  const appleMailPageWorks = await waitFor(() => mailPager?.textContent?.includes("Page 6 of 6") === true
    && document.body.textContent?.includes("apple_icloud sample mail 8") === true);
  for (let page = 0; page < 5; page++) previousMailPage();
  const mailPagerReturnsToFirstPage = await waitFor(() => mailPager?.textContent?.includes("Page 1 of 6") === true);
  const appleAccountSelect = document.querySelector<HTMLSelectElement>("select[aria-label='Mail account']");
  if (appleAccountSelect) { appleAccountSelect.value = "a-account-0"; appleAccountSelect.dispatchEvent(new Event("change", { bubbles: true })); }
  const appleSelectedMessageOpened = await waitFor(() => document.querySelectorAll(".daily-mail-calendar-message").length === 8);
  document.querySelector<HTMLButtonElement>(".daily-mail-calendar-message button")?.click();
  const appleMessageBodyPreviewWorks = appleSelectedMessageOpened && await waitFor(() => document.querySelector(".daily-mail-calendar-conversation")?.textContent?.includes("Synthetic Apple message body") === true)
    && requests.some((path) => path.endsWith("/message-preview"));
  const allCalendarEventsRendered = await waitFor(() => document.querySelectorAll(".mail-calendar-grid-event").length === 300);
  const initialPhoneTimeline = document.querySelector<HTMLElement>(".mail-calendar-time-grid.is-day");
  const phoneTimelineScrollWorks = window.innerWidth > 640
    || (!!initialPhoneTimeline && document.documentElement.scrollWidth === window.innerWidth
      && initialPhoneTimeline.scrollWidth > initialPhoneTimeline.clientWidth
      && (initialPhoneTimeline.querySelector<HTMLElement>(".mail-calendar-grid-event")?.getBoundingClientRect().width ?? 0) >= 130);
  const calendarSelector = document.querySelector<HTMLSelectElement>("select[aria-label='Show calendars']");
  if (calendarSelector) {
    calendarSelector.value = "g-account-0";
    calendarSelector.dispatchEvent(new Event("change", { bubbles: true }));
  }
  const focusedCalendarWorks = await waitFor(() => document.querySelectorAll(".mail-calendar-grid-event").length === 50);
  const gridEvents = [...document.querySelectorAll<HTMLElement>(".mail-calendar-grid-event")];
  const overlapLanesWork = gridEvents.length >= 2 && Math.abs(gridEvents[0].getBoundingClientRect().left - gridEvents[1].getBoundingClientRect().left) > 20;
  const eventTitleIsVisible = gridEvents[0]?.textContent?.includes("google sample event 1") ?? false;
  const gridEventHasKeyboardSemantics = gridEvents[0]?.getAttribute("role") === "button"
    && gridEvents[0]?.getAttribute("tabindex") === "0"
    && !!gridEvents[0]?.getAttribute("aria-label");
  gridEvents[0]?.focus();
  gridEvents[0]?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  const enterOpensDetails = await waitFor(() => document.querySelector(".daily-mail-calendar-event-detail h4")?.textContent?.includes("google sample event 1") === true);
  document.querySelector<HTMLButtonElement>(".daily-mail-calendar-event-detail-heading button")?.click();
  gridEvents[0]?.focus();
  gridEvents[0]?.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true }));
  const spaceOpensDetails = await waitFor(() => document.querySelector(".daily-mail-calendar-event-detail h4")?.textContent?.includes("google sample event 1") === true);
  gridEvents[0]?.click();
  const eventSelectionWorks = !!document.querySelector(".daily-mail-calendar-event-detail")?.textContent?.includes("google sample event 1") && !!document.querySelector(".daily-mail-calendar-event-detail")?.textContent?.includes("Draft an update");
  [...document.querySelectorAll<HTMLButtonElement>(".daily-mail-calendar-event-detail button")]
    .find((button) => button.textContent?.includes("Draft an update"))?.click();
  const localDraftActionWorks = await waitFor(() => localDraftWrites.some(({ body }) => body.action?.kind === "update_event")
    && document.querySelector(".mail-calendar-canvas-workspace button")?.textContent?.includes("Hide drafts"));
  const draft = localDraftWrites.find(({ body }) => body.action?.kind === "update_event")?.body;
  const clickedEventStaysLocal = localDraftActionWorks
    && draft?.account_id === "g-account-0"
    && draft?.source_refs?.[0]?.item_id === "g-account-0-event-0"
    && draft?.action?.kind === "update_event"
    && !settingsOpen()
    && !requests.some((path) => /\/candidates\/[^/]+\/(send|create-event|update-event|cancel-event)$/.test(path));
  const viewerBeforeRefresh = document.querySelector(".daily-mail-calendar-view");
  const selectedEventBeforeRefresh = document.querySelector(".daily-mail-calendar-event-detail");
  const mailRowBeforeRefresh = document.querySelector(".daily-mail-calendar-message");
  const calendarEventBeforeRefresh = document.querySelector(".mail-calendar-grid-event");
  const selectedMailAccount = document.querySelector<HTMLSelectElement>("select[aria-label='Mail account']");
  const mailReadsBeforeManualRefresh = requests.filter((path) => path.endsWith("/mail-preview")).length;
  document.querySelector<HTMLButtonElement>(".daily-mail-calendar-heading button:last-child")?.click();
  const manualRefreshWorks = await waitFor(() => requests.filter((path) => path.endsWith("/mail-preview")).length > mailReadsBeforeManualRefresh
    && document.querySelectorAll(".daily-mail-calendar-message").length > 0
    && !document.querySelector(".artifact-canvas-refreshing"), 15_000);
  const mailRowsExpectedAfterRefresh = document.querySelectorAll(".daily-mail-calendar-message").length;
  const manualRefreshDiagnostics = `before=${mailReadsBeforeManualRefresh}, after=${requests.filter((path) => path.endsWith("/mail-preview")).length}, account=${selectedMailAccount?.value ?? "missing"}, rows=${document.querySelectorAll(".daily-mail-calendar-message").length}/${mailRowsExpectedAfterRefresh}`;
  const refreshKeepsMountedRows = !!viewerBeforeRefresh && document.querySelector(".daily-mail-calendar-view") === viewerBeforeRefresh;
  const refreshKeepsSelectedEvent = !!selectedEventBeforeRefresh && document.querySelector(".daily-mail-calendar-event-detail") === selectedEventBeforeRefresh;
  const refreshKeepsMailAndCalendarRows = !!mailRowBeforeRefresh && document.querySelector(".daily-mail-calendar-message") === mailRowBeforeRefresh
    && !!calendarEventBeforeRefresh && document.querySelector(".mail-calendar-grid-event") === calendarEventBeforeRefresh;
  window.dispatchEvent(new Event("vak:mail-calendar-changed"));
  const mailReadsAfterManualRefresh = requests.filter((path) => path.endsWith("/mail-preview")).length;
  const changeRefreshWorks = await waitFor(() => requests.filter((path) => path.endsWith("/mail-preview")).length > mailReadsAfterManualRefresh);
  const inventoriesBeforeGate = requests.filter((path) => path.startsWith("/mail-calendar/accounts?")).length;
  (window as any).__holdNextAccountInventory();
  document.querySelector<HTMLButtonElement>(".daily-mail-calendar-heading button:last-child")?.click();
  const firstGatedInventoryStarted = await waitFor(() => requests.filter((path) => path.startsWith("/mail-calendar/accounts?")).length === inventoriesBeforeGate + 1);
  const backgroundRefreshKeepsContent = document.querySelectorAll(".daily-mail-calendar-message").length === mailRowsExpectedAfterRefresh
    && !document.querySelector(".artifact-canvas-loading")
    && !!document.querySelector(".artifact-canvas-refreshing");
  window.dispatchEvent(new Event("vak:mail-calendar-changed"));
  (window as any).__releaseAccountInventory();
  const queuedAccountRefreshWorks = firstGatedInventoryStarted
    && await waitFor(() => requests.filter((path) => path.startsWith("/mail-calendar/accounts?")).length >= inventoriesBeforeGate + 2
      && document.querySelectorAll(".daily-mail-calendar-message").length === mailRowsExpectedAfterRefresh
      && !document.querySelector(".artifact-canvas-refreshing"), 15_000);
  const calendarsBeforeRange = requests.filter((path) => path.endsWith("/calendar-preview")).length;
  const rangeFrom = day(new Date());
  const rangeThroughDate = new Date(`${rangeFrom}T12:00:00`);
  rangeThroughDate.setDate(rangeThroughDate.getDate() + 14);
  const rangeThrough = day(rangeThroughDate);
  const fromInput = document.querySelector<HTMLInputElement>("input[aria-label='Calendar start date']");
  const throughInput = document.querySelector<HTMLInputElement>("input[aria-label='Calendar end date']");
  if (fromInput && throughInput) {
    fromInput.value = rangeFrom; fromInput.dispatchEvent(new Event("input", { bubbles: true }));
    throughInput.value = rangeThrough; throughInput.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector<HTMLButtonElement>(".daily-mail-calendar-calendar-tools button:last-of-type")?.click();
  }
  const dateRangeCalendarWorks = await waitFor(() => requests.filter((path) => path.endsWith("/calendar-preview")).length === calendarsBeforeRange + 6
    && document.querySelector(".daily-mail-calendar-calendar-heading h3")?.textContent?.includes("15 days"));
  const exclusiveThrough = new Date(`${rangeThrough}T00:00:00`); exclusiveThrough.setDate(exclusiveThrough.getDate() + 1);
  const dateRangeBoundariesCorrect = calendarReadRanges.slice(-6).length === 6
    && calendarReadRanges.slice(-6).every((range) => range.from === new Date(`${rangeFrom}T00:00:00`).toISOString() && range.to === exclusiveThrough.toISOString());
  const calendarPageNav = document.querySelector<HTMLElement>("nav[aria-label='Calendar week pages']");
  const calendarWeekPagingStartsCorrectly = calendarPageNav?.textContent?.includes("Week 1 of 3") === true;
  calendarPageNav?.querySelector<HTMLButtonElement>("button:last-child")?.click();
  const calendarNextWeekWorks = await waitFor(() => calendarPageNav?.textContent?.includes("Week 2 of 3") === true);
  calendarPageNav?.querySelector<HTMLButtonElement>("button:first-child")?.click();
  const calendarPreviousWeekWorks = await waitFor(() => calendarPageNav?.textContent?.includes("Week 1 of 3") === true);
  const inboxesBeforeFilter = mailPreviewReads.length;
  const mailAccountSelect = document.querySelector<HTMLSelectElement>("select[aria-label='Mail account']");
  if (mailAccountSelect) {
    mailAccountSelect.value = "g-account-0";
    mailAccountSelect.dispatchEvent(new Event("change", { bubbles: true }));
  }
  const accountFoldersLoaded = await waitFor(() => document.querySelector<HTMLSelectElement>("select[aria-label='Mail folder or label']")?.options.length === 2
    && mailPreviewReads.length === inboxesBeforeFilter + 1);
  const mailFolderSelect = document.querySelector<HTMLSelectElement>("select[aria-label='Mail folder or label']");
  if (mailFolderSelect) {
    mailFolderSelect.value = "g-account-0-projects";
    mailFolderSelect.dispatchEvent(new Event("change", { bubbles: true }));
  }
  const searchInput = document.querySelector<HTMLInputElement>("input[aria-label='Search mail']");
  if (searchInput) { searchInput.value = "quarterly budget"; searchInput.dispatchEvent(new Event("input", { bubbles: true })); }
  document.querySelector<HTMLButtonElement>(".daily-mail-calendar-mail-filters button")?.click();
  const selectedFolderSearchWorks = await waitFor(() => mailPreviewReads.length === inboxesBeforeFilter + 2
    && mailPreviewReads.at(-1)?.accountId === "g-account-0"
    && mailPreviewReads.at(-1)?.folder_id === "g-account-0-projects"
    && mailPreviewReads.at(-1)?.query === "quarterly budget"
    && document.querySelectorAll(".daily-mail-calendar-message").length === 8);
  [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.trim() === "Load more messages")?.click();
  const providerMailPagingWorks = await waitFor(() => mailPreviewReads.at(-1)?.cursor === "fixture-mail-page-2"
    && mailRowsLoaded
    && document.querySelector("nav[aria-label='Recent email pages']")?.textContent?.includes("16 messages loaded") === true);
  [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.trim() === "Load more events")?.click();
  const providerCalendarPageRequested = await waitFor(() => calendarReadRanges.some((range) => range.cursor === "fixture-calendar-page-2"));
  const providerCalendarPagingWorks = await waitFor(() => requests.some((path) => path.endsWith("/calendar-preview"))
    && document.querySelectorAll(".mail-calendar-grid-event").length >= 60
    && document.body.textContent?.includes("google sample event 60") === true);
  const passed = [
    check(requests.some((path) => path === "/mail-calendar/accounts?agent_id=fixture-owner"), "Account inventory is scoped to this Agent"),
    check(initialMailReads === 9, "Recent mail is read once for each connected account"),
    check(initialCalendarReads === 6 && initialFreeBusyReads === 3, "Calendar details and free/busy use the exact provider grants across repeated accounts"),
    check(routinePagingStartsCorrectly && routineNextPageWorks, "Canvas routine list paginates mail/calendar routines and excludes other Agents and non-mail tasks"),
    check(routineCanvasHandoffWorks, "Opening a listed routine hands off to its existing Canvas routine workspace"),
    check(routineFormLoaded && routineCreationStaysAgentScoped && createdRoutine?.agent_id === "fixture-owner" && createdRoutine?.enabled === false && createdRoutine?.mail_calendar_scope?.calendar_source_id === "g-account-0-primary" && createdRoutine?.mail_calendar_scope?.max_items === 5, "Canvas saves the selected per-run result budget with the paused Agent-scoped routine"),
    check((firstRoutine()?.textContent ?? "").includes("Up to 10 results per run"), "Saved routine cards show the broker-enforced per-run result budget"),
    check(pauseAllStopsActiveRun, "Canvas pause-all disables enabled routines and asks a currently running Agent session to stop"),
    check(routineHistoryLoadsInCanvas && routinePreviewRunWorks && routineResumeReady && routineResumeWorks && routinePauseReady && routinePauseWorks && routinePauseSettled && routineDeletionWorks, `Canvas supports per-routine history, preview run, resume, pause and schedule deletion (${[routineHistoryLoadsInCanvas, routinePreviewRunWorks, routineResumeReady, routineResumeWorks, routinePauseReady, routinePauseWorks, routinePauseSettled, routineDeletionWorks].join(",")})`),
    check(conversationOpened && conversation?.textContent?.includes("First page full message"), "Opening a recent message shows its full conversation inside Today without adding it to session history"),
    check(conversationPagingWorks && conversationDedupesIds && requests.some((path) => path.endsWith("/thread-preview")), "Today conversation pagination follows the returned cursor and collapses repeated provider message IDs"),
    check(attachmentPreviewWorks, "An eligible conversation attachment opens a bounded read-only text preview in Today Canvas"),
    check(conversationHandoffWorks, "A reply opens as a saved Agent-scoped draft in Canvas without sending it"),
    check(mailPagingStartsCorrectly && microsoftMailPageWorks && appleMailPageWorks && mailPagerReturnsToFirstPage, "Recent mail is split into six stable pages across nine accounts with working next and previous controls"),
    check(appleMessageBodyPreviewWorks, "An Apple selected-message preview loads its body in Canvas without a conversation id"),
    check(allCalendarEventsRendered, "The daily view renders bounded 50-event batches on a time-based calendar grid"),
    check(phoneTimelineScrollWorks, "The phone keeps dense event lanes readable inside the calendar without widening the page"),
    check(focusedCalendarWorks, "The calendar selector focuses the timeline to one account without losing the all-calendar view"),
    check(overlapLanesWork && eventTitleIsVisible, "Concurrent events receive stable separate lanes with visible titles"),
    check(gridEventHasKeyboardSemantics && enterOpensDetails && spaceOpensDetails, "Calendar event cards expose an accessible name and open details with Enter or Space"),
    check(eventSelectionWorks, "Selecting a supported event opens its details and a local-draft next action"),
    check(clickedEventStaysLocal, "Draft an update saves only an Agent-scoped local candidate and opens Canvas drafts without a provider effect"),
    check(manualRefreshWorks && document.body.textContent?.includes("Auto-refreshes every 5 minutes while open"), `Manual refresh reloads the bounded sources and the view explains its refresh cadence (${manualRefreshDiagnostics})`),
    check(refreshKeepsMountedRows, "A settled refresh updates the existing viewer without remounting its page"),
    check(refreshKeepsSelectedEvent, "A settled refresh preserves the open event details without remounting them"),
    check(refreshKeepsMailAndCalendarRows, "A settled refresh keeps existing mail and calendar rows mounted"),
    check(backgroundRefreshKeepsContent, "A background refresh keeps the current page visible without replacing it with a loading screen"),
    check(changeRefreshWorks, "Account changes refresh the open Today view immediately"),
    check(queuedAccountRefreshWorks, `An account change during an in-flight refresh queues one immediate follow-up refresh (first=${firstGatedInventoryStarted}, mail=${requests.filter((path) => path.endsWith("/mail-preview")).length}, account=${requests.filter((path) => path.startsWith("/mail-calendar/accounts?")).length})`),
    check(dateRangeCalendarWorks && dateRangeBoundariesCorrect && calendarWeekPagingStartsCorrectly && calendarNextWeekWorks && calendarPreviousWeekWorks, `Canvas calendar reads inclusive date ranges and paginates longer selections by week without blanking the prior view (${[dateRangeCalendarWorks, dateRangeBoundariesCorrect, calendarWeekPagingStartsCorrectly, calendarNextWeekWorks, calendarPreviousWeekWorks].join(",")}; range=${document.querySelector(".daily-mail-calendar-calendar-heading h3")?.textContent}; nav=${calendarPageNav?.textContent}; reads=${calendarReadRanges.length})`),
    check(accountFoldersLoaded && selectedFolderSearchWorks, "Canvas inbox can browse an Agent-scoped provider folder or label and search only that selected mailbox"),
    check(providerMailPagingWorks, "Canvas fetches later inbox pages from the selected provider folder and appends them without duplicates"),
    check(providerCalendarPageRequested && providerCalendarPagingWorks, "Canvas fetches later provider calendar pages and appends them to the selected calendar without duplicates"),
    check(!!document.querySelector("select[aria-label='Show calendars']") && document.querySelectorAll(".mail-calendar-time-labels > div:not(.mail-calendar-all-day-label)").length <= 24, "The timeline can focus one account and fits its time scale to events"),
    check(document.querySelectorAll(".daily-mail-calendar-busy span").length === 24, "All three free/busy-only accounts render intervals without event details"),
    check(document.body.textContent?.includes("google sample event 1") && document.body.textContent.includes("google sample mail 8") && document.body.textContent.includes("google-0@example.test"), "The visible mail page and calendar retain provider source identity"),
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
    if (restoreSyntheticDemo) setSyntheticMailCalendarEnabled(true);
  }).catch((error: unknown) => {
    const report = document.createElement("pre");
    report.id = "fixture-report";
    report.setAttribute("role", "alert");
    report.textContent = `Failed: ${error instanceof Error ? error.message : String(error)}`;
    document.body.append(report);
    if (restoreSyntheticDemo) setSyntheticMailCalendarEnabled(true);
  });
}
