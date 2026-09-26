/** The sizes the brand generator writes for each character's portrait and
 * expression-atlas frame (scripts/brand/generate.mjs). */
export const CHARACTER_TIERS = [64, 128, 256] as const;
export type CharacterTier = (typeof CHARACTER_TIERS)[number];

/** The smallest copy that covers `pixels` device pixels, or the largest when
 * none does, so a 24px mark never loads art drawn for 256px. */
export function characterTier(pixels: number): CharacterTier {
  return CHARACTER_TIERS.find((tier) => tier >= pixels) ?? CHARACTER_TIERS[CHARACTER_TIERS.length - 1];
}
