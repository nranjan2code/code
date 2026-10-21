import { render } from "solid-js/web";
import ChatPane from "../src/components/ChatPane";
import * as store from "../src/store";
import { loadHistoryPage, showLatestHistory } from "../src/App";
import type { LoadedHistory } from "../src/historyPages";
import type { Message, OutputItem } from "../src/types";
import "../src/styles.css";

const sid = "history-scroll-fixture";
const total = Number(new URLSearchParams(location.search).get("turns") ?? 10000);
document.documentElement.dataset.theme = new URLSearchParams(location.search).get("theme") || "light";
const requests: string[] = [];
function page(first: number, request?: string): LoadedHistory {
  const last = Math.min(first + 40, total);
  const messages: Message[] = [];
  const entries: { entry_id: string }[] = [];
  const items: OutputItem[] = [];
  for (let n = first; n < last; n++) {
    for (const role of ["user", "assistant"] as const) {
      const id = `${role}-${n}`;
      const text = role === "user" ? `Question ${n}` : `Answer ${n}. ${"This answer is retained in history and loaded only when requested. ".repeat(12)}`;
      messages.push({ role, content: [{ type: "text", text }] });
      entries.push({ entry_id: id });
      items.push({ id, turn_id: `user-${n}`, role, kind: "message", timestamp: "", status: "succeeded", provenance: { entry_id: id },
        content: { type: "document", document: { schema_version: 2, source_markdown: text, blocks: [], metadata: {}, diagnostics: [] } }, actions: [], fallback_text: text });
    }
  }
  return { session_id: sid, running: false, revision: "revision-1", cursor: "durable-fixture",
    transcript: { messages, entries, count: total * 2, usage: {} },
    presentation: { schema_version: 2, session_id: sid, cursor: "durable-fixture", items, diagnostics: [] },
    page: { request, end: `end-${last}`, older: first ? String(Math.max(0, first - 40)) : null, newer: last < total ? String(last) : null, total_turns: total } };
}
window.fetch = async input => {
  const url = new URL(String(input), location.origin);
  requests.push(url.pathname + url.search);
  const value = url.pathname.endsWith("/snapshot")
    ? url.searchParams.get("since") === "revision-1" ? { session_id: sid, not_modified: true, running: false }
      : page(Number(url.searchParams.get("before") ?? Math.max(0, total - 40)), url.searchParams.get("before") ?? undefined)
    : { events: [], sessions: [], participants: [] };
  return new Response(JSON.stringify(value), { headers: { "Content-Type": "application/json" } });
};
store.setActiveId(sid);
store.hydrateHistoryPage(sid, page(Math.max(0, total - 40)), "refresh");
render(() => <main style="height:100vh;display:flex;flex-direction:column;min-width:0"><ChatPane /></main>, document.getElementById("root")!);
const tick = () => new Promise(resolve => setTimeout(resolve, 100));
(window as any).runChecks = async () => {
  const checks: string[] = [];
  const check = (value: unknown, text: string) => { if (!value) throw new Error(text); checks.push(text); };
  await tick();
  const chat = document.querySelector<HTMLElement>(".chat")!;
  check(store.itemsOf(sid).filter(item => item.kind === "user").length === 40, "opens with forty turns independent of total history");
  check(document.querySelectorAll(".chat-turn").length <= 41, "mounts only the recent turn window");
  chat.dispatchEvent(new WheelEvent("wheel", { deltaY: -1000, bubbles: true }));
  chat.scrollTop = 0;
  chat.dispatchEvent(new Event("scroll"));
  await tick(); await tick();
  check(requests.filter(request => request.includes("before=")).length === 1, "scrolling up fetches one older page");
  check(store.itemsOf(sid).filter(item => item.kind === "user").length === 80, "prepends older turns without dropping the recent page");
  check(new Set(store.presentationOf(sid)!.items.map(item => item.id)).size === store.presentationOf(sid)!.items.length, "page merge has no duplicate output identities");
  check(chat.scrollTop > 100, "prepending preserves the original reading anchor");
  const count = store.presentationOf(sid)!.items.length;
  const live = page(total - 1).presentation;
  store.applyPresentationEvent(sid, { snapshot: live });
  check(store.presentationOf(sid)!.items.length === count, "settlement preserves older loaded turns");
  store.applyPresentationEvent(sid, { delta: { type: "text_delta", item_id: `assistant-${total - 1}`, delta: " Updated." } });
  check(store.presentationOf(sid)!.items.at(-1)!.fallback_text.endsWith("Updated."), "live deltas update the addressed item");
  store.markRunning(sid, true);
  const streaming = { ...live.items.at(-1)!, id: "live-paging", turn_id: "live-paging", provenance: { source: "live_event" } };
  store.applyPresentationEvent(sid, { snapshot: { ...live, cursor: "live:fixture", items: [streaming] } });
  await loadHistoryPage(sid, "older");
  check(store.presentationOf(sid)!.items.some(item => item.id === "live-paging"), "paging preserves the active live turn");
  store.markRunning(sid, false);
  await showLatestHistory(sid);
  check(store.itemsOf(sid).filter(item => item.kind === "user").length === 40, "latest resets the reloadable historical cache");
  check(document.documentElement.scrollWidth <= innerWidth, "no horizontal overflow");
  return checks;
};
(window as any).historyFixture = { requests, page, sid, total, loadHistoryPage };
