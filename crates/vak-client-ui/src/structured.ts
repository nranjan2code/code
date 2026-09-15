import type { StructuredOutput } from "./types";

export type AssistantPart = { type: "text"; text: string } | { type: "card"; output: StructuredOutput; source: string };

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

  const fences = /^```(\w*)[^\n]*\n([\s\S]*?)(^```[^\n]*(?:\n|$)|(?![\s\S]))/gm;
  let cursor = 0;
  for (const match of text.matchAll(fences)) {
    processSegment(text.slice(cursor, match.index));
    const explicit = match[1] === "vak" || ((match[1] === "json" || !match[1]) && match[2].includes('"semantic_type"'));
    const output = explicit ? parseVakFence(match[2]) : null;
    if (output) appendCard(output, match[2]);
    else if (!(explicit && streaming && !match[3])) appendText(match[0]);
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
    // Stop-policy nudges are model-visible ledger entries, but are control
    // traffic rather than something a person should see as their own message.
    .replace(/\[stop-guard\]:[\s\S]*?(?:Please continue\.?|$)/gi, "")
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
