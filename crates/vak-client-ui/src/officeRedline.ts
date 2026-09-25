// Tracked changes arrive in Office projections as text markers written by
// the server's reader ("[inserted by Mira: text]", "[deleted by Mira: text]",
// "[hidden: text]"). They are parsed into typed segments here and drawn as
// elements; document text is never mounted as HTML (docs/design/72).

export type RedlineSegment =
  | { kind: "text"; text: string }
  | { kind: "inserted" | "deleted"; author: string; text: string }
  | { kind: "hidden" | "white"; text: string };

const MARKER = /\[(inserted by|deleted by|hidden|white text)(?: ([^:\]]+))?: ([^\]]*)\]/g;

export function parseRedline(value: string): RedlineSegment[] {
  const segments: RedlineSegment[] = [];
  let cursor = 0;
  for (const match of value.matchAll(MARKER)) {
    const index = match.index ?? 0;
    if (index > cursor) segments.push({ kind: "text", text: value.slice(cursor, index) });
    const [, kind, author, text] = match;
    if (kind === "inserted by") segments.push({ kind: "inserted", author: author ?? "", text });
    else if (kind === "deleted by") segments.push({ kind: "deleted", author: author ?? "", text });
    else if (kind === "hidden") segments.push({ kind: "hidden", text });
    else segments.push({ kind: "white", text });
    cursor = index + match[0].length;
  }
  if (cursor < value.length) segments.push({ kind: "text", text: value.slice(cursor) });
  return segments;
}

