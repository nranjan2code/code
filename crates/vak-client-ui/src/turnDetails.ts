import type { Item, WorkbenchExecution } from "./store";
import type { DocumentBlock, InlineNode, OutputItem, OutputTimeline } from "./types";
import { stripControlScaffolding } from "./structured.ts";

export type StepState = "running" | "done" | "failed" | "waiting" | "ended";
export type ActivityStep = { id: string; label: string; state: StepState; tool?: string; args?: string; result?: string; executionId?: string };
export type DetailTurn = { id: string; request: string; at: string; items: OutputItem[]; steps: ActivityStep[]; executionIds: string[]; live: boolean };

function argsOf(json: string): Record<string, unknown> {
  try { const args = JSON.parse(json); return args && typeof args === "object" && !Array.isArray(args) ? args : {}; } catch { return {}; }
}
const readable = (name: string) => name.replace(/_/g, " ");
const fileName = (args: Record<string, unknown>) => typeof args.path === "string" ? args.path.split(/[\\/]/).pop() : undefined;

/** Labels describe a recorded operation, never infer its subject or claim a
 * successful invocation verified the requested outcome. Raw arguments remain
 * available even when a tool has no specific label. */
export function toolLabel(name: string, json: string, running: boolean): string {
  const args = argsOf(json);
  const file = fileName(args);
  switch (name) {
    case "progress": return running ? "Updating progress" : "Updated progress";
    case "recall": return running ? "Looking back in the conversation" : "Opened conversation context";
    case "read": case "doc_read": case "office_read": return `${running ? "Reading" : "Read"} ${file || "a file"}`;
    case "write": return `${running ? "Writing" : "Wrote"} ${file || "a file"}`;
    case "edit": case "office_apply": return `${running ? "Editing" : "Edited"} ${file || "a file"}`;
    case "glob": return running ? "Finding files" : "Looked for files";
    case "grep": return running ? "Searching file contents" : "Searched file contents";
    case "webfetch": return running ? "Retrieving a web source" : "Retrieved a web source";
    case "skill": return running ? "Loading an ability" : "Loaded an ability";
    case "bash": return running ? "Running a command" : "Ran a command";
    default: return `${running ? "Using" : "Used"} ${readable(name)}`;
  }
}

export function stepFor(item: Item, live: boolean): ActivityStep | null {
  const active = (done: boolean): StepState => done ? "done" : live ? "running" : "ended";
  switch (item.kind) {
    case "tool": return { id: item.id, label: item.isError ? `Could not complete: ${toolLabel(item.name, item.argsJson, true)}` : item.interrupted ? `No completion receipt: ${toolLabel(item.name, item.argsJson, true)}` : toolLabel(item.name, item.argsJson, !item.done && live), state: item.isError ? "failed" : item.interrupted ? "ended" : active(item.done), tool: item.name, args: item.argsJson, result: item.preview ?? undefined };
    // Provider thinking is private technical detail, not a fabricated task
    // description. A later call does not prove an interrupted thought succeeded.
    case "thinking": return { id: item.key, label: item.done ? "Finished thinking" : live ? "Thinking" : "Thinking ended", state: active(item.done), result: item.text };
    case "approval": return { id: item.id, label: `Permission for ${readable(item.tool)}`, state: item.resolved === "denied" ? "failed" : item.resolved === "allowed" ? "done" : item.resolved === "gone" ? "ended" : "waiting", tool: item.tool, args: item.argsJson, result: item.reason };
    case "question": return { id: item.id, label: item.question, state: item.resolved === "answered" ? "done" : item.resolved === "gone" ? "ended" : "waiting", result: item.label };
    case "worker": return { id: `worker:${item.label}`, label: item.label, state: item.isError ? "failed" : item.done ? "done" : live ? "running" : "ended", result: item.lines.join("\n") };
    default: return null;
  }
}

