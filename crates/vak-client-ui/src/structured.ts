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

/** Decode explicit transport fences without guessing a result from prose.
 * Unfinished vak fences wait for completion; invalid completed data remains inspectable. */
export function assistantParts(text: string, streaming = false): AssistantPart[] {
  const parts: AssistantPart[] = [];
  const appendText = (value: string) => {
    const cleaned = cleanAssistantText(value);
    if (cleaned) parts.push({ type: "text", text: cleaned });
  };
  const appendCard = (output: StructuredOutput, source: string) => {
    parts.push({ type: "card", output, source });
  };

  const processSegment = (segment: string) => {
    let idx = 0;
    while (idx < segment.length) {
      const openBrace = segment.indexOf("{", idx);
      if (openBrace === -1) {
        appendText(segment.slice(idx));
        break;
      }

      if (!segment.slice(openBrace).includes('"semantic_type"')) {
        appendText(segment.slice(idx));
        break;
      }

      let depth = 0;
      let inString = false;
      let escape = false;
      let closeBrace = -1;

      for (let i = openBrace; i < segment.length; i++) {
        const ch = segment[i];
        if (escape) {
          escape = false;
          continue;
        }
        if (ch === "\\") {
          escape = true;
          continue;
        }
        if (ch === '"') {
          inString = !inString;
          continue;
        }
        if (!inString) {
          if (ch === "{") depth++;
          else if (ch === "}") {
            depth--;
            if (depth === 0) {
              closeBrace = i;
              break;
            }
          }
        }
      }

      if (closeBrace === -1) {
        if (streaming && segment.slice(openBrace).includes('"semantic_type"')) {
          appendText(segment.slice(idx, openBrace));
          return;
        }
        appendText(segment.slice(idx));
        break;
      }

      const candidateJson = segment.slice(openBrace, closeBrace + 1);
      if (candidateJson.includes('"semantic_type"')) {
        const output = parseVakFence(candidateJson);
        if (output) {
          appendText(segment.slice(idx, openBrace));
          appendCard(output, candidateJson);
          idx = closeBrace + 1;
          continue;
        }
      }

      appendText(segment.slice(idx, openBrace + 1));
      idx = openBrace + 1;
    }
  };

  const fences = /^[ \t]*```(\w*)[^\n]*\r?\n([\s\S]*?)(^[ \t]*```[^\n]*(?:\r?\n|$)|(?![\s\S]))/gim;
  let cursor = 0;
  for (const match of text.matchAll(fences)) {
    processSegment(text.slice(cursor, match.index));
    const lang = (match[1] || "").toLowerCase().trim();
    const explicit = lang === "vak" || ((lang === "json" || !lang) && match[2].includes('"semantic_type"'));
    const output = explicit ? parseVakFence(match[2]) : null;
    if (output) {
      appendCard(output, match[2]);
    } else if (explicit) {
      // If it's an explicit vak/semantic transport fence:
      // While streaming and not closed, suppress it so raw incomplete JSON doesn't flicker on screen.
      // If completed, attempt a relaxed parse; never dump raw control fence JSON into user chat prose.
      // But a fence that never parses (e.g. malformed/truncated JSON from the
      // model) must still leave a trace — silently appending nothing here
      // produced a turn with a real response that rendered as a blank div,
      // no error, no explanation. Appending a plain note keeps the "no raw
      // JSON in chat" rule while ending the total silence.
      if (!streaming) {
        const recovered = parseVakFence(match[2]);
        if (recovered) {
          appendCard(recovered, match[2]);
        } else {
          appendText("This response could not be rendered — the result was malformed.");
        }
      }
    } else {
      appendText(match[0]);
    }
    cursor = match.index! + match[0].length;
  }
  processSegment(text.slice(cursor));
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
    // Stop-policy and repair directives are model-visible control traffic,
    // not user or assistant messages.
    .replace(/\[stop-(?:guard|hook)[^\]]*\]:[\s\S]*?(?:Please continue\.?|$)/gi, "")
    .replace(/\[repair directive\][\s\S]*$/gi, "")
    .replace(/\[(?:recovery|post-tool-use hook)\][\s\S]*$/gi, "")
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
    // Strip transport-layer code fences so structured cards don't leave raw JSON artifacts in prose.
    .replace(/^[ \t]*```(?:vak|json)[^\n]*\r?\n[\s\S]*?(?:^[ \t]*```[^\n]*(?:\r?\n|$)|$)/gim, (fence) => {
      return fence.includes('"semantic_type"') ? "" : fence;
    })
    // Keep the chat focused on the result. Execution narration belongs in
    // Workbench and approval details, not in the assistant's answer bubble.
    .replace(/^\s*I will write and execute this within the sandbox[^\n]*\.?\s*$/gim, "")
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
    .filter((line) => !/^\s*\[(?:stop-(?:guard|hook)|repair directive|recovery|post-tool-use hook)/i.test(line))
    .join("\n")
    .replace(/^\s*\n+|\n+\s*$/g, "")
    .trim();
}

/**
 * Attempts to parse a raw JSON fragment as a valid StructuredOutput card.
 * Handles leading/trailing whitespace, surrounding markdown, and nested JSON.
 */
export function parseVakFence(rawContent: string): StructuredOutput | null {
  try {
    let trimmed = rawContent.trim();
    if (trimmed.startsWith("```")) {
      trimmed = trimmed.replace(/^```[^\n]*\r?\n?/, "").replace(/\r?\n?```$/, "").trim();
    }
    const firstBrace = trimmed.indexOf("{");
    const lastBrace = trimmed.lastIndexOf("}");
    if (firstBrace !== -1 && lastBrace > firstBrace) {
      trimmed = trimmed.slice(firstBrace, lastBrace + 1);
    }
    if (!trimmed.startsWith("{") || !trimmed.endsWith("}")) return null;
    const parsed = JSON.parse(trimmed);
    if (
      parsed &&
      typeof parsed === "object" &&
      typeof parsed.semantic_type === "string" &&
      (parsed.schema_version === undefined || Number(parsed.schema_version) >= 1) &&
      parsed.payload &&
      typeof parsed.payload === "object" &&
      !Array.isArray(parsed.payload)
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
