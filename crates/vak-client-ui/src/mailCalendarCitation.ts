export type MailCalendarCitation = {
  accountId: string;
  threadId: string;
  messageId: string;
};

const MAX_CITATION_PART = 2048;

function validPart(value: string): boolean {
  return value.length > 0 && value.length <= MAX_CITATION_PART && !/[\u0000-\u001f\u007f]/.test(value);
}

/** Inline-code token that an Agent can use to cite a selected mail message. */
export function parseMailCalendarCitation(token: string): MailCalendarCitation | null {
  const match = /^mailcite:([^/]+)\/([^/]+)\/([^/]+)$/.exec(token.trim());
  if (!match) return null;
  try {
    const [, accountId, threadId, messageId] = match.map((part) => decodeURIComponent(part));
    if (![accountId, threadId, messageId].every(validPart)) return null;
    return { accountId, threadId, messageId };
  } catch {
    return null;
  }
}

