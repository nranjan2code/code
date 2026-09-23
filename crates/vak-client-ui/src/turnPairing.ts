import type { OutputItem } from "./types";

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
