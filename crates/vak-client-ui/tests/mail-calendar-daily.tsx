import { render } from "solid-js/web";
import DailyMailCalendarViewer from "../src/components/canvas/DailyMailCalendarViewer";
import "../src/styles.css";

const providers = [
  { id: "g-account", provider: "google", capabilities: ["mail_read", "calendar_read"] },
  { id: "m-account", provider: "microsoft", capabilities: ["mail_read", "calendar_read"] },
  { id: "a-account", provider: "apple_icloud", capabilities: ["mail_read", "calendar_free_busy"] },
] as const;
const accounts = providers.map((provider) => ({
  ...provider,
  status: "connected",
  identity_masked: `${provider.provider}@example.test`,
  credential_available: true,
  superseded_by_active_link: false,
  connected_at: "2026-10-01T00:00:00Z",
  access_token_expires_at: null,
  refresh_token_available: false,
  revoked_at: null,
}));
const requests: string[] = [];
const now = new Date();
const day = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
const localDayStart = new Date(`${day(now)}T00:00:00`);
const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "Content-Type": "application/json" } });

window.fetch = async (input) => {
  const url = new URL(String(input), location.origin);
  requests.push(`${url.pathname}${url.search}`);
  if (url.pathname === "/mail-calendar/accounts") return json({ accounts });
  const accountId = url.pathname.split("/")[4];
  const account = providers.find((candidate) => candidate.id === accountId)!;
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
      const start = new Date(localDayStart.getTime() + (8 * 60 + index * 7) * 60_000);
      const end = new Date(start.getTime() + 5 * 60_000);
      return {
        provider_id: `${accountId}-event-${index}`, title: `${account.provider} sample event ${index + 1}`,
        starts_at: start.toISOString(), ends_at: end.toISOString(), starts_on: null, ends_on: null,
        all_day: false, location: null, description: null, attendee_count: 0, recurring: false, private: false, version: null,
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
(window as any).runChecks = async () => {
  await new Promise((resolve) => setTimeout(resolve, 500));
  const passed = [
    check(requests.some((path) => path === "/mail-calendar/accounts?agent_id=fixture-owner"), "Account inventory is scoped to this Agent"),
    check(requests.filter((path) => path.endsWith("/mail-preview")).length === 3, "Recent mail is read once for each connected provider"),
    check(requests.filter((path) => path.endsWith("/calendar-preview")).length === 2 && requests.filter((path) => path.endsWith("/free-busy-preview")).length === 1, "Calendar details and free/busy use the exact provider grants"),
    check(document.querySelectorAll(".daily-mail-calendar-message").length === 24, "The daily view renders eight recent rows from each of three providers"),
    check(document.querySelectorAll(".mail-calendar-event").length === 100, "The daily view renders both bounded 50-event provider batches"),
    check(document.querySelectorAll(".daily-mail-calendar-busy span").length === 8, "The free/busy-only account renders intervals without event details"),
    check(document.body.textContent?.includes("google sample event 1") && document.body.textContent.includes("microsoft sample mail 8") && document.body.textContent.includes("apple_icloud@example.test"), "Provider rows retain their visible source identity"),
    check(!document.body.textContent?.includes("apple_icloud sample event"), "Free/busy rows do not disclose event titles"),
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
