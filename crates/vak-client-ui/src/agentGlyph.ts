// Canonical built-in companion package. Character ids are persisted as part of
// an Agent's identity, so this registry is shared by every product surface.
const CHARACTER_BASE = `${import.meta.env.BASE_URL}characters/`;

export const AGENT_CHARACTERS = {
  vak: { name: "Vak", kind: "songbird", personality: "Attentive, capable, and reassuring", image: `${CHARACTER_BASE}vak.png`, hue: 238, cue: [392, 587] },
  mira: { name: "Mira", kind: "fox", personality: "Warm, perceptive, and quietly confident", image: `${CHARACTER_BASE}mira.png`, hue: 48, cue: [392, 523] },
  moss: { name: "Moss", kind: "forest friend", personality: "Patient, grounded, and thoughtful", image: `${CHARACTER_BASE}moss.png`, hue: 132, cue: [330, 392] },
  nori: { name: "Nori", kind: "seal", personality: "Curious, practical, and encouraging", image: `${CHARACTER_BASE}nori.png`, hue: 215, cue: [440, 587] },
  pip: { name: "Pip", kind: "penguin", personality: "Bright, attentive, and resourceful", image: `${CHARACTER_BASE}pip.png`, hue: 222, cue: [523, 659] },
  lumi: { name: "Lumi", kind: "cloud friend", personality: "Gentle, imaginative, and reassuring", image: `${CHARACTER_BASE}lumi.png`, hue: 255, cue: [494, 740] },
  tavi: { name: "Tavi", kind: "owl", personality: "Observant, measured, and wise", image: `${CHARACTER_BASE}tavi.png`, hue: 43, cue: [349, 466] },
  beni: { name: "Beni", kind: "rabbit", personality: "Steady, calm, and dependable", image: `${CHARACTER_BASE}beni.png`, hue: 348, cue: [294, 392] },
} as const;

export type AgentCharacter = keyof typeof AGENT_CHARACTERS;
export const AGENT_CHARACTER_IDS = Object.keys(AGENT_CHARACTERS) as AgentCharacter[];

export function agentCharacter(character: string | undefined) {
  const id = character && character in AGENT_CHARACTERS ? character as AgentCharacter : "vak";
  return AGENT_CHARACTERS[id];
}

export function agentGlyph(character: string | undefined): string {
  return agentCharacter(character).name.slice(0, 1);
}
