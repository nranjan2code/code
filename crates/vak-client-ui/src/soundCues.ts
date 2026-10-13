import { uiPreferences } from "./store";

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
export type SoundMoment = "listening" | "turn-finished" | "approval-needed" | "error" | "notification";

export function isSoundFamily(value: unknown): value is SoundFamily {
  return SOUND_FAMILIES.some((family) => family.id === value);
}

/** Overnight, when the person chose quiet hours. */
export function inQuietHours(at = new Date()): boolean {
  const hour = at.getHours();
  return uiPreferences.quietHours === "22-07" && (hour >= 22 || hour < 7);
}

const players = new Map<string, HTMLAudioElement>();

/** Plays one moment in the chosen family. Quiet hours silence everything
 * except a request for approval, which waits for the person. Best-effort:
 * a blocked or missing sound never affects the work. */
export function playSound(moment: SoundMoment, family: SoundFamily = uiPreferences.soundFamily) {
  if (moment !== "approval-needed" && inQuietHours()) return;
  play(`${import.meta.env.BASE_URL}assets/sounds/${family}/${moment}.mp3`);
}

/** A companion's signature: its two cue notes on its own instrument
 * (docs/brand/media/tools/tune_companions.py), played on a deliberate click,
 * so quiet hours do not apply. */
export function playCompanionSound(character: string) {
  play(`${import.meta.env.BASE_URL}assets/sounds/companions/${character}.mp3`);
}

function play(src: string) {
  if (!uiPreferences.soundCues || typeof Audio === "undefined") return;
  let player = players.get(src);
  if (!player) {
    player = new Audio(src);
    player.preload = "auto";
    player.volume = 0.6;
    players.set(src, player);
  }
  player.currentTime = 0;
  void player.play().catch(() => undefined);
}
