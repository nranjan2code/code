import { backendUrl, backendToken } from "./api";

/** `WS /voice/session` client, the one transport for spoken turns in the
 * browser and the desktop shell. The wire contract is
 * `crates/vak-voice/src/protocol.rs`: the client sends PCM and utterance
 * boundaries; only the server produces transcripts. */

export const VOICE_PROTOCOL_VERSION = 1;

export type DiscardReason = "insufficient_speech" | "empty_transcript" | "rate_limited";

export type VoiceClientControl =
  | { t: "speech_started" | "speech_stopped"; utterance_id: string }
  | { t: "playback"; utterance_id: string; emitted_ms: number; interrupted: boolean };

export interface VoiceSocketCallbacks {
  sessionId: string;
  onReady(): void;
  /** Final transcript; the server has started the Agent turn for it. */
  onTranscript(utteranceId: string, text: string): void;
  /** The utterance closed without starting a turn. */
  onDiscarded(utteranceId: string, reason: DiscardReason): void;
  onTurnCompleted(utteranceId: string, text: string): void;
  onError(message: string): void;
}

export class VoiceSessionSocket {
  private socket: WebSocket | null = null;
  private closing = false;

  constructor(private readonly callbacks: VoiceSocketCallbacks) {}

  connect(): Promise<void> {
    const base = backendUrl();
    const wsBase = base ? base.replace(/^http:/, "ws:").replace(/^https:/, "wss:") : `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}`;
    const query = new URLSearchParams({ session_id: this.callbacks.sessionId });
    const token = backendToken();
    if (token) query.set("token", token);
    return new Promise((resolve, reject) => {
      const socket = new WebSocket(`${wsBase}/voice/session?${query}`);
      socket.binaryType = "arraybuffer";
      this.socket = socket;
      let opened = false;
      socket.onopen = () => { opened = true; resolve(); };
      socket.onerror = () => {
        if (this.socket !== socket) return;
        if (socket.readyState !== WebSocket.OPEN) reject(new Error("voice session transport failed"));
        else this.callbacks.onError("voice session transport error");
      };
      socket.onclose = (event) => {
        if (this.socket !== socket || this.closing) return;
        this.socket = null;
        if (!opened) { reject(new Error(`voice session disconnected (${event.reason || event.code})`)); return; }
        if (event.code !== 1000) this.callbacks.onError(`voice session disconnected (${event.reason || event.code})`);
      };
      socket.onmessage = (event) => {
        if (typeof event.data !== "string") { this.callbacks.onError("unexpected audio frame from voice session"); return; }
        let control: Record<string, unknown>;
        try { control = JSON.parse(event.data) as Record<string, unknown>; } catch { this.callbacks.onError("invalid voice server frame"); return; }
        const utteranceId = String(control.utterance_id ?? "");
        switch (control.t) {
          case "ready":
            if (control.protocol_version !== VOICE_PROTOCOL_VERSION) {
              this.callbacks.onError(`unsupported voice protocol version: ${String(control.protocol_version)}`);
              this.close();
              return;
            }
            this.callbacks.onReady();
            return;
          case "transcript": this.callbacks.onTranscript(utteranceId, String(control.text ?? "")); return;
          case "discarded": this.callbacks.onDiscarded(utteranceId, control.reason as DiscardReason); return;
          case "turn_completed": this.callbacks.onTurnCompleted(utteranceId, String(control.text ?? "")); return;
          case "error": {
            const message = String(control.message ?? "voice session error");
            this.callbacks.onError(typeof control.remedy === "string" && control.remedy ? `${message} — ${control.remedy}` : message);
            return;
          }
          default: this.callbacks.onError(`unknown voice server frame: ${String(control.t)}`);
        }
      };
    });
  }

  sendAudio(pcm: Int16Array): void {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) throw new Error("voice session is not connected");
    if (pcm.byteLength === 0 || pcm.byteLength > 64 * 1024) throw new Error("invalid PCM audio frame");
    this.socket.send(pcm);
  }

  sendControl(control: VoiceClientControl): void {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) throw new Error("voice session is not connected");
    this.socket.send(JSON.stringify(control));
  }

  close(): void {
    this.closing = true;
    this.socket?.close(1000, "voice session ended");
    this.socket = null;
    queueMicrotask(() => { this.closing = false; });
  }
}
