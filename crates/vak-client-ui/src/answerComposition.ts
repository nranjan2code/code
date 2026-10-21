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

/** The Canvas opened from a result card follows that exact group, even when
 *  several partial/final result groups share one turn id. */
export function answerGroupItems<T extends { id: string; turn_id: string }>(items: readonly T[], turnId: string, itemIds?: readonly string[]): T[] {
  const selected = itemIds ? new Set(itemIds) : null;
  return items.filter((item) => item.turn_id === turnId && (!selected || selected.has(item.id)));
}

/** A delivered file already has a thumbnail and an Open action. Fold only
 * its exact file-preview duplicate into that card, within one result group.
 * Inline markup and ambiguous file identities remain independent content. */
export function consolidateFilePreviews<T extends { item: OutputItem }>(entries: readonly T[]): T[] {
  return entries.filter(({ item }) => {
    if (item.status !== "succeeded" || item.content.type !== "structured" || item.content.output.block !== "file_preview") return true;
    const payload = item.content.output.payload;
    if (payload.html != null || typeof payload.artifact_path !== "string" || !payload.artifact_path) return true;
    const matches = entries.filter(({ item: file }) => {
      if (file.status !== "succeeded" || file.content.type !== "artifact") return false;
      const artifact = file.content.artifact;
      const saved = artifact.status?.state !== "in_folder" ? artifact.status?.saved_as : undefined;
      return artifact.path === payload.artifact_path || saved?.path === payload.artifact_path;
    });
    return matches.length !== 1;
  });
}

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

/** Intrinsic reading widths in rem. These describe forms, never subjects. */
export function formReadingWidth(block: string): number {
  switch (block) {
    case "table": case "comparison": case "diff": return 32;
    case "chart": case "map": case "calendar": case "controls": return 28;
    case "summary": case "sources": return 24;
    case "callout": case "link": return 16;
    default: return 20;
  }
}

/** Semantic order is canonical. Only explicit adjacent pairs may share a row;
 * stale/missing anchors, prose boundaries and another result cannot rebind it. */
export function answerRows<T extends { item: OutputItem }>(entries: readonly T[], pairs: readonly (readonly [string, string])[] = []): T[][] {
  const beside = new Map(pairs.map(([anchor, item]) => [item, anchor]));
  const rows: T[][] = [];
  for (const entry of entries) {
    const previous = rows.at(-1);
    if (previous?.length === 1 && entry.item.kind === "card" && previous[0].item.kind === "card"
        && (entry.item.placement?.beside
          ? entry.item.placement.beside === previous[0].item.provenance?.presentation_id
          : beside.get(entry.item.id) === previous[0].item.id)
        && entry.item.turn_id === previous[0].item.turn_id
        && entry.item.outcome?.result_id === previous[0].item.outcome?.result_id) previous.push(entry);
    else rows.push([entry]);
  }
  return rows;
}
