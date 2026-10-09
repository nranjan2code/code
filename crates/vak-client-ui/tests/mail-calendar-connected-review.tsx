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
let latestConflictCandidate: any = null;
let forceCandidateConflict = false;
let task: any = {
  id: "account-pause-routine", name: "Account pause fixture", agent_id: "fixture-review-owner",
  enabled: true, interval_secs: 60, mail_calendar_scope: { account_id: "review-google-account", operations: ["recent_mail"], max_items: 10 },
  last_session_id: "fixture-active-session", last_run_status: "working", last_run_at: null,
};
const foreignAgentTask = { id: "foreign-agent-routine", name: "Private other agent routine", agent_id: "fixture-other-agent", enabled: true, interval_secs: 60, mail_calendar_scope: { account_id: "other-account" } };
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
    if (forceCandidateConflict && body.candidate_id === saved?.id) {
      forceCandidateConflict = false;
      latestConflictCandidate = {
        ...saved,
        revision: saved.revision + 1,
        action: { ...body.action, draft: { ...body.action.draft, body_text: "A concurrent edit from another browser." } },
        candidate_digest: "fixture-digest-concurrent",
      };
      saved = latestConflictCandidate;
      return new Response(JSON.stringify({ error: "The candidate changed; reload it before saving.", kind: "mail_calendar_candidate_conflict" }), {
        status: 409, headers: { "Content-Type": "application/json" },
      });
    }
    saved = {
      id: body.candidate_id ?? (body.action.kind === "send_mail" && saved?.action?.kind === "send_mail" ? "review-send_mail-candidate-copy" : `review-${body.action.kind}-candidate`), account_id: body.account_id, agent_id: "fixture-review-owner",
      audience_id: "owner:fixture-review-owner", source_refs: body.source_refs ?? [], action: body.action,
      revision: body.expected_revision ? body.expected_revision + 1 : 1, candidate_digest: "fixture-digest-1",
      created_at: saved?.created_at ?? new Date().toISOString(), action_state: null,
    };
    return json({ candidate: saved });
  }
  if (url.pathname.endsWith("/candidates/review-create_event-candidate") && method === "DELETE") {
    const deleted = saved?.id === "review-create_event-candidate";
    if (deleted) saved = null;
    return json({ deleted });
  }
  if (url.pathname.endsWith("/mail-folders")) return json({ folders: [{ provider_id: "INBOX", name: "Inbox" }] });
  if (url.pathname.endsWith("/calendar-sources")) return json({ sources: [{ provider_id: "primary-calendar", name: "Personal", primary: true }] });
  if (url.pathname.endsWith("/mail-preview")) return json({ messages: Array.from({ length: 20 }, (_, index) => index === 0 ? sourceMessage : {
    ...sourceMessage,
    provider_id: `message-${41 - index}`,
    thread_id: null,
    subject: `Synthetic inbox message ${index + 1}`,
    preview: `Synthetic bounded preview ${index + 1}`,
  }) });
  if (url.pathname.endsWith("/calendar-preview")) return json({ events: [sourceEvent] });
  if (url.pathname.endsWith("/thread-preview")) {
    if (body.cursor === "older-page") return json({ messages: [sourceMessage, {
      ...sourceMessage, provider_id: "message-40", subject: "Earlier launch note",
      received_at: "2026-09-30T09:00:00Z", body_text: "Earlier discussion context.",
    }], next_cursor: null });
    return json({ messages: [sourceMessage], next_cursor: "older-page" });
  }
  if (url.pathname === "/tasks" && method === "GET") return json({ tasks: [...(task ? [task] : []), foreignAgentTask] });
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
  if (url.pathname.startsWith("/tasks/") && method === "PATCH") {
    task = { ...task, ...body };
    return json(task);
  }
  if (url.pathname.includes("/routines/") && url.pathname.endsWith("/history")) return json({ runs: [{ run_id: "fixture-run-1", trigger: "manual", status: "complete", started_at: new Date().toISOString(), finished_at: new Date().toISOString(), session_id: "fixture-preview-session", items_returned: 0 }] });
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
const click = async (name: string): Promise<HTMLButtonElement> => {
  let button: HTMLButtonElement | undefined;
  await waitFor(() => {
    button = [...document.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent?.trim() === name);
    return !!button && !button.disabled;
  }, `enabled button ${name}`);
  button!.focus();
  button!.click();
  return button!;
};
const check = (ok: unknown, label: string) => { if (!ok) throw new Error(label); return label; };
const exerciseReviewKeyboard = async (opener: HTMLButtonElement) => {
  let sheet: HTMLElement | undefined;
  await waitFor(() => {
    sheet = [...document.querySelectorAll<HTMLElement>(".sheet[role='dialog'][aria-modal='true']")].at(-1);
    return !!sheet && sheet.contains(document.activeElement);
  }, "keyboard focus to enter the exact Review dialog");
  const controls = [...sheet!.querySelectorAll<HTMLElement>("a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex='-1'])")];
  const first = controls[0];
  const last = controls.at(-1);
  if (!first || !last) throw new Error("The exact Review dialog has no keyboard controls");
  last.focus();
  last.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
  const forwardWraps = document.activeElement === first;
  first.focus();
  first.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true }));
  const reverseWraps = document.activeElement === last;
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  await waitFor(() => !sheet!.isConnected, "Escape to close exact Review");
  await waitFor(() => document.activeElement === opener, "focus to return to the Review opener");
  return { forwardWraps, reverseWraps, escapeRestoresFocus: document.activeElement === opener };
};

