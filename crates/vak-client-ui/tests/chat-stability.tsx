import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import ChatPane from "../src/components/ChatPane";
import Composer from "../src/components/Composer";
import * as store from "../src/store";
import type { Message, OutputItem, OutputTimeline } from "../src/types";
import "../src/styles.css";

// Deterministic run-boundary regressions using the real chat and composer.
let paused = false;
const requests: string[] = [];
window.fetch = async (input) => {
  const path = String(input);
  requests.push(path);
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
    setResult(`PASS: ${passed.length} checks`);
  } catch (error) { setResult(`FAIL after ${passed.length}: ${String(error)}`); }
}

setTimeout(() => void runChecks(), 300);
