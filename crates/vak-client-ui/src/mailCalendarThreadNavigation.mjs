/** Maximum extra pages fetched automatically for a conversation citation. */
export const MAX_CITATION_PAGES = 20;

/**
 * Load bounded pages from one already-selected conversation until its cited
 * message is present, the provider cursor ends, or the automatic page budget
 * is spent. The caller supplies a loader bound to the same account/thread.
 *
 * @template T extends { provider_id: string }
 * @param {{ messages: T[], next_cursor: string | null }} firstPage
 * @param {string} targetMessageId
 * @param {(cursor: string) => Promise<{ messages: T[], next_cursor: string | null }>} loadNext
 * @param {number} [maxAdditionalPages]
 */
export async function loadConversationCitation(firstPage, targetMessageId, loadNext, maxAdditionalPages = MAX_CITATION_PAGES) {
  const messages = [...firstPage.messages];
  let nextCursor = firstPage.next_cursor ?? null;
  let additionalPages = 0;
  const seenCursors = new Set();
  while (
    !messages.some((message) => message.provider_id === targetMessageId) &&
    nextCursor &&
    additionalPages < Math.max(0, Math.min(MAX_CITATION_PAGES, maxAdditionalPages)) &&
    !seenCursors.has(nextCursor)
  ) {
    seenCursors.add(nextCursor);
    const page = await loadNext(nextCursor);
    const knownIds = new Set(messages.map((message) => message.provider_id));
    for (const message of page.messages) {
      if (!knownIds.has(message.provider_id)) {
        messages.push(message);
        knownIds.add(message.provider_id);
      }
    }
    additionalPages += 1;
    nextCursor = page.next_cursor ?? null;
  }
  return {
    messages,
    nextCursor,
    found: messages.some((message) => message.provider_id === targetMessageId),
    additionalPages,
  };
}
