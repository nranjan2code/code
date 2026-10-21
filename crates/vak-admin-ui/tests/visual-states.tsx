/** Synthetic visual acceptance for states absent from an idle installed home.
 * No request may reach the installed service and no action writes data. */
import { render } from "solid-js/web";
import { createSignal, Show } from "solid-js";
import { api } from "../src/api";
import { Inbox } from "../src/Inbox";
import { AccessPicker, BUILTIN_TOOLS, MATCHER_TOOLS, MatcherBuilder, ScheduleBuilder } from "../src/controls";
import { EvidenceMeter } from "../src/Commitments";
import { setSelectedAgentId } from "../src/store";
import type { InboxEntry } from "../src/types";
import "../src/styles.css";

const params = new URLSearchParams(location.search);
document.documentElement.dataset.theme = params.get("theme") || "light";
setSelectedAgentId("global");
globalThis.fetch = async () => { throw new Error("Visual fixture blocks all network requests"); };
const now = "2026-10-20T07:00:00Z";
const kinds = ["approval_pending", "approval_denied", "budget_alert", "proposal_opened", "heartbeat", "task_summary", "digest", "routine_failed"];
const entries: InboxEntry[] = kinds.map((kind, i) => ({
  id: `fixture-${i}`, ts: now, kind,
  title: kind === "proposal_opened" ? "Review a skill proposal" : `${kind.replaceAll("_", " ")} · sample notice`,
  body: "Synthetic acceptance data. A detailed notice with a long workspace reference /workspace/reports/quarterly-review-and-follow-up to verify wrapping, spacing, and readable status colors.",
  session_id: "synthetic-session", task_id: "synthetic-task", result_id: "synthetic-result", dedupe_key: "synthetic-delivery", origin_state: "available",
}));
api.inbox = async () => ({ entries: params.get("state") === "empty" ? [] : entries, unread_count: entries.length });
api.approvals = async () => ({ approvals: [{ session_id: "synthetic-session", request_id: "synthetic-approval", tool: "bash", args_json: JSON.stringify({ command: "python3 report.py", workspace: "/workspace/reports" }, null, 2), reason: "This command needs your decision before it runs.", requested_at: now }], total: 1 });
api.skillProposals = async () => ({ proposals: [] });

function Controls() {
  const [schedule, setSchedule] = createSignal("0 9 * * 1-5");
  const [matcher, setMatcher] = createSignal("Bash(git *)");
  const [access, setAccess] = createSignal<{ allow: string[] | null; deny: string[] }>({ allow: null, deny: [] });
  return <div class="stack">
    <section class="panel"><h2>Schedule controls</h2><ScheduleBuilder value={schedule()} onChange={setSchedule} /></section>
    <section class="panel"><h2>Rule matcher</h2><MatcherBuilder value={matcher()} onChange={setMatcher} tools={MATCHER_TOOLS} /></section>
    <section class="panel"><AccessPicker title="Tools" inheritLabel="Use the shared tools" limitLabel="Choose tools" noneLabel="No tools" noun="tool" options={BUILTIN_TOOLS} allow={access().allow} deny={access().deny} onChange={setAccess} /></section>
    <section class="panel"><h2>Evidence levels</h2><EvidenceMeter achieved="asserted" required="attested" /><EvidenceMeter achieved="attested" required="cited" /></section>
    <section class="panel"><h2>Button and input states</h2><div class="toolbar"><button>Primary</button><button class="ghost">Secondary</button><button class="approve">Approve</button><button class="danger">Refuse</button><button disabled>Unavailable</button><a class="button ghost" href="#fixture">Navigation</a></div><label class="form-row">Text<input placeholder="Placeholder" /></label><label class="toggle-label"><input type="checkbox" /> Checkbox</label></section>
  </div>;
}
if (params.get("view") === "login") {
  localStorage.setItem("vak_admin_theme", params.get("theme") || "light");
  api.session = async () => ({ authenticated: false });
  const { headlessAuth } = await import("../src/headlessAuth");
  headlessAuth.method = async () => ({ method: params.get("method") === "bootstrap" ? "bootstrap" : "passkey" });
  const { default: App } = await import("../src/App");
  render(() => <App />, document.getElementById("root")!);
} else {
render(() => <div class="view" style={{ padding: "16px" }}><header class="page-header"><div><h1>Synthetic admin states</h1><p>No installed data or actions are used.</p></div></header><Show when={params.get("view") === "controls"} fallback={<Inbox />}><Controls /></Show></div>, document.getElementById("root")!);

}

if (params.get("view") === "controls") Object.assign(window, { runChecks: () => {
  const passed: string[] = [];
  const check = (ok: unknown, label: string) => { if (!ok) throw new Error(label); passed.push(label); };
  const toolbar = [...document.querySelectorAll<HTMLElement>(".panel")].find(x => x.querySelector("h2")?.textContent === "Button and input states")!;
  const primary = toolbar.querySelector<HTMLButtonElement>("button")!;
  const secondary = toolbar.querySelector<HTMLButtonElement>("button.ghost")!;
  const coarse = matchMedia("(pointer: coarse)").matches;
  check(primary.getBoundingClientRect().height === (coarse ? 44 : 32), "Admin actions use shared pointer-aware geometry");
  check(getComputedStyle(secondary).borderTopColor === "rgba(0, 0, 0, 0)", "Admin secondary actions are quiet at rest");
  check(getComputedStyle(primary).backgroundColor !== getComputedStyle(secondary).backgroundColor, "Admin primary emphasis remains explicit");
  check(getComputedStyle(toolbar.querySelector("button.danger")!).color !== getComputedStyle(primary).color, "Destructive actions retain their distinct state");
  for (const field of toolbar.querySelectorAll<HTMLElement>('input:not([type="checkbox"])')) check(field.getBoundingClientRect().height >= (coarse ? 40 : 32), "Admin fields follow the same density");
  for (const control of document.querySelectorAll<HTMLElement>("button,input,select,textarea")) check(parseFloat(getComputedStyle(control).fontSize) >= 12, "Operator controls retain the readable type floor");
  secondary.focus(); check(document.activeElement === secondary, "Admin quiet actions retain keyboard focus");
  const box = toolbar.querySelector<HTMLInputElement>('input[type="checkbox"]')!; box.click(); check(box.checked, "Native checkbox behavior is preserved");
  check(document.documentElement.scrollWidth <= innerWidth + 1, "Admin controls fit the viewport");
  return passed;
} });
