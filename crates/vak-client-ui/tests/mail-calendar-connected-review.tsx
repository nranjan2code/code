import { render } from "solid-js/web";
import Settings from "../src/components/Settings";
import { setActiveAgent, setConnection, setHealth, setPendingSettingsPage, setSettingsOpen } from "../src/store";
import "../src/styles.css";

const requests: Array<{ path: string; method: string; body: any }> = [];
const account = {
  id: "review-google-account", provider: "google", status: "connected",
  identity_masked: "owner@example.test", auth_method: "oauth", credential_available: true,
  superseded_by_active_link: false, capabilities: ["mail_read", "mail_send", "calendar_read", "calendar_write"],
  connected_at: "2026-10-01T00:00:00Z", access_token_expires_at: null,
  refresh_token_available: true, revoked_at: null,
};
const sourceMessage = {
  provider_id: "message-41", thread_id: "thread-41", from: "Maya Chen <maya@example.test>",
  to: "owner@example.test", cc: null, subject: "Planning the launch review",
  received_at: "2026-10-01T09:00:00Z", preview: "Could we confirm the agenda?",
  body_text: "Could we confirm the agenda for Friday?", body_status: "available",
  has_attachments: false, attachments: [],
};
const sourceEvent = {
  provider_id: "event-52", account_id: "review-google-account", account_name: "Google · owner@example.test",
  version: "event-version-4", title: "Friday launch review", starts_at: "2026-10-02T16:00:00Z",
  ends_at: "2026-10-02T16:45:00Z", starts_on: null, ends_on: null, all_day: false,
  location: "Room 4", description: "Review the launch checklist.", attendee_count: 0,
  recurring: false, private: false, can_cancel: true,
};
const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "Content-Type": "application/json" } });
let saved: any = null;
let task: any = null;
const agent = { id: "fixture-review-owner", name: "Review fixture", revision: 1, lifecycle: "active", character: "vak", animation: "off" };

window.fetch = async (input, init) => {
  const url = new URL(String(input), location.origin);
  const method = init?.method ?? "GET";
  const body = (() => { try { return JSON.parse(String(init?.body ?? "{}")); } catch { return {}; } })();
  const path = `${url.pathname}${url.search}`;
  requests.push({ path, method, body });
  if (url.pathname === "/mail-calendar/accounts" && method === "GET") return json({ accounts: [account] });
  if (url.pathname.endsWith("/candidates") && method === "GET") return json({ candidates: saved ? [saved] : [] });
  if (url.pathname.endsWith("/candidates") && method === "POST") {
    saved = {
      id: body.candidate_id ?? `review-${body.action.kind}-candidate`, account_id: body.account_id, agent_id: "fixture-review-owner",
      audience_id: "owner:fixture-review-owner", source_refs: body.source_refs ?? [], action: body.action,
      revision: body.expected_revision ? body.expected_revision + 1 : 1, candidate_digest: "fixture-digest-1",
      created_at: saved?.created_at ?? new Date().toISOString(), action_state: null,
    };
    return json({ candidate: saved });
  }
  if (url.pathname.endsWith("/mail-folders")) return json({ folders: [{ provider_id: "INBOX", name: "Inbox" }] });
  if (url.pathname.endsWith("/calendar-sources")) return json({ sources: [{ provider_id: "primary-calendar", name: "Personal", primary: true }] });
  if (url.pathname.endsWith("/mail-preview")) return json({ messages: [sourceMessage] });
  if (url.pathname.endsWith("/calendar-preview")) return json({ events: [sourceEvent] });
  if (url.pathname.endsWith("/thread-preview")) {
    if (body.cursor === "older-page") return json({ messages: [sourceMessage, {
      ...sourceMessage, provider_id: "message-40", subject: "Earlier launch note",
      received_at: "2026-09-30T09:00:00Z", body_text: "Earlier discussion context.",
    }], next_cursor: null });
    return json({ messages: [sourceMessage], next_cursor: "older-page" });
  }
  if (url.pathname === "/tasks" && method === "GET") return json({ tasks: task ? [task] : [] });
  if (url.pathname === "/tasks" && method === "POST") {
    task = {
      id: "routine-event-fixture", name: body.name, agent_id: body.agent_id,
      agent_revision: body.agent_revision, enabled: false, interval_secs: body.interval_secs,
      schedule: body.schedule ?? null, timezone: body.timezone, mail_calendar_scope: body.mail_calendar_scope,
      last_session_id: null, last_run_status: null, last_run_at: null, next_run_at: null,
      mail_calendar_last_check_at: null,
    };
    return json({ task });
  }
  if (url.pathname === "/tasks/routine-event-fixture/run-now" && method === "POST") {
    task = { ...task, last_session_id: "fixture-preview-session", last_run_status: "complete", last_run_at: new Date().toISOString() };
    return json({ started: true });
  }
  if (url.pathname === "/tasks/routine-event-fixture" && method === "PATCH") {
    task = { ...task, ...body };
    return json(task);
  }
  if (url.pathname.includes("/routines/") && url.pathname.endsWith("/history")) return json({ runs: [{ run_id: "fixture-run-1", routine_id: "routine-event-fixture", account_id: account.id, trigger: "manual", status: "complete", started_at: new Date().toISOString(), finished_at: new Date().toISOString(), session_id: "fixture-preview-session" }] });
  if (url.pathname === "/agents") return json({ agents: [agent] });
  if (url.pathname === "/providers") return json({ providers: [] });
  if (url.pathname === "/config" || url.pathname.startsWith("/config?")) return json({ provider: "", model: "", max_turns: 8, paths: { cwd: "/tmp/vak-mail-review-fixture" }, permissions: { allow: [], ask: [], deny: [] }, memory: { search_enabled: true, write_enabled: true, reflection: false, skill_proposals: true } });
  if (url.pathname.includes("presentations")) return json({ definitions: [], activations: [] });
  if (url.pathname.includes("voice")) return json({ providers: [] });
  if (url.pathname.includes("global-route")) return json({ provider: null, model: null });
  if (/\/(send|create-event|update-event|cancel-event)$/.test(url.pathname)) return json({ error: "A provider effect escaped the Review gate." , kind: "fixture_effect_forbidden" });
  return json({});
};