/** User directives and call ids are the join keys. Transcript containment is
 * the recorded message boundary; timestamps, subjects and nearest execution
 * are never used to attach an execution or a file to a turn. */
export function detailTurns(timeline: OutputTimeline | null, transcript: Item[], running: boolean, executions: WorkbenchExecution[]): DetailTurn[] {
  const turns = new Map<string, DetailTurn>();
  const callDirectives = new Map<string, string>();
  let directive: string | undefined;
  for (const item of transcript) {
    if (item.kind === "user") directive = item.entryId ?? "pending-directive";
    else if (item.kind === "tool" && directive) callDirectives.set(item.id, directive);
  }
  // Live presentation frames have per-model-step ids, while the transcript
  // contains the human directive. A shared call id can establish its admitted
  // address before transcript hydration supplies that directive's entry id.
  const admitted = new Set((timeline?.items ?? []).flatMap(item => item.provenance?.tool_call_id && callDirectives.get(item.provenance.tool_call_id) === "pending-directive" && !item.turn_id.startsWith("live") ? [item.turn_id] : []));
  const addressed = (id: string) => id === "pending-directive" && admitted.size === 1 ? [...admitted][0] : id;
  const activeDirective = running && directive ? addressed(directive) : undefined;
  const ensure = (id: string) => {
    let turn = turns.get(id);
    if (!turn) { turn = { id, request: "", at: "", items: [], steps: [], executionIds: [], live: false }; turns.set(id, turn); }
    return turn;
  };
  const callTurns = new Map<string, string>();
  for (const item of timeline?.items ?? []) {
    const liveFrame = item.turn_id.startsWith("live") && item.provenance?.source === "live_event";
    const turn = ensure(liveFrame && activeDirective ? activeDirective : item.turn_id);
    turn.items.push(item);
    if (item.role === "user") { turn.request = stripControlScaffolding(item.fallback_text).trim(); turn.at = item.timestamp; }
    if (item.provenance?.tool_call_id) callTurns.set(item.provenance.tool_call_id, turn.id);
  }
  let current: DetailTurn | undefined;
  const contained: Array<{ turn: DetailTurn; item: Item }> = [];
  for (const item of transcript) {
    if (item.kind === "user") {
      // A not-yet-admitted local directive has no durable id. It is only a
      // live placeholder; it is never used to join historical receipts.
      current = ensure(addressed(item.entryId ?? "pending-directive"));
      current.request = stripControlScaffolding(item.text).trim();
    } else if (item.kind === "tool" && callTurns.has(item.id)) {
      contained.push({ turn: ensure(callTurns.get(item.id)!), item });
    } else if (current) contained.push({ turn: current, item });
  }
  if (running && current) current.live = true;
  // Presentation-only consumers can identify the current live frame without
  // assigning unrelated historical tools to it.
  if (running && !current) {
    const live = [...turns.values()].filter(t => t.items.some(i => i.provenance?.source === "live_event"));
    if (live.length === 1) live[0].live = true;
  }
  const workerOccurrences = new Map<string, number>();
  for (const { turn, item } of contained) {
    const step = stepFor(item, turn.live);
    // Worker events name a label, not a durable call id. Repeated labels
    // need separate render rows; these keys never join any saved records.
    if (step && item.kind === "worker") {
      const key = `${turn.id}:${item.label}`;
      const occurrence = (workerOccurrences.get(key) ?? 0) + 1;
      workerOccurrences.set(key, occurrence);
      step.id = `${step.id}:${occurrence}`;
    }
    if (step && !turn.steps.some(s => s.id === step.id)) turn.steps.push(step);
  }
  const byExecution = new Map(executions.map(exec => [exec.id, exec]));
  for (const turn of turns.values()) {
    const ids = new Set(turn.steps.filter(step => step.tool).map(step => step.id));
    for (const item of turn.items) {
      if (item.content.type === "work_progress") continue;
      const call = item.provenance?.tool_call_id;
      if (call) ids.add(call);
      for (const action of item.actions) if (action.data.execution_id) ids.add(action.data.execution_id);
      // A projected progress/error without a transcript row still has a
      // stable identity. Avoid duplicating its tool's row.
      if (["progress", "retry", "error", "approval"].includes(item.kind) && !turn.steps.some(s => s.id === (call ?? item.id))) {
        turn.steps.push({ id: call ?? item.id, label: item.content.type === "progress" ? item.content.label : item.content.type === "error" ? item.content.message : item.fallback_text,
          state: item.status === "failed" || item.status === "denied" ? "failed" : item.status === "succeeded" ? "done" : item.status === "cancelled" ? "ended" : turn.live ? "running" : "ended" });
      }
    }
    turn.executionIds = [...ids].filter(id => byExecution.has(id));
    for (const step of turn.steps) {
      const card = turn.items.find(item => item.provenance?.tool_call_id === step.id && item.content.type === "structured");
      if (card?.content.type === "structured" && card.content.output.block && step.state === "done") step.label = `Displayed a ${readable(card.content.output.block)} card`;
    }
    for (const step of turn.steps) if (step.tool && byExecution.has(step.id)) step.executionId = step.id;
    for (const id of turn.executionIds) {
      if (turn.steps.some(step => step.executionId === id)) continue;
      const exec = byExecution.get(id)!;
      turn.steps.push({ id, executionId: id, tool: exec.tool, label: toolLabel(exec.tool, "{}", exec.status === "running" && turn.live), state: exec.status === "failed" ? "failed" : exec.status === "completed" ? "done" : turn.live ? "running" : "ended" });
    }
  }
  return [...turns.values()].filter(t => t.request || t.items.length || t.steps.length);
}

