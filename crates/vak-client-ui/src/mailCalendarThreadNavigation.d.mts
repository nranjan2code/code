export const MAX_CITATION_PAGES: number;

export function loadConversationCitation<T extends { provider_id: string }>(
  firstPage: { messages: T[]; next_cursor?: string | null },
  targetMessageId: string,
  loadNext: (cursor: string) => Promise<{ messages: T[]; next_cursor?: string | null }>,
  maxAdditionalPages?: number,
): Promise<{
  messages: T[];
  nextCursor: string | null;
  found: boolean;
  additionalPages: number;
}>;
