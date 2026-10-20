import { render } from "solid-js/web";
import Data from "../src/Data";
import { api } from "../src/api";
import "../src/styles.css";
const params = new URLSearchParams(location.search);
document.documentElement.dataset.theme = params.get("theme") ?? "light";
window.fetch = async () => { throw new Error("No live data in this fixture"); };
api.dataIntegrity = async () => ({ at: "2026-10-20T12:00:00Z", conversations: 3, chains: 4, records: 15, segments: 8, torn_tails: 0, broken: [], unreadable: params.get("state") === "incomplete" ? ["erasure_receipts"] : [], unchecked_classes: ["tenant_object_payloads", "scope_key_payloads", "derived_index_payloads"], keys: 3, keys_destroyed: 1, keys_held: 0, receipts: 1, receipts_unverified: 0, search: "current", fenced: false });
render(() => <main class="view" style={{ padding: "16px" }}><Data section="integrity" /></main>, document.getElementById("root")!);
Object.assign(window, { runChecks: () => {
  const body = document.body.textContent ?? "";
  if (!body.includes("have not been independently verified")) return null;
  if (body.includes("Nothing is damaged")) throw new Error("False whole-store guarantee");
  if (params.get("state") === "incomplete" && (!document.querySelector('[role="alert"]') || body.includes("Checked records passed"))) throw new Error("Unreadable inventory reported healthy");
  if (document.documentElement.scrollWidth > innerWidth) throw new Error("Horizontal overflow");
  return { checked: true, width: innerWidth, theme: params.get("theme"), state: params.get("state") };
} });