setConnection("live");
setHealth({
  status: "ok", provider: "", model: "", permission_mode: "ReadOnly", sandbox: "",
  context_window: 0, cwd: "/tmp/vak-mail-review-fixture", warnings: [],
  automation_scheduler: {
    status: "stale", last_tick_at: "2026-10-01T00:00:00Z", age_seconds: 61,
    tick_interval_seconds: 20, stale_after_seconds: 60,
  },
});
setActiveAgent({ id: "fixture-review-owner", name: "Review fixture", character: "vak", animation: "off" });
setPendingSettingsPage("mail-calendar");
setSettingsOpen(true);
render(() => <Settings />, document.getElementById("root")!);

const waitFor = async (predicate: () => boolean, label: string, timeout = 5000) => {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error(`Timed out waiting for ${label}`);
};
const click = async (name: string) => {
  let button: HTMLButtonElement | undefined;
  await waitFor(() => {
    button = [...document.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent?.trim() === name);
    return !!button && !button.disabled;
  }, `enabled button ${name}`);
  button!.click();
};
const setField = (label: string, value: string) => {
  const field = [...document.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("input, textarea")].find((item) => item.closest("label")?.textContent?.trim().startsWith(label));
  if (!field) throw new Error(`Could not find field: ${label}`);
  const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(field), "value")?.set;
  setter?.call(field, value);
  field.dispatchEvent(new Event("input", { bubbles: true }));
};
const check = (ok: unknown, label: string) => { if (!ok) throw new Error(label); return label; };

