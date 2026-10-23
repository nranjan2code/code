import { render } from "solid-js/web";
import ChatPane from "../src/components/ChatPane";
import Composer from "../src/components/Composer";
import { sendPrompt } from "../src/App";
import * as store from "../src/store";
import type { OutputItem } from "../src/types";
import "../src/styles.css";

const sid = "composer-commands-fixture";
const params = new URLSearchParams(location.search);
document.documentElement.dataset.theme = params.get("theme") || "light";
window.fetch = async input => {
  const url = new URL(String(input), location.origin);
  const body = url.pathname === "/fs/tree" ? { files: ["notes.txt", "space name.txt", "नोट.txt", 'say "hi".txt'] }
    : url.pathname === "/skills" ? { skills: ["alpha", "beta"].map(name => ({ name, description: `${name} skill` })) }
    : url.pathname === "/commands" ? { commands: [{ name: "echo", description: "Echo command" }] }
    : url.pathname.endsWith("/work") ? null : { participants: [], sessions: [] };
  return new Response(JSON.stringify(body), { headers: { "Content-Type": "application/json" } });
};
store.setHealth({ status: "ok", provider: "fixture", model: "fixture", permission_mode: "ReadOnly", cwd: "/tmp/composer-fixture", warnings: [] } as never);
store.setActiveId(sid);
store.hydrateFromTranscript(sid, [
  { role: "user", content: [{ type: "text", text: "Original request" }] },
  { role: "assistant", content: [{ type: "text", text: "Completed answer" }] },
], [{ entry_id: "turn-1" }, { entry_id: "answer-1" }]);
const doc = (text: string) => ({ schema_version: 2, source_markdown: text, blocks: [], metadata: {}, diagnostics: [] });
store.hydrateFromPresentation(sid, { schema_version: 2, session_id: sid, diagnostics: [], items: [
  { id: "request", turn_id: "turn-1", role: "user", kind: "message", status: "succeeded", content: { type: "document", document: doc("Original request") }, provenance: { entry_id: "turn-1" }, fallback_text: "Original request", actions: [], timestamp: "" },
  { id: "answer", turn_id: "turn-1", role: "assistant", kind: "outcome", status: "succeeded", content: { type: "document", document: doc("Completed answer") }, fallback_text: "Completed answer", actions: [], timestamp: "" },
] as OutputItem[] });
render(() => <main style="height:100dvh;display:flex;flex-direction:column;min-width:0"><ChatPane /><Composer cwd="/tmp/composer-fixture" /></main>, document.getElementById("root")!);

const tick = () => new Promise(resolve => setTimeout(resolve, 50));
const check = (condition: unknown, message: string) => { if (!condition) throw new Error(message); return message; };
const input = (text: string) => {
  const textarea = document.querySelector<HTMLTextAreaElement>("textarea")!;
  textarea.focus();
  textarea.value = text;
  textarea.setSelectionRange(text.length, text.length);
  textarea.dispatchEvent(new InputEvent("input", { bubbles: true }));
  return textarea;
};
const choose = (selector: string, text: string) => {
  const button = [...document.querySelectorAll<HTMLButtonElement>(selector)].find(button => button.textContent?.includes(text));
  check(button, `missing option ${text}`);
  button!.click();
};
(window as unknown as { runChecks: () => Promise<string[]> }).runChecks = async () => {
  const passed: string[] = [];
  await tick();
  for (const [command, feedback] of [["/status", "Nothing is currently tracked"], ["/pause", "Nothing is running to pause"], ["/resume", "Nothing is running to resume"], ["/stop", "Nothing is running to stop"]]) {
    await sendPrompt(command);
    await tick();
    passed.push(check(document.querySelector(".turn-result"), "settled answer uses the server projection"));
    passed.push(check(document.body.innerText.includes(feedback), `${command} feedback stays visible after a settled answer`));
    passed.push(check(document.body.innerText.split(feedback).length === 2, `${command} feedback appears once`));
  }
  for (const path of ["space name.txt", "नोट.txt", 'say "hi".txt']) {
    input("Read @"); await tick();
    choose(".mention-item", path); await tick();
    passed.push(check(document.querySelector<HTMLTextAreaElement>("textarea")!.value === `Read @${JSON.stringify(path)} `, `picker preserves ${path} as one quoted token`));
  }
  input('/skill:alpha /skill:beta /'); await tick();
  passed.push(check(!document.querySelector(".slash-item .slash-name")?.textContent?.includes("clear"), "built-ins are excluded from a skill chain"));
  choose(".slash-item", "/echo"); await tick();
  const text = '/skill:alpha /skill:beta /echo @';
  input(text); await tick();
  choose(".mention-item", "space name.txt"); await tick();
  passed.push(check(document.querySelector<HTMLTextAreaElement>("textarea")!.value === '/skill:alpha /skill:beta /echo @"space name.txt" ', "quoted mention combines with two skills and a custom command"));
  passed.push(check(document.documentElement.scrollWidth <= innerWidth, "no horizontal overflow"));
  return passed;
};
if (params.has("run")) {
  const report = document.createElement("output");
  report.style.cssText = "position:fixed;top:0;right:0;max-width:90vw;z-index:100;background:var(--surface);color:var(--text);padding:8px;font-size:12px";
  document.body.append(report);
  (window as unknown as { runChecks: () => Promise<string[]> }).runChecks().then(checks => {
    report.dataset.result = "passed";
    report.textContent = `${checks.length} checks passed`;
  }).catch(error => { report.dataset.result = "failed"; report.textContent = String(error); });
}
