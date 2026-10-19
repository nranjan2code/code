import { render } from "solid-js/web";
import { retryHydrate } from "../src/App";
import { createSignal } from "solid-js";
import ChatPane from "../src/components/ChatPane";
import Composer from "../src/components/Composer";
import * as store from "../src/store";
import type { Message, OutputItem, OutputTimeline } from "../src/types";
import "../src/styles.css";

// Deterministic run-boundary regressions using the real chat and composer.
let paused = false;
const requests: string[] = [];
let durableTranscript: unknown;
let durablePresentation: unknown;
window.fetch = async (input) => {
  const path = String(input);
  requests.push(path);
  if (path.endsWith("/snapshot") && durableTranscript) return new Response(JSON.stringify({ session_id: "handoff-fixture", running: store.isRunning("handoff-fixture"), cursor: "durable:handoff", transcript: durableTranscript, presentation: durablePresentation }));
  if (path.endsWith("/transcript") && durableTranscript) return new Response(JSON.stringify(durableTranscript));
  if (path.endsWith("/presentation") && durablePresentation) return new Response(JSON.stringify(durablePresentation));
  if (path.includes("/library")) return new Response(JSON.stringify({ artifacts: [] }));
  if (path.endsWith("/pause")) paused = true;
  if (path.endsWith("/resume")) paused = false;
  return new Response(JSON.stringify(path.endsWith("/control-state")
    ? { running: true, paused, revision: 0 } : {}), { headers: { "Content-Type": "application/json" } });
};
document.documentElement.dataset.theme = new URLSearchParams(location.search).get("theme") || "light";
const sid = "chat-stability";
const text = (i: number) => `Answer ${i}. ` + "This is a paragraph to keep the conversation tall enough to exercise scrolling. ".repeat(12);
const doc = (value: string) => ({ schema_version: 2, blocks: [{ type: "paragraph", id: value.slice(0, 20), content: [{ type: "text", text: value }] }], source_markdown: value, metadata: {}, diagnostics: [] });
const messages: Message[] = Array.from({ length: 8 }, (_, i) => [
  { role: "user", content: [{ type: "text", text: `Question ${i}` }] },
  { role: "assistant", content: [{ type: "text", text: text(i) }] },
]).flat() as Message[];
const timeline: OutputTimeline = { schema_version: 2, session_id: sid, diagnostics: [], items: Array.from({ length: 8 }, (_, i) => [
  { id: `u${i}`, turn_id: `t${i}`, role: "user", kind: "message", status: "succeeded", provenance: { entry_id: `entry${i * 2}` }, content: { type: "document", document: doc(`Question ${i}`) }, actions: [], fallback_text: `Question ${i}` },
  { id: `a${i}`, turn_id: `t${i}`, role: "assistant", kind: "message", status: "succeeded", content: { type: "document", document: doc(text(i)) }, actions: [], fallback_text: text(i) },
]).flat() as OutputItem[] };
store.hydrateFromTranscript(sid, messages, messages.map((_, i) => ({ entry_id: `entry${i}` })));
store.hydrateFromPresentation(sid, timeline);
store.hydrateFromTranscript("other-stability", messages);
store.setActiveId(sid);
const [result, setResult] = createSignal("Ready");
render(() => <main style="height:100vh;display:flex;flex-direction:column;min-width:0">
  <header style="flex:none;padding:8px;display:flex;gap:12px;align-items:center"><button class="btn" onClick={() => void runChecks()}>Run stability checks</button><output aria-live="polite">{result()}</output></header>
  <ChatPane /><Composer cwd="/tmp/chat-stability" />
</main>, document.getElementById("root")!);
const tick = () => new Promise((resolve) => setTimeout(resolve, 100));
async function runChecks() {
  const passed: string[] = [];
  const check = (condition: unknown, description: string) => { if (!condition) throw new Error(description); passed.push(description); };
  try {
    await tick();
    const chat = document.querySelector<HTMLElement>(".chat")!;
    const oldResult = document.querySelector(".primary-result")!;
    check(Boolean(oldResult), "settled result exists");
    chat.dispatchEvent(new WheelEvent("wheel", { deltaY: -600, bubbles: true }));
    chat.scrollTop = 300;
    await tick();
    const readerTop = chat.scrollTop;
    const viewportTop = chat.getBoundingClientRect().top;
    store.markRunning(sid, true);
    await tick();
    check(chat.getBoundingClientRect().top === viewportTop, "run start keeps viewport top");
    check(Math.abs(chat.scrollTop - readerTop) < 2, "run start keeps reading position");
    check(oldResult.isConnected, "run start retains old result DOM");
    store.hydrateFromPresentation(sid, { ...timeline, cursor: "live:1", items: [...timeline.items, { ...timeline.items[1], id: "live-answer", turn_id: "live:1" }] });
    await tick();
    check(oldResult.isConnected, "live frame retains old result DOM");
    const pause = document.querySelector<HTMLButtonElement>('[aria-label="Pause task"]')!;
    check(Boolean(pause?.closest(".composer-actions")), "Pause lives in input tray");
    check(document.querySelectorAll('[aria-label="Stop running task"]').length === 1, "one existing Stop button");
    pause.click(); await tick();
    check(Boolean(document.querySelector('[aria-label="Resume task"]')), "Pause changes to Resume after server readback");
    document.querySelector<HTMLButtonElement>('[aria-label="Resume task"]')!.click(); await tick();
    check(requests.some((path) => path.endsWith("/resume")), "Resume reaches server");
    store.markRunning(sid, false); await tick();
    check(!document.querySelector('[aria-label="Pause task"]'), "Pause disappears when settled");
    check(chat.getBoundingClientRect().top === viewportTop, "run settlement keeps viewport top");
    check(Math.abs(chat.scrollTop - readerTop) < 2, "run settlement keeps reading position");
    store.setActiveId("other-stability"); await tick();
    store.setActiveId(sid); await tick();
    check(Math.abs(chat.scrollTop - readerTop) < 2, "switching conversations restores position");
    store.appendUser(sid, "A new question from history"); store.markRunning(sid, true); await tick();
    check(chat.scrollHeight - chat.scrollTop - chat.clientHeight < 2, "local send follows new turn");
    chat.dispatchEvent(new WheelEvent("wheel", { deltaY: -24, bubbles: true }));
    chat.scrollTop -= 24; await tick();
    const nearEndTop = chat.scrollTop;
    store.hydrateFromPresentation(sid, { ...timeline, cursor: "live:2" }); await tick();
    check(Math.abs(chat.scrollTop - nearEndTop) < 2, "small upward scroll disengages following");
    check(document.documentElement.scrollWidth <= window.innerWidth, "layout has no horizontal overflow");
    // Exercise the actual App hydration entrypoint, not a copy of its logic.
    // The server snapshot reaches the presentation stream while transcript
    // identity is still optimistic; the same read must apply on settlement.
    const handoff = "handoff-fixture";
    store.markRunning(sid, false);
    store.setActiveId(handoff);
    store.appendUser(handoff, "Explain the evidence and its limits.");
    store.markRunning(handoff, true);
    store.applyEvent(handoff, { TextDelta: { delta: "Closing confidence paragraph." } } as never, {});
    const user = { ...timeline.items[0], id: "saved-user", turn_id: "saved-turn", provenance: { entry_id: "saved-user" }, content: { type: "document", document: doc("Explain the evidence and its limits.") } } as OutputItem;
    const card = { ...timeline.items[1], id: "saved-card", turn_id: "saved-turn", role: "tool", kind: "card", provenance: { source: "emit_summary_card", presentation_id: "presentation-card" }, content: { type: "structured", output: { semantic_type: "summary", block: "summary", schema_version: 2, skill_id: "core", skill_version: "2.0.0", payload: { headline: "Recorded findings survive settlement", text: "Source-supported findings and explicitly unresolved questions.", points: [] } } }, fallback_text: "Recorded findings survive settlement" } as OutputItem;
    const closing = { ...timeline.items[1], id: "saved-closing", turn_id: "saved-turn", content: { type: "document", document: doc("Closing confidence paragraph.") } } as OutputItem;
    durableTranscript = { messages: [{ role: "user", content: [{ type: "text", text: "Explain the evidence and its limits." }] }, { role: "assistant", content: [{ type: "text", text: "Closing confidence paragraph." }] }], entries: [{ entry_id: "saved-user" }, { entry_id: "saved-closing" }], usage: {}, set_aside: [] };
    durablePresentation = { schema_version: 2, session_id: handoff, cursor: "durable:handoff", items: [user, card, closing], diagnostics: [] };
    store.hydrateFromPresentation(handoff, durablePresentation as OutputTimeline);
    await retryHydrate(handoff);
    check(!store.itemsOf(handoff).some(item => item.kind === "user" && item.entryId), "running hydration defers transcript identity");
    store.markRunning(handoff, false);
    await retryHydrate(handoff);
    await tick();
    check(store.itemsOf(handoff).some(item => item.kind === "user" && item.entryId === "saved-user"), "identical settled read applies durable identity");
    check(document.body.innerText.includes("Recorded findings survive settlement"), "settled recorded card is visible through the real ChatPane");
    check(document.body.innerText.split("Closing confidence paragraph.").length === 2, "closing prose appears exactly once beside the recorded card");
    await retryHydrate(handoff); await tick();
    check(document.querySelectorAll(".primary-result").length === 1, "duplicate snapshots do not duplicate the result");
    check(document.documentElement.scrollWidth <= innerWidth, "settled card fits the viewport");
    setResult(`PASS: ${passed.length} checks`);
  } catch (error) { setResult(`FAIL after ${passed.length}: ${String(error)}`); }
}

setTimeout(() => void runChecks(), 300);