(window as any).runChecks = async () => {
  await waitFor(() => !!document.querySelector("button") && document.body.textContent?.includes("owner@example.test") === true, "the fake connected account");
  await waitFor(() => document.body.textContent?.includes("background scheduler has not checked in recently") === true, "stale scheduler health disclosure");
  await click("Preview inbox");
  await waitFor(() => document.body.textContent?.includes("Planning the launch review") === true, "the bounded source preview");
  await click("Open conversation");
  await waitFor(() => document.body.textContent?.includes("Draft a reply in this conversation") === true, "the source conversation");
  await click("Load more messages");
  await waitFor(() => document.querySelectorAll(".mail-calendar-thread-message").length === 2, "the next conversation page without duplicate messages");
  const paginatedMessages = [...document.querySelectorAll<HTMLElement>(".mail-calendar-thread-message")];
  await click("Draft a reply in this conversation");
  await waitFor(() => !!document.querySelector(".mail-calendar-editor"), "the reply work area");
  setField("To", "maya@example.test");
  setField("Message", "Thanks, Friday works. I will bring the revised agenda.");
  await click("Preview draft");
  await waitFor(() => document.querySelector(".mail-calendar-draft-preview")?.textContent?.includes("Friday works") === true, "the exact local draft preview");
  await click("Save draft");
  await waitFor(() => saved?.candidate_digest === "fixture-digest-1" && saved?.revision === 1, "the Agent-scoped saved candidate");
  await click("Review and send this exact reply");
  await waitFor(() => document.querySelector("[aria-label='Exact effect preview']")?.textContent?.includes("maya@example.test") === true, "the exact-effect Review");
  const review = document.querySelector("[aria-label='Exact effect preview']")?.textContent ?? "";
  const cancel = [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.trim() === "Cancel");
  cancel?.click();
  const mailSaved = saved;
  await click("Preview calendar");
  await waitFor(() => document.body.textContent?.includes("Friday launch review") === true, "the bounded calendar source preview");
  await click("New event draft");
  await waitFor(() => document.querySelector(".mail-calendar-editor")?.textContent?.includes("Calendar event draft") === true, "the event work area");
  setField("Title", "Synthetic project follow-up");
  setField("Starts", "2026-10-03T10:00");
  setField("Ends", "2026-10-03T10:30");
  setField("Location", "Project room");
  setField("Description", "Review the synthetic action plan.");
  await click("Preview draft");
  await waitFor(() => document.querySelector(".mail-calendar-draft-preview")?.textContent?.includes("Synthetic project follow-up") === true, "the exact local event draft preview");
  await click("Save draft");
  await waitFor(() => saved?.action?.kind === "create_event" && saved?.candidate_digest === "fixture-digest-1", "the saved event candidate");
  await click("Review and create this exact event");
  await waitFor(() => document.querySelector("[aria-label='Exact effect preview']")?.textContent?.includes("Synthetic project follow-up") === true, "the exact calendar-effect Review");
  const eventReview = document.querySelector("[aria-label='Exact effect preview']")?.textContent ?? "";
  const eventModal = document.body.textContent ?? "";
  const eventCancel = [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.trim() === "Cancel");
  eventCancel?.click();
  const effectCalls = requests.filter((request) => /\/(send|create-event|update-event|cancel-event)$/.test(request.path));
  const eventTriggerLabel = [...document.querySelectorAll<HTMLLabelElement>("label.capability-item")].find((label) => label.textContent?.includes("Run around a calendar event"));
  const eventTriggerCheckbox = eventTriggerLabel?.querySelector<HTMLInputElement>("input[type=checkbox]");
  eventTriggerCheckbox?.click();
  await waitFor(() => (document.querySelector("select[aria-label='Routine calendar source']") as HTMLSelectElement | null)?.value === "primary-calendar", "the verified calendar source selector");
  setField("Routine name", "Prepare for the next meeting");
  await click("Save paused routine");
  await waitFor(() => task?.id === "routine-event-fixture", "the paused event-trigger routine");
  await click("Preview run");
  await waitFor(() => document.body.textContent?.includes("Open latest run") === true, "the settled one-off routine preview");
  const history = [...document.querySelectorAll<HTMLElement>(".mail-calendar-routine-history summary")].find((summary) => summary.textContent?.includes("Run history"));
  history?.click();
  await waitFor(() => (document.querySelector(".mail-calendar-routine-history") as HTMLDetailsElement | null)?.open === true, "the expanded routine history panel");
  await waitFor(() => requests.some((request) => request.path.endsWith("/routines/routine-event-fixture/history")), "the routine run history request");
  await waitFor(() => document.querySelector(".mail-calendar-routine-history li") !== null, "the preview run history entry");
  await click("Resume");
  await waitFor(() => task?.enabled === true, "the resumed event-trigger routine");
  const passed = [
    check(requests.some((request) => request.path === "/mail-calendar/accounts?agent_id=fixture-review-owner"), "Account inventory stays scoped to the owning Agent"),
    check(document.body.textContent?.includes("service API answers, but its background scheduler has not checked in recently") === true, "The owner UI distinguishes API reachability from a stale background scheduler heartbeat"),
    check(requests.some((request) => request.path.endsWith("/mail-preview") && request.method === "POST"), "Preview reads only the selected provider inbox"),
    check(requests.some((request) => request.path.endsWith("/thread-preview") && request.body.thread_id === "thread-41"), "The source opens its provider conversation"),
    check(requests.some((request) => request.path.endsWith("/thread-preview") && request.body.cursor === "older-page") && paginatedMessages.length === 2 && paginatedMessages[0].dataset.mailMessageId === "message-41" && paginatedMessages[1].dataset.mailMessageId === "message-40", "Load more follows the conversation cursor and collapses message IDs repeated across provider pages"),
    check(mailSaved?.source_refs?.[0]?.item_id === "message-41" && mailSaved?.action?.draft?.reply_to_message_id === "message-41" && mailSaved?.action?.draft?.reply_to_thread_id === "thread-41", "The saved reply retains exact message and conversation lineage"),
    check(mailSaved?.action?.draft?.to?.[0]?.address === "maya@example.test" && mailSaved?.action?.draft?.body_text.includes("Friday works"), "The work area saves the edited recipient and body"),
    check(review.includes("Only this saved revision will be sent") && review.includes("Thanks, Friday works"), "Review shows the exact saved payload and revision semantics"),
    check(effectCalls.length === 0 && !!document.querySelector(".mail-calendar-editor"), "Closing Review leaves the draft in the work area and does not perform a provider effect"),
    check(requests.some((request) => request.path.endsWith("/calendar-preview") && request.method === "POST") && document.body.textContent?.includes("Friday launch review"), "Calendar preview reads the selected account's bounded event range"),
    check(saved?.action?.kind === "create_event" && saved?.action?.draft?.title === "Synthetic project follow-up" && saved?.action?.draft?.location === "Project room", "The work area saves the reviewed event fields"),
    check(eventReview.includes("Only this saved revision will be created") && eventReview.includes("AttendeesNone") && eventReview.includes("ReminderNone") && eventModal.includes("will not invite attendees or set a reminder"), "Calendar Review shows the exact saved revision and effect limits"),
    check(effectCalls.length === 0 && !!document.querySelector(".mail-calendar-editor"), "Closing event Review leaves the draft in the work area without creating an event"),
    check(eventTriggerCheckbox?.checked && task?.mail_calendar_scope?.calendar_event_trigger?.boundary === "start" && task?.mail_calendar_scope?.calendar_source_id === "primary-calendar", "Event-trigger setup pins its boundary and selected calendar source"),
    check(task?.enabled === true && task?.interval_secs === 60 && task?.mail_calendar_scope?.max_items === 10, "The routine previews while paused, then resumes with a bounded one-minute cadence"),
    check(requests.some((request) => request.path.endsWith("/run-now") && request.method === "POST") && requests.some((request) => request.path.endsWith("/history")), "A one-off preview run appears in the routine's run history"),
    check(!requests.some((request) => new URL(request.path, location.origin).origin !== location.origin), "All fixture requests stay same-origin; no provider or credential endpoint is contacted"),
  ];
  const report = document.createElement("pre");
  report.id = "fixture-report";
  report.textContent = `${passed.length} checks passed\n${passed.join("\n")}`;
  document.body.append(report);
  return passed;
};
if (new URLSearchParams(location.search).has("run")) {
  (window as any).runChecks().catch((error: unknown) => {
    const report = document.createElement("pre");
    report.id = "fixture-report";
    report.setAttribute("role", "alert");
    report.textContent = `Failed: ${error instanceof Error ? error.message : String(error)}`;
    document.body.append(report);
  });
}
