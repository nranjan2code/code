/** A selected path must remain one token, including spaces and punctuation. */
export function fileMention(path: string): string {
  return `@${/^[A-Za-z0-9_./-]+$/.test(path) && !path.endsWith(".") ? path : JSON.stringify(path)}`;
}

export function detectMention(text: string, caret: number): { start: number; query: string } | null {
  const match = /(^|[\s(])@("(?:[^"\\]|\\.)*|[\p{L}\p{M}\p{N}._/-]*)$/u.exec(text.slice(0, caret));
  if (!match) return null;
  let query = match[2];
  if (query.startsWith('"')) {
    try { query = JSON.parse(`${query}"`) as string; }
    catch { return null; }
  }
  return { start: caret - match[2].length - 1, query };
}
