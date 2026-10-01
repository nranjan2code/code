import * as api from "../src/api";
import { setSyntheticMailCalendarEnabled } from "../src/mailCalendarDemo";

const routes: string[] = [];
window.fetch = async (input) => {
  routes.push(String(input));
  return new Response("{}", { headers: { "Content-Type": "application/json" } });
};
const check = (condition: unknown, label: string) => { if (!condition) throw new Error(label); return label; };
const expectRefused = async (action: () => Promise<unknown>) => {
  try { await action(); return false; }
  catch (error) { return error instanceof api.ApiError && error.status === 403 && error.kind === "synthetic_demo_read_only"; }
};
(window as any).runChecks = async () => {
  window.localStorage.removeItem("vak.mail-calendar.synthetic-drafts.v1.fixture-agent");
  setSyntheticMailCalendarEnabled(true);
  const inventory = await api.listMailCalendarAccounts("fixture-agent");
  const mail = await api.previewMailCalendarMail("fixture-agent", "demo-google-1", 8);
  const calendar = await api.previewMailCalendarEvents("fixture-agent", "demo-google-1", new Date().toISOString(), new Date(Date.now() + 86_400_000).toISOString());
  const saved = await api.saveMailCalendarCandidate("fixture-agent", { account_id: "local-draft", action: { kind: "create_event", draft: { title: "Practice", description: "", location: null, starts_at: new Date().toISOString(), ends_at: new Date(Date.now() + 3_600_000).toISOString(), time_zone: "UTC", all_day: false, attendee_addresses: [], recurrence: null, occurrence_id: null } } });
  const listed = await api.listMailCalendarCandidates("fixture-agent");
  const deniedConnect = await expectRefused(() => api.beginMailCalendarOAuth("fixture-agent", "google", ["mail_read"]));
  const deniedEffect = await expectRefused(() => api.createMailCalendarEventCandidate("fixture-agent", saved.candidate.id, saved.candidate.revision, "demo-digest"));
  const passed = [
    check(inventory.accounts.length === 9 && new Set(inventory.accounts.map((item) => item.provider)).size === 3, "Nine labelled demo accounts cover Google, Microsoft and Apple"),
    check(inventory.accounts.every((item) => item.identity_masked?.endsWith("@example.test") && item.capabilities.every((capability) => capability !== "mail_send" && capability !== "calendar_write")), "Accounts use example.test identities and read-only capabilities"),
    check(mail.messages.length === 8 && mail.messages.every((item) => item.body_text?.toLowerCase().includes("synthetic")), "Mail previews are generated synthetic samples"),
    check(calendar.events.length > 0 && calendar.events.every((item) => item.description?.includes("Synthetic")), "Calendar preview is generated synthetic sample data"),
    check(listed.candidates.some((item) => item.id === saved.candidate.id && item.action.kind === "create_event"), "Drafts save and reopen in browser-local demo storage"),
    check(deniedConnect, "OAuth connection attempts are refused in synthetic mode"),
    check(deniedEffect, "Provider effects are refused in synthetic mode"),
    check(routes.length === 0, "No mail/calendar API request escapes to the server in demo mode"),
  ];
  setSyntheticMailCalendarEnabled(false);
  const report = document.createElement("pre");
  report.id = "fixture-report";
  report.textContent = `${passed.length} checks passed\n${passed.join("\n")}`;
  document.body.append(report);
  return passed;
};
if (new URLSearchParams(location.search).has("run")) {
  (window as any).runChecks().catch((error: unknown) => {
    const report = document.createElement("pre"); report.id = "fixture-report"; report.setAttribute("role", "alert");
    report.textContent = `Failed: ${error instanceof Error ? error.message : String(error)}`; document.body.append(report);
  });
}
