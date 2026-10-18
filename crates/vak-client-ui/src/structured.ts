import type { StructuredOutput } from "./types";

export type AssistantPart = { type: "text"; text: string } | { type: "card"; output: StructuredOutput; source: string };
export type CardPart = Extract<AssistantPart, { type: "card" }>;

/** A rendering unit built from `AssistantPart[]`: either prose, or one or
 * more cards the model emitted back-to-back with no prose between them. */
export type RenderGroup = { type: "text"; text: string } | { type: "card_group"; cards: CardPart[] };

/**
 * Groups consecutive `{type:"card"}` parts (no text part between them) into
 * a single `card_group` unit, so a multi-fence answer — e.g. a research
 * synthesis alongside a supporting chart and a comparison table — renders
 * as one connected, laid-out unit instead of unrelated full-width blocks
 * that happen to be adjacent in the DOM. A lone card between prose is still
 * a `card_group` of length 1, so callers only need to handle two cases
 * (text vs. card_group), not three.
 */
export function groupAssistantParts(parts: AssistantPart[]): RenderGroup[] {
  const groups: RenderGroup[] = [];
  for (const part of parts) {
    if (part.type === "text") {
      groups.push(part);
      continue;
    }
    const last = groups[groups.length - 1];
    if (last && last.type === "card_group") {
      last.cards.push(part);
    } else {
      groups.push({ type: "card_group", cards: [part] });
    }
  }
  return groups;
}

/** Split assistant text into prose and its ```vak fences (docs/design/92-card-forms.md).
 * Nothing outside a vak fence is a card. While streaming, an unclosed fence is held
 * back; a completed fence that does not parse says so instead of vanishing. */
export function assistantParts(text: string, streaming = false): AssistantPart[] {
  const parts: AssistantPart[] = [];
  const appendText = (value: string) => {
    const cleaned = cleanAssistantText(value);
    if (cleaned) parts.push({ type: "text", text: cleaned });
  };
  const fences = /^[ \t]*```vak[^\n]*\r?\n([\s\S]*?)(^[ \t]*```[^\n]*(?:\r?\n|$)|(?![\s\S]))/gim;
  let cursor = 0;
  for (const match of text.matchAll(fences)) {
    appendText(text.slice(cursor, match.index));
    cursor = (match.index ?? 0) + match[0].length;
    const closed = Boolean(match[2]);
    if (!closed && streaming) continue;
    const output = parseVakFence(match[1]);
    if (output) parts.push({ type: "card", output, source: match[1] });
    else appendText("This response could not be rendered — the result was malformed.");
  }
  appendText(text.slice(cursor));
  return parts;
}

/** Control/authorship metadata is filtered by the server projection. Text
 * itself is user content: examples, quoted markers and lifecycle words must
 * survive. Retained as the common whitespace boundary for existing views. */
export function stripControlScaffolding(text: string): string {
  return text.trim();
}

/** Preserve authored prose; typed vak fences are parsed by assistantParts. */
export function cleanAssistantText(text: string): string {
  return text.trim();
}

/** Speech uses the same visible prose as chat, never transport fences or
 * model-authored transcript blocks. A card-only result stays visible on
 * screen and gets a short review cue instead of reading raw JSON aloud. */
export function spokenReplyText(text: string): string {
  const prose = cleanAssistantText(text)
    .replace(/^[ \t]*```[^\n]*\r?\n[^]*?^[ \t]*```[^\n]*(?:\r?\n|$)/gim, "")
    .replace(/^#{1,6}[ \t]+(?:metric|dataset|chart|table)[ \t]*$/gim, "")
    .replace(/^#{1,6}[ \t]+/gm, "")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/[*_`]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  return prose || "The result is ready to review on screen.";
}

/** A ```vak fence's body as a card: `{"semantic_type", "payload"}`, strict JSON. */
export function parseVakFence(rawContent: string): StructuredOutput | null {
  try {
    const parsed = JSON.parse(rawContent.trim());
    if (
      parsed && typeof parsed === "object" && typeof parsed.semantic_type === "string"
      && parsed.payload && typeof parsed.payload === "object" && !Array.isArray(parsed.payload)
    ) {
      return {
        semantic_type: parsed.semantic_type,
        block: parsed.semantic_type,
        schema_version: 2,
        skill_id: "core",
        skill_version: "2.0.0",
        payload: parsed.payload as Record<string, unknown>,
      };
    }
  } catch {
    // Not a card.
  }
  return null;
}

/**
 * Determines whether an assistant message is short, fleeting transitional commentary
 * (e.g. "I'll search for that...", "Let me check the files...") that can be tucked into
 * execution details when a subsequent substantive answer or card is present.
 * Substantive prose, reports, lists, headings, and actual answers are NEVER considered fleeting.
 */
export function isFleetingNarration(text: string): boolean {
  if (!text) return true;
  const cleaned = cleanAssistantText(text).trim();
  if (!cleaned) return true;
  if (cleaned.length > 180) return false;
  if (/^#{1,6}\s+/m.test(cleaned)) return false;
  if (/^[-*+]\s+/m.test(cleaned)) return false;
  if (/^\d+\.\s+/m.test(cleaned)) return false;
  if (/```|\|.*\|/.test(cleaned)) return false;
  if (cleaned.split(/\n\s*\n/).length > 1) return false;

  const lower = cleaned.toLowerCase();
  const transitionalStarters = [
    "i will ",
    "i'll ",
    "let me ",
    "looking into ",
    "searching ",
    "analyzing ",
    "running ",
    "checking ",
    "now checking ",
    "reading ",
    "writing ",
    "fetching ",
    "inspecting ",
    "querying ",
  ];
  return transitionalStarters.some((prefix) => lower.startsWith(prefix));
}
