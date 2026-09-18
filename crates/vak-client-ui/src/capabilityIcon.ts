// Deterministic per-item visual identity for MCP servers, skills, hooks, and
// plugins — the same idea as agentGlyph.ts, applied to Settings > Integrations
// so a "list of names" reads as a set of distinct things, not identical rows.
const HUES = [250, 150, 80, 210, 320, 20, 190, 280] as const;

function hashString(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;
  return h;
}

export function capabilityHue(name: string): number {
  return HUES[hashString(name) % HUES.length];
}

export function capabilityInitial(name: string): string {
  const trimmed = name.trim();
  return trimmed ? trimmed[0].toUpperCase() : "?";
}
