import type { OutputItem, PresentationDocument } from "./types";

export function answerDocument(item: OutputItem): PresentationDocument | undefined {
  if (item.role !== "assistant") return undefined;
  if (item.content.type === "document") return item.content.document;
  if (item.content.type === "outcome") return item.content.document ?? undefined;
  return undefined;
}

function isResultPart(item: OutputItem): boolean {
  if (item.role === "user") return false;
  return Boolean(answerDocument(item)) || item.content.type === "structured" ||
    item.content.type === "adaptive" || item.kind === "artifact";
}

export type AnswerGroup<T> = { kind: "result"; entries: T[]; resultId?: string } | { kind: "item"; entry: T };

/** Group adjacent result material without moving or dropping any projected
 * prose. A known result id or a non-result item is a composition boundary. */
export function groupAnswerEntries<T extends { item: OutputItem }>(entries: readonly T[]): AnswerGroup<T>[] {
  const groups: AnswerGroup<T>[] = [];
  let pending: T[] = [];
  let resultId: string | undefined;
  const flush = () => {
    if (pending.length) groups.push({ kind: "result", entries: pending, resultId });
    pending = [];
    resultId = undefined;
  };
  for (const entry of entries) {
    if (!isResultPart(entry.item)) {
      flush();
      groups.push({ kind: "item", entry });
      continue;
    }
    const nextId = entry.item.outcome?.result_id ?? undefined;
    if (resultId && nextId && resultId !== nextId) flush();
    pending.push(entry);
    resultId ??= nextId;
  }
  flush();
  return groups;
}

/** Card layout can arrange adjacent cards; a prose document is always a
 * boundary so an outline cannot move cards across what the model wrote. */
export function answerSegments<T extends { item: OutputItem }>(entries: readonly T[]): T[][] {
  const segments: T[][] = [];
  let cards: T[] = [];
  const flush = () => { if (cards.length) segments.push(cards); cards = []; };
  for (const entry of entries) {
    if (entry.item.kind === "artifact") continue;
    if (answerDocument(entry.item)) {
      flush();
      segments.push([entry]);
    } else {
      cards.push(entry);
    }
  }
  flush();
  return segments;
}
