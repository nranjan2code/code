import type { StructuredOutput } from "./types";

export type AssistantPart = { type: "text"; text: string } | { type: "card"; output: StructuredOutput; source: string };

/** Decode explicit transport fences without guessing a result from prose.
 * Unfinished vak fences wait for completion; invalid completed data remains inspectable. */
export function assistantParts(text: string, streaming = false): AssistantPart[] {
  const parts: AssistantPart[] = [];
  const append = (value: string) => {
    const cleaned = cleanAssistantText(value);
    if (cleaned) parts.push({ type: "text", text: cleaned });
  };
  const fences = /^```(\w*)[^\n]*\n([\s\S]*?)(^```[^\n]*(?:\n|$)|(?![\s\S]))/gm;
  let cursor = 0;
  for (const match of text.matchAll(fences)) {
    append(text.slice(cursor, match.index));
    const explicit = match[1] === "vak" || ((match[1] === "json" || !match[1]) && match[2].includes('"semantic_type"'));
    const output = explicit ? parseVakFence(match[2]) : null;
    if (output) parts.push({ type: "card", output, source: match[2] });
    else if (!(explicit && streaming && !match[3])) append(match[0]);
    cursor = match.index! + match[0].length;
  }
  const tail = text.slice(cursor);
  const output = parseVakFence(tail.trim().replace(/^vak\s+/, ""));
  if (output) parts.push({ type: "card", output, source: tail });
  else append(tail);
  return parts;
}

/**
 * Strips prompt-scaffolding control blocks injected into the model-visible
 * context (e.g. <conversation_thread>, <context_summary>, <intent>,
 * <work_contract>, <managed_work>, <context_packet>) so they never leak into user or assistant views.
 */
export function stripControlScaffolding(text: string): string {
  if (!text) return "";
  return text
    .replace(/<conversation_thread[\s\S]*?(?:<\/conversation_thread>|$)/gi, "")
    .replace(/<context_summary[\s\S]*?(?:<\/context_summary>|$)/gi, "")
    .replace(/<intent[\s\S]*?(?:<\/intent>|$)/gi, "")
    .replace(/<work_contract[\s\S]*?(?:<\/work_contract>|$)/gi, "")
    .replace(/<managed_work[\s\S]*?(?:<\/managed_work>|$)/gi, "")
    .replace(/<context_packet[\s\S]*?(?:<\/context_packet>|$)/gi, "")
    .replace(/<system_reminder[\s\S]*?(?:<\/system_reminder>|$)/gi, "")
    .replace(/<runtime_guidance[\s\S]*?(?:<\/runtime_guidance>|$)/gi, "")
    .replace(/<scratchpad[\s\S]*?(?:<\/scratchpad>|$)/gi, "")
    .trim();
}

/**
 * Strips internal scaffolding, prompt leaking, and lifecycle metadata from assistant prose.
 */
export function cleanAssistantText(text: string): string {
  const normalized = stripControlScaffolding(text)
    .replace(/^\s*Surface:\s+(?:desktop app|web client)\.?(?:\s*)/gim, "")
    .replace(/(?:\r?\n)?\s*primary deliverable\s*:\s*(?:produced|completed|done)[\s\S]*$/gi, "")
    .replace(/(?:\r?\n)?\s*completed\s*$/gi, "")
    .replace(/^\s*Outcome:\s+[^\n]*(?:\n|$)/gim, "")
    .replace(/^\s*contract_id:\s+[^\n]*(?:\n|$)/gim, "")
    .replace(/^\s*vak(?:\r?\n|\s+|$)/gim, "");

  return normalized
    .split("\n")
    .filter((line) => !/^\s*Surface:\s+(?:desktop app|web client)\.?\s*$/i.test(line))
    .filter((line) => !/^\s*primary deliverable\s*:\s*(?:produced|completed|done)\s*$/i.test(line))
    .filter((line) => !/^\s*completed\s*$/i.test(line))
    .filter((line) => !/^\s*Outcome:\s+/i.test(line))
    .filter((line) => !/^\s*contract_id:\s+/i.test(line))
    .filter((line) => !/^\s*vak\s*$/i.test(line))
    .join("\n")
    .replace(/^\s*\n+|\n+\s*$/g, "")
    .trim();
}

/**
 * Attempts to parse a raw JSON fragment as a valid StructuredOutput card.
 * Invalid transport data is preserved as a fallback, never repaired into a claim.
 */
export function parseVakFence(rawContent: string): StructuredOutput | null {
  try {
    const trimmed = rawContent.trim();
    if (!trimmed.startsWith("{") || !trimmed.endsWith("}")) return null;
    const parsed = JSON.parse(trimmed);
    if (
      parsed &&
      typeof parsed === "object" &&
      typeof parsed.semantic_type === "string" &&
      (parsed.schema_version === undefined || parsed.schema_version === 2) &&
      parsed.payload &&
      typeof parsed.payload === "object" && !Array.isArray(parsed.payload)
    ) {
      return {
        semantic_type: parsed.semantic_type,
        schema_version: 2,
        skill_id: typeof parsed.skill_id === "string" ? parsed.skill_id : "built-in",
        skill_version: typeof parsed.skill_version === "string" ? parsed.skill_version : "1.0",
        payload: parsed.payload as Record<string, unknown>,
      };
    }
  } catch {
    // Incomplete or invalid JSON
  }
  return null;
}
