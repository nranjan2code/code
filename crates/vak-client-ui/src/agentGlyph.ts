// Shared per-character visual identity for agents. Every surface that shows
// an agent (sidebar list, chat header, fleet roster, settings, create wizard)
// renders the same glyph/color so an agent is recognizable at a glance
// wherever it appears — not just inside its own settings page.
export type AgentCharacter = "orb" | "leaf" | "sun" | "wave" | "spark";

export const AGENT_GLYPHS: Record<AgentCharacter, string> = {
  orb: "◌",
  leaf: "◒",
  sun: "☼",
  wave: "〰",
  spark: "✦",
};

export function agentGlyph(character: string | undefined): string {
  return AGENT_GLYPHS[(character as AgentCharacter) ?? "orb"] ?? AGENT_GLYPHS.orb;
}
