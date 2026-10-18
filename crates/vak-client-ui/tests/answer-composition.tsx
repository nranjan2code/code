import { render } from "solid-js/web";
import "../src/components/ChatPane";
import PresentationTimelineView from "../src/components/PresentationRenderer";
import * as store from "../src/store";
import type { OutputItem, OutputTimeline } from "../src/types";
import "../src/styles.css";

document.documentElement.dataset.theme = new URLSearchParams(location.search).get("theme") === "dark" ? "dark" : "light";
const sid = "answer-order-fixture";
const doc = (text: string) => ({ schema_version: 2, blocks: [{ type: "paragraph", id: text, content: [{ type: "text", text }] }], source_markdown: text, metadata: {}, diagnostics: [] });
const item = (id: string, role: string, kind: string, content: unknown, fallback_text: string) => ({ id, turn_id: "turn-1", role, kind, status: "succeeded", timestamp: "2026-10-18T00:00:00Z", content, fallback_text, actions: [], provenance: { source: role === "assistant" ? "session_ledger" : "emit_table_card", presentation_id: kind === "card" ? id : null }, outcome: { result_id: "result-1", status: "succeeded", evidence: [], evidence_receipt_ids: [], requirement_ids: [] } }) as unknown as OutputItem;
const text = (id: string, text: string) => item(id, "assistant", "outcome", { type: "document", document: doc(text) }, text);
const table = (id: string, title: string) => item(id, "tool", "card", { type: "structured", output: { semantic_type: "table", block: "table", schema_version: 2, skill_id: "core", skill_version: "2.0.0", payload: { title, columns: [{ label: "Item", numeric: false }, { label: "Amount", numeric: true }], rows: [["Value", "9007199254740993"]] } } }, title);
const fragments = ["Leading context survives.", "Note: the second result uses a different scope.", "The closing explanation survives too."];
const timeline = { schema_version: 2, session_id: sid, diagnostics: [], items: [
  item("user", "user", "message", { type: "document", document: doc("Compare both results.") }, "Compare both results."),
  text("leading", fragments[0]), table("first", "First table"), text("middle", fragments[1]), table("second", "Second table"), text("closing", fragments[2]),
] } as OutputTimeline;
window.fetch = async () => new Response("{}", { headers: { "Content-Type": "application/json" } });
store.setActiveId(sid);
store.setTechnicalDetails(false);
store.hydrateFromPresentation(sid, timeline);
render(() => <main style="max-width:820px;margin:0 auto;padding:16px"><PresentationTimelineView timeline={store.presentationOf(sid)!} sessionId={sid} /></main>, document.getElementById("root")!);
const tick = () => new Promise(resolve => setTimeout(resolve, 80));
(window as any).runAnswerChecks = async () => {
  const passed: string[] = [];
  const check = (value: unknown, message: string) => { if (!value) throw new Error(message); passed.push(message); };
  await tick();
  const result = document.querySelector(".primary-result")!;
  const body = result.textContent ?? "";
  const order = [fragments[0], "First table", fragments[1], "Second table", fragments[2]];
  check(order.every((text, index) => body.indexOf(text) >= 0 && (index === 0 || body.indexOf(text) > body.indexOf(order[index - 1]))), "Leading, interleaved and closing prose retain source order around both cards");
  check(fragments.every(text => body.split(text).length === 2), "Every prose fragment appears once");
  check(document.querySelectorAll(".primary-result").length === 1 && document.querySelectorAll(".assistant-turn-head").length === 1, "One result shares one author header");
  check(document.querySelectorAll(".card-table").length === 2, "Distinct tables survive grouping");
  check(document.documentElement.scrollWidth <= innerWidth, "The composed result fits the viewport");
  store.setTechnicalDetails(true); await tick();
  check(document.querySelectorAll(".card-table").length === 2, "Technical view also retains both cards");
  store.setTechnicalDetails(false); await tick();
  return passed;
};
