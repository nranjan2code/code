import { render } from "solid-js/web";
import DailyMailCalendarViewer from "../src/components/canvas/DailyMailCalendarViewer";
import { setSyntheticMailCalendarEnabled } from "../src/mailCalendarDemo";
import "../src/styles.css";

setSyntheticMailCalendarEnabled(false);
let serverRequests = 0;
window.fetch = async () => {
  serverRequests += 1;
  return new Response(JSON.stringify({ accounts: [] }), { headers: { "Content-Type": "application/json" } });
};

render(() => <DailyMailCalendarViewer
  subject={{ kind: "daily_mail_calendar", title: "Today", agentId: "demo-entry-owner" }}
  view={null}
  reloadKey={0}
  selection={null}
  onSelect={() => undefined}
  register={() => undefined}
/>, document.getElementById("root")!);

const waitFor = async (predicate: () => boolean) => {
  const deadline = Date.now() + 5000;
  while (!predicate() && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 25));
  return predicate();
};
(window as any).runChecks = async () => {
  const emptyStateLoaded = await waitFor(() => document.body.textContent?.includes("No connected accounts") === true);
  const initialRequests = serverRequests;
  const tryButton = document.querySelector<HTMLButtonElement>("button[aria-label='Use synthetic demo']");
  tryButton?.click();
  const demoLoaded = await waitFor(() => document.querySelector(".daily-mail-calendar-heading p")?.textContent?.includes("Synthetic demo data") === true
    && document.body.textContent?.includes("Synthetic sample message") === true
    && document.querySelectorAll(".mail-calendar-grid-event").length > 0);
  const noDemoServerRequest = serverRequests === initialRequests;
  document.querySelector<HTMLButtonElement>("button[aria-label='Exit synthetic demo']")?.click();
  const returnedToAccounts = await waitFor(() => document.body.textContent?.includes("No connected accounts") === true
    && document.querySelector(".daily-mail-calendar-heading p")?.textContent?.includes("From connected accounts") === true);
  const report = [
    [emptyStateLoaded, "Today starts with the owner’s connected-account view"],
    [!!tryButton && demoLoaded, "Today opens the generated mail/calendar demo with one click"],
    [noDemoServerRequest, "Synthetic mode does not make another mail/calendar server request"],
    [returnedToAccounts, "Exit demo returns to the owner’s connected-account view"],
  ] as const;
  const failed = report.find(([passed]) => !passed);
  const output = document.createElement("pre");
  output.id = "fixture-report";
  output.setAttribute("role", failed ? "alert" : "status");
  output.textContent = `${report.filter(([passed]) => passed).length}/${report.length} checks passed\n${report.map(([passed, label]) => `${passed ? "PASS" : "FAIL"} ${label}`).join("\n")}`;
  document.body.append(output);
  if (failed) throw new Error(failed[1]);
  return report;
};

if (new URLSearchParams(location.search).has("run")) {
  (window as any).runChecks().catch((error: unknown) => {
    const output = document.createElement("pre");
    output.id = "fixture-report";
    output.setAttribute("role", "alert");
    output.textContent = `Failed: ${error instanceof Error ? error.message : String(error)}`;
    document.body.append(output);
  });
}
