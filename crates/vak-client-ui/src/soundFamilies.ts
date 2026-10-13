// The sound-family vocabulary on its own, with no imports: `store.ts` checks
// a stored family while it loads, before `soundCues.ts` (which reads the
// store) can have run.

/** The interface sound families (docs/brand/media, Batch 3). Each plays the
 * same moments in one instrument voice; the files are
 * `public/assets/sounds/<family>/<moment>.mp3`. */
export const SOUND_FAMILIES = [
  { id: "glass", label: "Glass" },
  { id: "santoor", label: "Santoor" },
  { id: "brass", label: "Brass bell" },
  { id: "wood", label: "Wood" },
  { id: "pure", label: "Pure tone" },
] as const;
export type SoundFamily = (typeof SOUND_FAMILIES)[number]["id"];

export function isSoundFamily(value: unknown): value is SoundFamily {
  return SOUND_FAMILIES.some((family) => family.id === value);
}
