import { render } from "solid-js/web";
import "../src/components/ChatPane";
import PresentationTimelineView, { LiveAnswer } from "../src/components/PresentationRenderer";
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
const note = item("paired-note", "tool", "card", { type: "structured", output: { semantic_type: "callout", block: "callout", schema_version: 2, skill_id: "core", skill_version: "2.0.0", payload: { tone: "note", text: "A compact note beside a wide table." } } }, "Compact note");
note.placement = { beside: "first-presentation" };
const timeline = { schema_version: 2, session_id: sid, diagnostics: [], items: [
  item("user", "user", "message", { type: "document", document: doc("Compare both results.") }, "Compare both results."),
  text("leading", fragments[0]), {...table("first", "First table"), provenance: {presentation_id:"first-presentation"}}, note, text("middle", fragments[1]), table("second", "Second table"), text("closing", fragments[2]),
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
  check(document.querySelectorAll("table.card-table").length === 2, "Distinct tables survive grouping");
  check(Array.from(document.querySelectorAll<HTMLElement>(".primary-result .card")).every(card => getComputedStyle(card).borderTopWidth === "0px" && getComputedStyle(card).paddingLeft === "0px"), "Composed forms share the answer surface without independent enclosing boxes");
  check(result.querySelectorAll('.primary-result-actions [aria-label="Copy text"]').length === 1 && result.querySelectorAll('.primary-result-actions button').length > 1, "One result toolbar contains copy and the other result actions");
  check(document.documentElement.scrollWidth <= innerWidth, "The composed result fits the viewport");
  const row = Array.from(document.querySelectorAll<HTMLElement>(".answer-row")).find(row => row.querySelectorAll(".answer-block").length === 2)!;
  check(Boolean(row), "The exact addressed table/note pair shares one layout group");
  const [wide, compact] = Array.from(row.querySelectorAll<HTMLElement>(".answer-block"));
  check(wide.getBoundingClientRect().width <= row.getBoundingClientRect().width + 1 && compact.getBoundingClientRect().width <= row.getBoundingClientRect().width + 1, "Grouped forms fit their available width");
  if (innerWidth < 600) check(compact.getBoundingClientRect().top >= wide.getBoundingClientRect().bottom, "Phone layout stacks the pair");
  else if (row.getBoundingClientRect().width >= 48 * 16 + parseFloat(getComputedStyle(row).gap))
    check(wide.getBoundingClientRect().width > compact.getBoundingClientRect().width, "A fitting pair gives the table more reading width than its note");
  else check(compact.getBoundingClientRect().top >= wide.getBoundingClientRect().bottom, "A pair stacks until both reading widths fit");
  const main = document.querySelector("main")!;
  main.classList.add("answer-in-canvas"); await tick();
  check(Boolean(row.querySelector(".card-table")) && getComputedStyle(row).display === "flex", "Canvas uses the same addressed layout group");
  main.classList.remove("answer-in-canvas");
  store.setTechnicalDetails(true); await tick();
  check(document.querySelectorAll("table.card-table").length === 2, "Technical view also retains both cards");
  store.setTechnicalDetails(false); await tick();
  return passed;
};

(window as any).runLivePlacementChecks = async () => {
  const liveId = 'live-placement-fixture';
  const host = document.createElement('div'); document.body.append(host);
  const a = table('live-a','Live anchor');
  const b = {...note,id:'live-b', placement:{part:null,beside:'live-a',replaces:null}};
  const liveItem = (value: OutputItem) => ({...value,provenance:{...value.provenance,source:'live_event'}});
  store.hydrateFromPresentation(liveId,{...timeline,session_id:liveId,items:[liveItem(a),liveItem(b)]});
  const dispose = render(()=><LiveAnswer sessionId={liveId}/>,host);
  try {
    await tick();
    if (!Array.from(host.querySelectorAll('.answer-row')).some(row=>row.querySelectorAll('.answer-block').length===2)) throw new Error('Live accepted anchors did not pair by presentation identity');
    const replacement=liveItem({...table('live-replacement','Replacement'),placement:{part:null,beside:null,replaces:'live-a'}});
    store.applyPresentationEvent(liveId,{sequence:1,snapshot:null,delta:{type:'item_started',item:replacement}});
    await tick();
    if (host.textContent?.includes('Live anchor') || !host.textContent?.includes('Replacement')) throw new Error('Live replacement did not remove its exact anchor');
    if (Array.from(host.querySelectorAll('.answer-row')).some(row=>row.querySelectorAll('.answer-block').length===2)) throw new Error('Live replacement rebound a missing anchor');
    return ['Live accepted anchors pair by presentation identity','Live store removes exact replacement target','Missing live anchor never rebinds'];
  } finally { dispose(); host.remove(); }
};

(window as any).runResultStateChecks = async () => {
  const passed: string[] = [];
  try {
    for (const [status, label] of [["partial", "Partial result"], ["denied", "Action not allowed"]] as const) {
      const value = text("status-answer", "The available result is preserved.");
      value.status = status;
      value.outcome = { ...value.outcome!, status };
      store.hydrateFromPresentation(sid, { ...timeline, items: [timeline.items[0], value] });
      await tick();
      if (document.querySelectorAll('.primary-result-caution').length !== 1 || !document.querySelector('.primary-result-caution')?.textContent?.includes(label) || !document.body.textContent?.includes('The available result is preserved.')) throw new Error(`${status} lost its visible status or partial content`);
      passed.push(`${status} retains one visible status and its answer`);
    }
    const first = text("separate-a", "First independent result.");
    const second = text("separate-b", "Second independent result.");
    second.outcome = { ...second.outcome!, result_id: "result-2" };
    store.hydrateFromPresentation(sid, { ...timeline, items: [timeline.items[0], first, second] });
    await tick();
    const results = Array.from(document.querySelectorAll('.primary-result'));
    if (results.length !== 2 || results.some(result => result.querySelectorAll('.primary-result-actions').length !== 1 || result.querySelectorAll('[aria-label="Copy text"]').length !== 1)) throw new Error('Independent results lost their separate action scopes');
    passed.push("Independent results keep separate content and action scopes");
    return passed;
  } finally {
    store.hydrateFromPresentation(sid, timeline);
    await tick();
  }
};