export const turnFiles = (turn: DetailTurn) => turn.items.filter(item => item.content.type === "artifact");
export function turnStatus(turn: DetailTurn): string {
  if (turn.steps.some(s => s.state === "waiting")) return "Needs your decision";
  if (turn.live) return "Working";
  const statuses = turn.items.flatMap(i => i.outcome ? [i.outcome.status] : []);
  if (statuses.includes("partial")) return "Partial result";
  if (statuses.includes("failed") || statuses.includes("denied")) return "Could not complete";
  if (statuses.includes("cancelled")) return "Stopped";
  return "Turn ended";
}

/** These are links the answer actually contains, not an inferred list of
 * sources used and not proof that a linked claim was verified. */
export function answerLinks(items: OutputItem[]): Array<{ label: string; url: string }> {
  const links = new Map<string, string>();
  const add = (url: string, label: string) => { if (/^https?:\/\//i.test(url)) links.set(url, label || url); };
  const inlineText = (nodes: InlineNode[]): string => nodes.map(n => n.type === "text" ? n.text : n.type === "code" ? n.code : "content" in n ? inlineText(n.content) : "label" in n ? inlineText(n.label) : "").join("");
  const inline = (nodes: InlineNode[]) => { for (const n of nodes) { if (n.type === "link" && n.safe) add(n.url, inlineText(n.label)); if ("content" in n) inline(n.content); if ("label" in n) inline(n.label); } };
  const blocks = (entries: DocumentBlock[]) => { for (const b of entries) {
    if (b.type === "citations") b.items.forEach(i => add(i.url, i.title || i.label));
    if ((b.type === "paragraph" || b.type === "heading")) inline(b.content);
    if (b.type === "table") [...b.header, ...b.rows.flat()].forEach(inline);
    if (b.type === "quote" || b.type === "callout") blocks(b.blocks);
    if (b.type === "list") b.items.forEach(blocks);
  } };
  for (const item of items) {
    if (item.role === "user") continue;
    const doc = item.content.type === "document" ? item.content.document : item.content.type === "outcome" ? item.content.document : null;
    if (doc) blocks(doc.blocks);
  }
  return [...links].map(([url, label]) => ({ label, url }));
}
