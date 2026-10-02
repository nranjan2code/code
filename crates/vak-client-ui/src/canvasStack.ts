// The Canvas of each conversation: the subjects opened in it, which is in
// front, and the state a reader would be annoyed to lose by looking at
// something else (the view, a selection, an unsent note). Pure and immutable so
// the rules are testable without the UI (docs/plans/canvas-plan.md K2).
//
// The server keeps each conversation's Canvas so every surface shows the same
// one (docs/design/66 §0a). A change is an operation (`CanvasOp`): applied
// here at once, and applied again on a newer Canvas when another surface
// wrote first. These are the only rules for what a change does; the server
// stores what they produce.

import { isCanvasSubject, sameSubject, subjectKey, type CanvasSubject } from "./canvasSubject.ts";
import { sameSelection, type Selection } from "./canvasSelection.ts";

export type CanvasMode = "split" | "focused";

/** Which of the comment area's tabs is open. */
export type ContextPanel = "discussion" | "activity" | "changes";

export interface CanvasEntry {
  key: string;
  subject: CanvasSubject;
  /** Beside the conversation or on the whole window: this device's choice,
   *  never shared, because a phone and a desktop lay out differently. */
  mode: CanvasMode;
  /** Which of the viewer's views is showing; null when it has only one. */
  view: string | null;
  /** What the reader has pointed at, which a comment would be about. */
  selection: Selection | null;
  /** The note being written for the Agent, not yet sent. */
  draft: string;
  feedbackOpen: boolean;
  panel: ContextPanel;
  /** The newest version whose arrival the reader has dismissed. */
  dismissedVersion: number;
}

export interface ConversationCanvas {
  entries: CanvasEntry[];
  /** `key` of the entry in front. */
  active: string;
}

export type CanvasStacks = Readonly<Record<string, ConversationCanvas>>;

/** More than this and the strip stops being a strip; the oldest unused one goes. */
export const MAX_ENTRIES = 8;

export function activeEntry(stacks: CanvasStacks, conversation: string): CanvasEntry | null {
  const canvas = stacks[conversation];
  return canvas?.entries.find((entry) => entry.key === canvas.active) ?? null;
}

export function entriesOf(stacks: CanvasStacks, conversation: string): readonly CanvasEntry[] {
  return stacks[conversation]?.entries ?? [];
}

/**
 * Opens a subject in a conversation's Canvas and brings it to the front. One
 * already open is reused, keeping what the reader did in it; only its subject
 * is replaced (so a citation can move to another place in the file).
 * `fresh` supplies the state of a subject that was not open yet.
 */
export function openEntry(
  stacks: CanvasStacks,
  conversation: string,
  subject: CanvasSubject,
  fresh: (subject: CanvasSubject) => Omit<CanvasEntry, "key" | "subject">,
): CanvasStacks {
  const key = subjectKey(subject);
  const current = stacks[conversation]?.entries ?? [];
  let entries: CanvasEntry[];
  if (current.some((entry) => entry.key === key)) {
    // The same subject again is the same thing to read: nothing is read again.
    entries = current.map((entry) => (entry.key === key && !sameSubject(entry.subject, subject) ? { ...entry, subject } : entry));
  } else {
    entries = [...current, { key, subject, ...fresh(subject) }];
    while (entries.length > MAX_ENTRIES) {
      const oldest = entries.findIndex((entry) => entry.key !== key);
      entries = entries.filter((_, index) => index !== oldest);
    }
  }
  return { ...stacks, [conversation]: { entries, active: key } };
}

export function activateEntry(stacks: CanvasStacks, conversation: string, key: string): CanvasStacks {
  const canvas = stacks[conversation];
  if (!canvas || canvas.active === key || !canvas.entries.some((entry) => entry.key === key)) return stacks;
  return { ...stacks, [conversation]: { ...canvas, active: key } };
}

/** Closes one entry; the one before it (or after, if first) comes to the front. */
export function closeEntry(stacks: CanvasStacks, conversation: string, key: string): CanvasStacks {
  const canvas = stacks[conversation];
  const index = canvas?.entries.findIndex((entry) => entry.key === key) ?? -1;
  if (!canvas || index < 0) return stacks;
  const entries = canvas.entries.filter((entry) => entry.key !== key);
  if (entries.length === 0) {
    const { [conversation]: _closed, ...rest } = stacks;
    return rest;
  }
  const active = canvas.active === key ? entries[Math.max(0, index - 1)].key : canvas.active;
  return { ...stacks, [conversation]: { entries, active } };
}

export function updateEntry(stacks: CanvasStacks, conversation: string, key: string, patch: Partial<Omit<CanvasEntry, "key" | "subject">>): CanvasStacks {
  const canvas = stacks[conversation];
  if (!canvas?.entries.some((entry) => entry.key === key)) return stacks;
  return {
    ...stacks,
    [conversation]: { ...canvas, entries: canvas.entries.map((entry) => (entry.key === key ? { ...entry, ...patch } : entry)) },
  };
}

/** What a reader changes in a tab; `mode` is this device's own and is changed
 *  without being shared. */
export type EntryPatch = Partial<Omit<CanvasEntry, "key" | "subject">>;

