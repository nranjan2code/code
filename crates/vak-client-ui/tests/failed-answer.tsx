import { render } from "solid-js/web";
import "../src/components/ChatPane";
import PresentationTimelineView from "../src/components/PresentationRenderer";
import * as store from "../src/store";
import type { OutputItem, OutputTimeline } from "../src/types";
import "../src/styles.css";

// A card may stand in for an answer's prose only when the turn did what was
// asked. The link-only card and the unrecovered tool failure below are the
// two shapes that once replaced a failure report with a bare link.
const sid = "failed-answer-fixture";
const doc = (text: string) => ({ schema_version: 2, blocks: [{ type: "paragraph", id: text, content: [{ type: "text", text }] }], source_markdown: text, metadata: {}, diagnostics: [] });
const outcome = (result: string) => ({ result_id: result, status: "succeeded", completion: null, evidence_state: null, requirement_ids: [], evidence_receipt_ids: [], evidence: [], human_review: null });
const at = new Date().toISOString();
const message = (turn: string, role: "user" | "assistant", text: string): OutputItem => ({ id: `${turn}-${role}`, turn_id: turn, timestamp: at, role, kind: role === "user" ? "message" : "outcome", status: "succeeded", outcome: role === "user" ? undefined : outcome(`${turn}-result`), content: { type: "document", document: doc(text) }, actions: [], fallback_text: text } as unknown as OutputItem);
const adaptive = (turn: string, root: { primitive: string; props: Record<string, unknown> }, source: string): OutputItem => ({
  id: `${turn}-card`, turn_id: turn, timestamp: at, role: "assistant", kind: "outcome", status: "succeeded", outcome: outcome(`${turn}-result`),
  content: { type: "adaptive", tree: { schema_version: 1, spec_id: "fixture", revision: 1, digest: "d", root: { ...root, children: [] }, accessibility_summary: "", coverage: { rendered_paths: ["$.*"], omitted_paths: [] } }, fallback_text: "" },
  provenance: { session_id: sid, source }, actions: [], fallback_text: "",
} as unknown as OutputItem);
const toolError = (turn: string): OutputItem => ({ id: `${turn}-tool`, turn_id: turn, timestamp: at, role: "tool", kind: "error", status: "failed", content: { type: "error", message: "denied outside the workspace" }, actions: [], fallback_text: "denied outside the workspace" } as unknown as OutputItem);

const approval = (turn: string, command: string): OutputItem => ({ id: `${turn}-approval`, turn_id: turn, timestamp: at, role: "assistant", kind: "approval", status: "pending", content: { type: "approval", request_id: "req-1", tool: "bash", args_json: JSON.stringify({ command }), reason: null }, actions: [], fallback_text: "" } as unknown as OutputItem);
const longCommand = `curl -s https://example.com | head -3 # ${"padding ".repeat(60)}&& echo tail-end-marker`;

const linkOnly = "The file /etc/hosts is outside the workspace, and curl returned no output.";
const afterFailure = "I could not read that file, so this table is incomplete.";
const timeline = { schema_version: 2, session_id: sid, diagnostics: [], items: [
  message("t1", "user", "read /etc/hosts and curl example.com"),
  adaptive("t1", { primitive: "link_preview", props: { title: "Open source link", url: "https://example.com" } }, "adaptive_library"),
  message("t1", "assistant", linkOnly),
  message("t2", "user", "summarise the folder"),
  toolError("t2"),
  adaptive("t2", { primitive: "metric", props: { label: "files", value: "3" } }, "adaptive_library"),
  message("t2", "assistant", afterFailure),
  message("t3", "user", "fetch the page"),
  approval("t3", longCommand),
] } as unknown as OutputTimeline;

store.setHealth({ status: "ok", provider: "x", model: "y", permission_mode: "WorkspaceWrite", sandbox: "seatbelt", context_window: 1, cwd: "/", warnings: [] } as never);
store.setSessions([{ session_id: sid, cwd: "/Users/me/vak-home" } as never]);
store.setActiveId(sid);
store.hydrateFromPresentation(sid, timeline);
render(() => <main style="max-width: 820px; margin: 0 auto; padding: 24px 16px"><PresentationTimelineView timeline={store.presentationOf(sid)!} sessionId={sid} /></main>, document.getElementById("root")!);

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const assert = (condition: unknown, message: string) => { if (!condition) throw new Error(message); return message; };
(window as unknown as { runChecks: () => Promise<string[]> }).runChecks = async () => {
  store.setTechnicalDetails(false);
  await sleep(1200);
  const text = document.body.innerText;
  return [
    assert(text.includes(linkOnly), "a link-only card never hides the answer that reports a failure"),
    assert(text.includes(afterFailure), "a turn with an unrecovered tool failure keeps the answer's prose"),
    assert(document.querySelector(".semantic-approval .ap-primary")?.textContent?.includes("tail-end-marker"), "an approval shows the whole command without opening details"),
    assert(document.querySelector(".semantic-approval .ap-network-note")?.textContent?.includes("no internet access"), "a sandboxed approval says the command has no internet"),
    assert([...document.querySelectorAll(".semantic-approval .ap-primary")].some((row) => row.textContent?.includes("in/Users/me/vak-home")), "the card names the folder the command runs in, even when none was given"),
    assert(!document.querySelector(".semantic-approval details[open]"), "the command is visible while the details stay closed"),
  ];
};
