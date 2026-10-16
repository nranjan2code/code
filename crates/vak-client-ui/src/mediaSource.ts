import { safeUrl } from "./safeUrl.ts";

/**
 * Where a media card's file comes from. A card's `url` is "the file's URL or
 * workspace path" (`emit_media_card`): an address with no scheme names a file
 * in the conversation's workspace, read through the authenticated file route,
 * which decides whether the path is inside it. Any other scheme is refused.
 */
export type MediaSource =
  | { kind: "web"; url: string }
  | { kind: "workspace"; path: string }
  | { kind: "refused" };

export function mediaSource(address: string): MediaSource {
  const value = address.trim();
  if (!value) return { kind: "refused" };
  if (safeUrl(value, true)) return { kind: "web", url: value };
  if (value.startsWith("//") || /^[a-z][a-z0-9+.-]*:/i.test(value)) return { kind: "refused" };
  return { kind: "workspace", path: value };
}
