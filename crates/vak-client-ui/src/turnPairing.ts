import type { InlineNode, OutputItem, OutputTimeline } from "./types";

function inlineLabel(nodes: InlineNode[]): string {
  return nodes.map(node => {
    if (node.type === "text") return node.text;
    if (node.type === "code") return node.code;
    if (node.type === "link") return inlineLabel(node.label);
    if ("content" in node) return inlineLabel(node.content);
    if (node.type === "image") return node.alt;
    if (node.type === "soft_break" || node.type === "hard_break") return " ";
    return "";
  }).join("");
}

/** Recorded answer titles, joined by directive identity, never inferred from request words. */
export function turnDisplayLabel(entryId: string | undefined, timeline: OutputTimeline | null | undefined): string {
  if (!timeline) return "";
  const turn = serverTurnFor(entryId, timeline.items);
  if (!turn) return "";
  const items = timeline.items.filter(item => item.turn_id === turn);
  for (const item of items) {
    if (item.role !== "assistant" || item.content.type !== "document") continue;
    for (const block of item.content.document.blocks) {
      if (block.type === "heading") {
        const label = inlineLabel(block.content).trim();
        if (label) return label;
      }
    }
  }
  const titles = items.flatMap(item => {
    if (item.kind !== "card" || item.provenance?.source === "derived" || item.content.type !== "structured") return [];
    const payload = item.content.output.payload;
    const title = payload.title || payload.label || payload.headline;
    return typeof title === "string" && title.trim() ? [title.trim()] : [];
  });
  if (titles.length) return [...new Set(titles)].join(" · ");
  return "";
}

/** A sidebar names the newest turn, with its request as the untitled fallback. */
export function latestWorkLabel(timeline: OutputTimeline | null | undefined): string {
  const request = timeline?.items.findLast(item => item.role === "user");
  if (!request || !timeline) return "";
  return turnDisplayLabel(request.provenance?.entry_id ?? undefined, timeline)
    || request.fallback_text?.trim() || "";
}

/**
 * The server turn a chat turn corresponds to, found by the ledger entry of the
 * user message that started it — never by position. The client and the server
 * count turns independently, and one disagreement (a message one side counted
 * and the other did not) would otherwise displace every later turn.
 *
 * Returns null when the chat turn has no entry id yet (a message just sent,
 * before the transcript is hydrated) or the projection does not have it.
 */
export function serverTurnFor(entryId: string | undefined, items: readonly OutputItem[]): string | null {
  if (!entryId) return null;
  return items.find((item) => item.role === "user" && item.provenance?.entry_id === entryId)?.turn_id ?? null;
}

/** A settled turn may leave useful artifacts or an error without final prose. */
export function hasSettledProjection(items: readonly OutputItem[]): boolean {
  return items.some((item) =>
    (item.role === "assistant" && (["document", "structured", "adaptive"].includes(item.content.type) || item.kind === "error")) ||
    item.kind === "artifact" ||
    item.content.type === "structured" ||
    item.content.type === "adaptive"
  );
}
