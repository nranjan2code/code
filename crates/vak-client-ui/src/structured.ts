import type { StructuredOutput } from "./types";

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
 * Handles common LLM syntax irregularities like missing commas or unescaped quotes.
 */
export function parseVakFence(rawContent: string): StructuredOutput | null {
  try {
    const trimmed = rawContent.trim();
    if (!trimmed.startsWith("{") || !trimmed.endsWith("}")) return null;
    let parsed: any = null;
    try {
      parsed = JSON.parse(trimmed);
    } catch {
      // 1. Fix missing comma between value and property name: e.g. text."key": -> text.", "key":
      const repaired = trimmed.replace(/([^\s,:{}\[\]])"([a-zA-Z0-9_]+)"\s*:/g, '$1", "$2":');
      try {
        parsed = JSON.parse(repaired);
      } catch {
        // 2. Fix trailing commas before closing braces/brackets
        const r2 = repaired.replace(/,\s*([\}\]])/g, "$1");
        try {
          parsed = JSON.parse(r2);
        } catch {}
      }
    }
    if (
      parsed &&
      typeof parsed === "object" &&
      typeof parsed.semantic_type === "string" &&
      parsed.payload &&
      typeof parsed.payload === "object"
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
 * Extracts structured cards from model text (both fenced ```vak / ```json and
 * un-fenced vak\n{"semantic_type": ...}), scrubs the JSON and scaffolding completely,
 * and returns the parsed cards along with clean user-facing prose.
 */
export function extractAssistantStructuredCards(text: string): { cards: StructuredOutput[]; displayText: string; leadText: string; followText: string } {
  const cards: StructuredOutput[] = [];
  let cleaned = text;

  // 1. Extract and strip closed or open code fences (```vak ... ``` or ```json ... ```)
  const fenceRegex = /```(?:vak|json)?\s*([\s\S]*?)(?:```|$)/gi;
  let fenceMatch: RegExpExecArray | null;
  while ((fenceMatch = fenceRegex.exec(text)) !== null) {
    if (fenceMatch[1].includes('"semantic_type"')) {
      const card = parseVakFence(fenceMatch[1]);
      if (card && !cards.some((c) => c.semantic_type === card.semantic_type && JSON.stringify(c.payload) === JSON.stringify(card.payload))) {
        cards.push(card);
      }
    }
  }
  cleaned = cleaned.replace(/```vak\s*[\s\S]*?(?:```|$)/gi, "");
  cleaned = cleaned.replace(/```(?:json)?\s*\{[\s\S]*?"semantic_type"[\s\S]*?\}\s*```/gi, "");

  // 2. Scan for un-fenced {"semantic_type" (with or without preceding "vak")
  let searchIdx = 0;
  while (true) {
    const needle = '"semantic_type"';
    const found = cleaned.indexOf(needle, searchIdx);
    if (found === -1) break;

    const start = cleaned.lastIndexOf("{", found);
    if (start === -1) {
      searchIdx = found + needle.length;
      continue;
    }

    // Identify any preceding "vak" token or newline right before the opening brace
    const beforeStr = cleaned.slice(0, start);
    const vakMatch = beforeStr.match(/(?:^|\n)\s*vak\s*$/i);
    const sliceStart = vakMatch ? beforeStr.lastIndexOf(vakMatch[0]) + (vakMatch[0].startsWith("\n") ? 1 : 0) : start;

    // Scan forward with brace balancing
    let depth = 0;
    let inString = false;
    let escape = false;
    let end = -1;
    for (let i = start; i < cleaned.length; i++) {
      const c = cleaned[i];
      if (escape) {
        escape = false;
        continue;
      }
      if (c === "\\") {
        escape = true;
        continue;
      }
      if (c === '"') {
        inString = !inString;
        continue;
      }
      if (!inString) {
        if (c === "{") depth++;
        else if (c === "}") {
          depth--;
          if (depth === 0) {
            end = i + 1;
            break;
          }
        }
      }
    }

    // Fallback if formatting or quote anomalies prevented depth from hitting 0
    if (end === -1) {
      const doubleNewline = cleaned.indexOf("\n\n", found);
      const closePattern = cleaned.lastIndexOf("}}", doubleNewline !== -1 ? doubleNewline : cleaned.length);
      if (closePattern > found) {
        end = closePattern + 2;
      }
    }

    if (end !== -1) {
      const rawJson = cleaned.slice(start, end);
      const card = parseVakFence(rawJson);
      if (card && !cards.some((c) => c.semantic_type === card.semantic_type && JSON.stringify(c.payload) === JSON.stringify(card.payload))) {
        cards.push(card);
      }
      cleaned = cleaned.slice(0, sliceStart) + cleaned.slice(end);
      searchIdx = sliceStart;
    } else {
      searchIdx = found + needle.length;
    }
  }

  // 3. Scrub remaining scaffolding and lifecycle metadata
  const isScaffoldingLeadIn = (str: string) => {
    const t = str.trim();
    if (!t) return false;
    if (t.endsWith(":") && t.length < 90) return true;
    if (/^(?:the|here\s+is|here\s+are|current|weather|result|details|summary|overview|breakdown|recipe|timeline)\b.*:?$/i.test(t) && t.length < 80) return true;
    return false;
  };

  let leadText = "";
  let followText = "";

  if (cards.length > 0) {
    const doubleNewline = cleaned.indexOf("\n\n");
    if (doubleNewline !== -1) {
      const firstPart = cleanAssistantText(cleaned.slice(0, doubleNewline));
      const secondPart = cleanAssistantText(cleaned.slice(doubleNewline + 2));
      leadText = isScaffoldingLeadIn(firstPart) ? "" : firstPart;
      followText = isScaffoldingLeadIn(secondPart) ? "" : secondPart;
    } else {
      const single = cleanAssistantText(cleaned);
      if (isScaffoldingLeadIn(single)) {
        leadText = "";
        followText = "";
      } else {
        leadText = single;
        followText = "";
      }
    }
    cleaned = [leadText, followText].filter(Boolean).join("\n\n").trim();
  } else {
    cleaned = cleanAssistantText(cleaned);
    leadText = cleaned;
    followText = "";
  }

  return { cards, displayText: cleaned, leadText, followText };
}
