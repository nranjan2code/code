// What a reader has pointed at in the subject in front of them, and how it is
// said and sent (docs/plans/canvas-plan.md K4). A selection belongs to a
// viewer: source shows lines, a document shows the place a reader gave it. A
// place is never guessed where the viewer cannot name one.

export type Selection =
  | { kind: "lines"; start: number; end?: number }
  | { kind: "anchor"; anchor: string };

/** Which kind of selection a view of a subject can make. */
export type SelectionKind = Selection["kind"];

/** Where a comment says it is. Lines and anchors are alternatives. */
export interface CommentPlace {
  lineStart?: number;
  lineEnd?: number;
  anchor?: string;
}

export function commentPlace(selection: Selection | null): CommentPlace {
  if (!selection) return {};
  if (selection.kind === "anchor") return { anchor: selection.anchor };
  return { lineStart: selection.start > 0 ? selection.start : undefined, lineEnd: selection.end && selection.end > 0 ? selection.end : undefined };
}

/** The selection a saved comment points at, for putting the reader back on it. */
export function selectionOfComment(comment: { line_start?: number; line_end?: number; anchor?: string }): Selection | null {
  if (comment.anchor) return { kind: "anchor", anchor: comment.anchor };
  if (comment.line_start) return { kind: "lines", start: comment.line_start, end: comment.line_end && comment.line_end !== comment.line_start ? comment.line_end : undefined };
  return null;
}

/** In everyday words; an anchor is a reader's address and appears only when asked for. */
export function selectionLabel(selection: Selection, technical = false): string {
  if (selection.kind === "anchor") return technical ? `Selected place · ${selection.anchor}` : "Selected place";
  return selection.end && selection.end !== selection.start ? `Lines ${selection.start}–${selection.end}` : `Line ${selection.start}`;
}

export function selectionInvalid(selection: Selection | null): boolean {
  return selection?.kind === "lines" && (selection.start < 1 || (selection.end !== undefined && selection.end < selection.start));
}

export function sameSelection(a: Selection | null, b: Selection | null): boolean {
  if (!a || !b) return a === b;
  if (a.kind === "anchor" && b.kind === "anchor") return a.anchor === b.anchor;
  return a.kind === "lines" && b.kind === "lines" && a.start === b.start && (a.end ?? a.start) === (b.end ?? b.start);
}

/** The words a comment about a place adds to a request that has no comment to carry it. */
export function selectionForPrompt(selection: Selection | null): string {
  if (!selection) return "";
  return selection.kind === "anchor" ? ` The change is about the place ${selection.anchor}.` : ` The change is about ${selectionLabel(selection).toLowerCase()}.`;
}
