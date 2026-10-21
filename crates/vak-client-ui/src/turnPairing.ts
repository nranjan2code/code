import type { OutputItem, OutputTimeline } from "./types";

/** Use accepted plan names, joined by the directive's address, not page position. */
export function turnPlanLabel(entryId: string | undefined, timeline: OutputTimeline | null | undefined): string {
  if (!timeline) return "";
  const turn = serverTurnFor(entryId, timeline.items);
  if (!turn) return "";
  return timeline.answers?.find(answer => answer.turn_id === turn)?.sections
    .map(section => section.name.trim()).filter(Boolean).join(" · ") ?? "";
}

/** A sidebar names the newest request, never an older plan or first-message title. */
export function latestWorkLabel(timeline: OutputTimeline | null | undefined): string {
  const request = timeline?.items.findLast(item => item.role === "user");
  if (!request || !timeline) return "";
  const plan = timeline.answers?.find(answer => answer.turn_id === request.turn_id);
  return plan?.sections.map(section => section.name.trim()).filter(Boolean).join(" · ")
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
