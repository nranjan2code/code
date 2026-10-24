/** Control/authorship metadata is filtered by the server projection. Text
 * itself is user content: examples, quoted markers and lifecycle words must
 * survive. Retained as the common whitespace boundary for existing views. */
export function stripControlScaffolding(text: string): string {
  return text.trim();
}

/** Preserve authored prose at the display whitespace boundary. */
export function cleanAssistantText(text: string): string {
  return text.trim();
}

/** Speech uses the same visible prose as chat, never transport fences or
 * model-authored transcript blocks. A card-only result stays visible on
 * screen and gets a short review cue instead of reading raw JSON aloud. */
export function spokenReplyText(text: string): string {
  const prose = cleanAssistantText(text)
    .replace(/^[ \t]*```[^\n]*\r?\n[^]*?^[ \t]*```[^\n]*(?:\r?\n|$)/gim, "")
    .replace(/^#{1,6}[ \t]+/gm, "")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/[*_`]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  return prose || "The result is ready to review on screen.";
}
