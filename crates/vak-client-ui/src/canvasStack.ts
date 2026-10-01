// The Canvas of each conversation: the subjects opened in it, which is in
// front, and the state a reader would be annoyed to lose by looking at
// something else (the view, a selection, an unsent note). Pure and immutable so
// the rules are testable without the UI (docs/plans/canvas-plan.md K2).

import { subjectKey, type CanvasSubject } from "./canvasSubject.ts";
import type { Selection } from "./canvasSelection.ts";

export type CanvasMode = "split" | "focused";

/** Which of the comment area's tabs is open. */
export type ContextPanel = "discussion" | "activity" | "changes";

export interface CanvasEntry {
  key: string;
  subject: CanvasSubject;
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
    entries = current.map((entry) => (entry.key === key ? { ...entry, subject } : entry));
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
