/** Client-side voice conversation state. Audio transport is deliberately
 * injected: browser and Tauri hosts can use different WebSocket/worklet
 * implementations while sharing transcript and interruption semantics. */

export type VoicePhase = "idle" | "listening" | "thinking" | "speaking" | "error";

export interface TranscriptRevision {
  utteranceId: string;
  text: string;
  final: boolean;
}

export interface VoiceConversationCallbacks {
  onTranscript(revision: TranscriptRevision): void;
  onCommit(text: string, utteranceId: string): void;
  onSteer(text: string, utteranceId: string): void;
  onCancelRun(): void;
  onPlaybackStop(): void;
  onPhase(phase: VoicePhase): void;
  onError(message: string): void;
}

/**
 * Coordinates voice UX without assuming that a barge-in cancels work.
 * Speech onset always stops playback; only an explicit cancellation action
 * reaches the running agent. Final transcripts are de-duplicated by the
 * utterance id and can be committed or steered independently.
 */
export class VoiceConversationController {
  private phase: VoicePhase = "idle";
  private revisions = new Map<string, TranscriptRevision>();
  private activeUtterance: string | null = null;

  constructor(private readonly callbacks: VoiceConversationCallbacks) {}

  getPhase(): VoicePhase { return this.phase; }

  beginListening(utteranceId: string): void {
    this.activeUtterance = utteranceId;
    this.setPhase("listening");
  }

  receiveTranscript(revision: TranscriptRevision): void {
    if (!revision.utteranceId || !revision.text.trim()) return;
    const prior = this.revisions.get(revision.utteranceId);
    if (prior?.final) return;
    if (prior?.text === revision.text && prior.final === revision.final) return;
    this.revisions.set(revision.utteranceId, revision);
    this.callbacks.onTranscript(revision);
    if (revision.final && this.activeUtterance === revision.utteranceId) {
      this.setPhase("thinking");
    }
  }

  commit(utteranceId: string): void {
    const revision = this.revisions.get(utteranceId);
    if (!revision?.final || !revision.text.trim()) return;
    this.callbacks.onCommit(revision.text, utteranceId);
  }

  steer(utteranceId: string): void {
    const revision = this.revisions.get(utteranceId);
    if (!revision?.final || !revision.text.trim()) return;
    this.callbacks.onSteer(revision.text, utteranceId);
  }

  beginSpeaking(): void { this.setPhase("speaking"); }

  /** Stops audio immediately while leaving the agent run untouched. */
  interruptPlayback(): void {
    if (this.phase === "speaking") {
      this.callbacks.onPlaybackStop();
      this.setPhase("listening");
    }
  }

  cancelRun(): void {
    this.callbacks.onCancelRun();
    this.setPhase("idle");
  }

  fail(message: string): void {
    this.setPhase("error");
    this.callbacks.onError(message);
  }

  reset(): void {
    this.activeUtterance = null;
    this.setPhase("idle");
  }

  private setPhase(phase: VoicePhase): void {
    this.phase = phase;
    this.callbacks.onPhase(phase);
  }
}

export interface VoiceSocketCallbacks {
  onReady(sampleRateHz: number, channels: number): void;
  onTranscript(revision: TranscriptRevision): void;
  onPlayback(frame: ArrayBuffer, utteranceId: string, interrupted: boolean): void;
  onError(message: string): void;
  /** Provider attribution and settlement for the synthesized turn. */
  onReceipt?(utteranceId: string, receipt: Record<string, unknown>): void;
}

/** Canonical browser/Tauri transport for a governed voice session. */
export class VoiceSessionSocket {
  private socket: WebSocket | null = null;
  private sentBytes = 0;
  private closing = false;

  constructor(private readonly callbacks: VoiceSocketCallbacks & { sessionId?: string }) {}

  connect(url?: string): Promise<void> {
    url ??= `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/voice/session${this.callbacks.sessionId ? `?session_id=${encodeURIComponent(this.callbacks.sessionId)}` : ""}`;
    return new Promise((resolve, reject) => {
      const socket = new WebSocket(url);
      socket.binaryType = "arraybuffer";
      this.socket = socket;
      let opened = false;
      socket.onopen = () => { opened = true; resolve(); };
      socket.onerror = () => {
        if (this.socket === socket && socket.readyState !== WebSocket.OPEN) {
          reject(new Error("voice session transport failed"));
        } else if (this.socket === socket) {
          this.callbacks.onError("voice session transport error");
        }
      };
      socket.onclose = (event) => {
        if (this.socket !== socket || this.closing) return;
        this.socket = null;
        this.sentBytes = 0;
        if (!opened) {
          reject(new Error(`voice session disconnected (${event.reason || event.code})`));
          return;
        }
        if (event.code !== 1000) this.callbacks.onError(`voice session disconnected (${event.reason || event.code})`);
      };
      socket.onmessage = (event) => {
        if (event.data instanceof ArrayBuffer) {
          this.callbacks.onPlayback(event.data, "", false);
          return;
        }
        try {
          const control = JSON.parse(String(event.data)) as Record<string, unknown>;
          if (control.type === "error") { this.callbacks.onError(String(control.message ?? "voice session error")); return; }
          if (control.t === "ready") {
            const version = control.protocol_version;
            if (version !== undefined && version !== 1) {
              this.callbacks.onError(`unsupported voice protocol version: ${String(version)}`);
              this.close();
              return;
            }
            this.callbacks.onReady(Number(control.sample_rate_hz), Number(control.channels)); return;
          }
          if (control.t === "transcript") {
            this.callbacks.onTranscript({ utteranceId: String(control.utterance_id), text: String(control.text), final: Boolean(control.final) });
            return;
          }
          if (control.t === "playback") this.callbacks.onPlayback(new ArrayBuffer(0), String(control.utterance_id ?? ""), Boolean(control.interrupted));
          if (control.t === "receipt" && this.callbacks.onReceipt) this.callbacks.onReceipt(String(control.utterance_id ?? ""), (control.receipt ?? {}) as Record<string, unknown>);
        } catch { this.callbacks.onError("invalid voice server frame"); }
      };
    });
  }

  sendAudio(pcm: ArrayBuffer | Int16Array): void {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) throw new Error("voice session is not connected");
    const bytes = pcm instanceof Int16Array ? pcm.buffer : pcm;
    if (bytes.byteLength === 0 || bytes.byteLength > 64 * 1024 || bytes.byteLength % 2 !== 0) throw new Error("invalid PCM audio frame");
    this.sentBytes += bytes.byteLength;
    this.socket.send(bytes);
  }

  sendControl(control: Record<string, unknown>): void {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) throw new Error("voice session is not connected");
    this.socket.send(JSON.stringify(control));
  }

  close(): void {
    this.closing = true;
    this.socket?.close(1000, "voice session ended");
    this.socket = null;
    this.sentBytes = 0;
    queueMicrotask(() => { this.closing = false; });
  }
}
