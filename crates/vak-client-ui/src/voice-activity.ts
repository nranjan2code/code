/** Live speech endpointing for the voice composer.
 *
 * Decides where an utterance starts and stops from 16 kHz microphone PCM.
 * Loudness alone cannot tell a person from a room, so a chunk is voiced only
 * when it stands above a quiet floor the detector keeps learning: calibrated
 * from the first moments of capture, falling quickly and rising slowly while
 * idle, and reset to the room's level whenever a closed utterance turns out
 * to carry no speech. The server re-measures every utterance with the same
 * constants (`crates/vak-voice/src/vad.rs`, which pins them) before it may
 * cost a provider call, so this detector shapes latency, never authority. */

export const ABSOLUTE_SPEECH_RMS = 0.018;
export const FLOOR_RATIO = 2.5;
export const MIN_VOICED_MS = 240;
export const FLOOR_PERCENTILE = 0.2;
/** Natural pauses inside one request must not split it into two turns. */
export const END_SILENCE_MS = 1_500;
/** An onset candidate is abandoned after this much quiet. */
export const ONSET_RESET_MS = 160;
/** No utterance runs longer than this, whatever the room does. */
export const MAX_UTTERANCE_MS = 30_000;
/** The floor is measured before any onset is accepted. */
export const CALIBRATION_MS = 300;
/** While an answer is playing, its own echo must not read as the user
 * interrupting: barge-in needs this much more level and duration. */
export const BARGE_IN_GUARD = 2;

const SAMPLE_RATE_HZ = 16_000;
const FLOOR_FALL = 0.3;
const FLOOR_RISE = 0.01;
const MIN_FLOOR = 0.001;

export type DetectorEvent =
  /** Nothing to send. */
  | { kind: "idle" }
  /** Speech began; send `lead` (the buffered onset, including this chunk) first. */
  | { kind: "started"; lead: Int16Array[] }
  /** Send this chunk as part of the open utterance. */
  | { kind: "continuing" }
  /** Send this chunk, then close the utterance. */
  | { kind: "stopped" };

interface Chunk { rms: number; ms: number }

function chunkRms(pcm: Int16Array): number {
  let energy = 0;
  for (const sample of pcm) {
    const value = sample / 32768;
    energy += value * value;
  }
  return pcm.length ? Math.sqrt(energy / pcm.length) : 0;
}

/** Voiced milliseconds measured against the utterance's own floor, exactly
 * as the server's `SpeechEvidence` does. */
export function voicedMs(chunks: Chunk[]): { voicedMs: number; floor: number } {
  if (!chunks.length) return { voicedMs: 0, floor: 0 };
  const sorted = chunks.map((chunk) => chunk.rms).sort((a, b) => a - b);
  const floor = sorted[Math.floor((sorted.length - 1) * FLOOR_PERCENTILE)];
  const threshold = Math.max(ABSOLUTE_SPEECH_RMS, floor * FLOOR_RATIO);
  return { voicedMs: chunks.reduce((sum, chunk) => sum + (chunk.rms >= threshold ? chunk.ms : 0), 0), floor };
}

export class SpeechDetector {
  private floor = ABSOLUTE_SPEECH_RMS / FLOOR_RATIO;
  private calibrationMs = 0;
  private calibrationFloor = Number.POSITIVE_INFINITY;
  private candidate: Int16Array[] = [];
  private candidateChunks: Chunk[] = [];
  private candidateVoicedMs = 0;
  private candidateQuietMs = 0;
  private open = false;
  private utterance: Chunk[] = [];
  private utteranceMs = 0;
  private quietMs = 0;

  get speaking(): boolean { return this.open; }

  /** Feed one captured chunk. `answerPlaying` raises the bar for an onset. */
  push(pcm: Int16Array, answerPlaying = false): DetectorEvent {
    if (!pcm.length) return { kind: "idle" };
    const chunk = { rms: chunkRms(pcm), ms: (pcm.length / SAMPLE_RATE_HZ) * 1_000 };
    if (this.calibrationMs < CALIBRATION_MS) {
      // The quietest calibration chunk, so a person who starts talking at
      // once does not teach the detector that speech is the room.
      this.calibrationMs += chunk.ms;
      this.calibrationFloor = Math.min(this.calibrationFloor, chunk.rms);
      if (this.calibrationMs >= CALIBRATION_MS) this.floor = Math.max(MIN_FLOOR, this.calibrationFloor);
      return { kind: "idle" };
    }
    if (!this.open) return this.listen(pcm, chunk, answerPlaying);
    this.utterance.push(chunk);
    this.utteranceMs += chunk.ms;
    this.quietMs = chunk.rms >= this.threshold() ? 0 : this.quietMs + chunk.ms;
    if (this.quietMs >= END_SILENCE_MS || this.utteranceMs >= MAX_UTTERANCE_MS) {
      this.close();
      return { kind: "stopped" };
    }
    return { kind: "continuing" };
  }

  private threshold(guard = 1): number {
    return Math.max(ABSOLUTE_SPEECH_RMS, this.floor * FLOOR_RATIO) * guard;
  }

  private listen(pcm: Int16Array, chunk: Chunk, answerPlaying: boolean): DetectorEvent {
    const guard = answerPlaying ? BARGE_IN_GUARD : 1;
    const voiced = chunk.rms >= this.threshold(guard);
    this.floor = Math.max(MIN_FLOOR, this.floor + (chunk.rms - this.floor) * (chunk.rms < this.floor ? FLOOR_FALL : FLOOR_RISE));
    if (voiced) {
      this.candidateVoicedMs += chunk.ms;
      this.candidateQuietMs = 0;
    } else if (this.candidate.length) {
      this.candidateQuietMs += chunk.ms;
    } else {
      return { kind: "idle" };
    }
    this.candidate.push(pcm);
    this.candidateChunks.push(chunk);
    if (this.candidateQuietMs >= ONSET_RESET_MS) {
      this.clearCandidate();
      return { kind: "idle" };
    }
    if (this.candidateVoicedMs < MIN_VOICED_MS * guard) return { kind: "idle" };
    const lead = this.candidate;
    this.open = true;
    this.utterance = this.candidateChunks;
    this.utteranceMs = this.candidateChunks.reduce((sum, item) => sum + item.ms, 0);
    this.quietMs = 0;
    this.candidate = [];
    this.candidateChunks = [];
    this.candidateVoicedMs = 0;
    this.candidateQuietMs = 0;
    return { kind: "started", lead };
  }

  private close(): void {
    const evidence = voicedMs(this.utterance);
    // An utterance with no speech in it was the room. Learn its level so the
    // same noise does not immediately open the next one.
    if (evidence.voicedMs < MIN_VOICED_MS) this.floor = Math.max(this.floor, evidence.floor);
    this.open = false;
    this.utterance = [];
    this.utteranceMs = 0;
    this.quietMs = 0;
  }

  private clearCandidate(): void {
    this.candidate = [];
    this.candidateChunks = [];
    this.candidateVoicedMs = 0;
    this.candidateQuietMs = 0;
  }
}
