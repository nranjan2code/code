import { agentCharacter } from "./agentGlyph";
import { uiPreferences } from "./store";

let context: AudioContext | undefined;

/** A short local signature played only after a deliberate character click. */
export function playCharacterCue(character: string) {
  if (!uiPreferences.soundCues) return;
  try {
    context ??= new AudioContext();
    if (context.state === "suspended") void context.resume();
    const start = context.currentTime;
    agentCharacter(character).cue.forEach((frequency, index) => {
      const oscillator = context!.createOscillator();
      const gain = context!.createGain();
      const at = start + index * 0.065;
      oscillator.type = "sine";
      oscillator.frequency.setValueAtTime(frequency, at);
      gain.gain.setValueAtTime(0, at);
      gain.gain.linearRampToValueAtTime(0.035, at + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + 0.11);
      oscillator.connect(gain).connect(context!.destination);
      oscillator.start(at);
      oscillator.stop(at + 0.12);
    });
  } catch {
    // Character previews remain fully usable when WebAudio is unavailable.
  }
}