/** One change to a conversation's Canvas. */
export type CanvasOp =
  | { op: "open"; subject: CanvasSubject }
  | { op: "activate"; key: string }
  | { op: "close"; key: string }
  | { op: "update"; key: string; patch: EntryPatch };

export type FreshEntry = (subject: CanvasSubject) => Omit<CanvasEntry, "key" | "subject">;

export function applyOp(stacks: CanvasStacks, conversation: string, op: CanvasOp, fresh: FreshEntry): CanvasStacks {
  switch (op.op) {
    case "open": return openEntry(stacks, conversation, op.subject, fresh);
    case "activate": return activateEntry(stacks, conversation, op.key);
    case "close": return closeEntry(stacks, conversation, op.key);
    case "update": return updateEntry(stacks, conversation, op.key, op.patch);
  }
}

/** Whether an operation changes anything the other surfaces see. */
export function isShared(op: CanvasOp): boolean {
  return op.op !== "update" || Object.keys(op.patch).some((field) => field !== "mode");
}

/** A Canvas as the server keeps it: every tab, without this device's layout. */
export interface StoredCanvasShape {
  entries: Record<string, unknown>[];
  active: string | null;
}

export function toStored(canvas: ConversationCanvas | undefined): StoredCanvasShape {
  if (!canvas) return { entries: [], active: null };
  return {
    entries: canvas.entries.map(({ mode: _mode, ...shared }) => shared as unknown as Record<string, unknown>),
    active: canvas.active,
  };
}

const PANELS: readonly ContextPanel[] = ["discussion", "activity", "changes"];

function storedSelection(value: unknown): Selection | null {
  if (!value || typeof value !== "object") return null;
  const selection = value as Record<string, unknown>;
  if (selection.kind === "anchor" && typeof selection.anchor === "string" && selection.anchor) return { kind: "anchor", anchor: selection.anchor };
  if (selection.kind === "lines" && Number.isInteger(selection.start) && (selection.end === undefined || Number.isInteger(selection.end))) {
    return selection.end === undefined ? { kind: "lines", start: selection.start as number } : { kind: "lines", start: selection.start as number, end: selection.end as number };
  }
  return null;
}

function sameEntry(a: CanvasEntry, b: CanvasEntry): boolean {
  const left = a as unknown as Record<string, unknown>;
  const right = b as unknown as Record<string, unknown>;
  const fields = new Set([...Object.keys(left), ...Object.keys(right)]);
  for (const field of fields) {
    if (field === "subject" ? !sameSubject(a.subject, b.subject) : field === "selection" ? !sameSelection(a.selection, b.selection) : left[field] !== right[field]) return false;
  }
  return true;
}

/**
 * A Canvas read from the server, as this device shows it. Each tab keeps the
 * layout this device gave it, and a tab that has not changed stays the same
 * object, so nothing already drawn is drawn or read again. A tab this client
 * cannot show is left out. `null` when no tab is open.
 */
export function fromStored(stored: { entries: readonly unknown[]; active: string | null }, local: ConversationCanvas | undefined, fresh: FreshEntry): ConversationCanvas | null {
  const entries: CanvasEntry[] = [];
  for (const raw of stored.entries) {
    if (!raw || typeof raw !== "object") continue;
    const value = raw as Record<string, unknown>;
    if (typeof value.key !== "string" || !isCanvasSubject(value.subject) || subjectKey(value.subject) !== value.key) continue;
    if (entries.some((entry) => entry.key === value.key)) continue;
    const previous = local?.entries.find((entry) => entry.key === value.key);
    const base = fresh(value.subject);
    const next: CanvasEntry = {
      ...(value as object),
      key: value.key,
      subject: previous && sameSubject(previous.subject, value.subject) ? previous.subject : value.subject,
      mode: previous?.mode ?? base.mode,
      view: typeof value.view === "string" || value.view === null ? value.view : base.view,
      selection: storedSelection(value.selection),
      draft: typeof value.draft === "string" ? value.draft : "",
      feedbackOpen: typeof value.feedbackOpen === "boolean" ? value.feedbackOpen : base.feedbackOpen,
      panel: PANELS.includes(value.panel as ContextPanel) ? (value.panel as ContextPanel) : "discussion",
      dismissedVersion: typeof value.dismissedVersion === "number" ? value.dismissedVersion : 0,
    };
    entries.push(previous && sameEntry(previous, next) ? previous : next);
  }
  if (entries.length === 0) return null;
  const active = stored.active && entries.some((entry) => entry.key === stored.active) ? stored.active : entries[entries.length - 1].key;
  if (local && local.active === active && local.entries.length === entries.length && local.entries.every((entry, index) => entry === entries[index])) return local;
  return { entries, active };
}

/** Puts a conversation's Canvas as `canvas` says, or removes it. */
export function withCanvas(stacks: CanvasStacks, conversation: string, canvas: ConversationCanvas | null): CanvasStacks {
  if (stacks[conversation] === canvas) return stacks;
  if (!canvas) {
    if (!(conversation in stacks)) return stacks;
    const { [conversation]: _gone, ...rest } = stacks;
    return rest;
  }
  return { ...stacks, [conversation]: canvas };
}