(window as any).runChecks = async () => {
  await waitFor(() => !!document.querySelector("button") && document.body.textContent?.includes("owner@example.test") === true, "the fake connected account");
  await waitFor(() => document.body.textContent?.includes("background scheduler has not checked in recently") === true, "stale scheduler health disclosure");
  const settingsHasNoDailyWorkspace = ![...document.querySelectorAll<HTMLButtonElement>("button")]
    .some((button) => ["Preview inbox", "Preview calendar", "Check availability"].includes(button.textContent?.trim() ?? ""))
    && !document.querySelector(".mail-calendar-draft-workspace, .daily-mail-calendar-viewer")
    && !document.querySelector(".mail-calendar-routine-create")
    && ![...document.querySelectorAll<HTMLButtonElement>("button")].some((button) => button.textContent?.trim() === "Save paused routine");
  const canvasEntryPointWorks = [...document.querySelectorAll<HTMLButtonElement>("button")]
    .some((button) => button.textContent?.trim() === "Open in Canvas");
  await click("Pause routines for this account");
  await waitFor(() => task?.enabled === false, "the per-account pause control");
  const passed = [
    check(requests.some((request) => request.path === "/mail-calendar/accounts?agent_id=fixture-review-owner"), "Account inventory stays scoped to the owning Agent"),
    check(document.body.textContent?.includes("service API answers, but its background scheduler has not checked in recently") === true, "The owner UI distinguishes API reachability from a stale background scheduler heartbeat"),
    check(canvasEntryPointWorks && settingsHasNoDailyWorkspace, "Settings links to Canvas and keeps mail/calendar work and routine authoring out of account controls"),
    check(task?.enabled === false && requests.some((request) => request.path === "/tasks/account-pause-routine" && request.method === "PATCH" && request.body.enabled === false), "The account-level pause control pauses only this Agent’s routines"),
    check(!requests.some((request) => new URL(request.path, location.origin).origin !== location.origin), "All fixture requests stay same-origin; no provider or credential endpoint is contacted"),
  ];  const report = document.createElement("pre");
  report.id = "fixture-report";
  report.setAttribute("role", "region");
  report.setAttribute("aria-label", "Acceptance results");
  report.textContent = `${passed.length} checks passed\n${passed.join("\n")}`;
  document.body.append(report);
  return passed;
};
if (new URLSearchParams(location.search).has("run")) {
  (window as any).runChecks().catch((error: unknown) => {
    const report = document.createElement("pre");
    report.id = "fixture-report";
    report.setAttribute("role", "alert");
    report.setAttribute("aria-label", "Acceptance results");
    report.textContent = `Failed: ${error instanceof Error ? error.message : String(error)}`;
    document.body.append(report);
  });
}
