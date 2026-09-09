import { createSignal, onCleanup } from "solid-js";
import { startMicrophone, type MicrophoneCapture } from "../voice-capture";
import { VoiceSessionSocket } from "../voice";

/** Governed voice capture control for the composer. It only streams PCM after
 * an explicit user gesture and reports final text back into the draft. */
export default function VoiceControl(props: { sessionId?: string; onFinal(text: string): void }) {
  const [active, setActive] = createSignal(false);
  const [connecting, setConnecting] = createSignal(false);
  const [status, setStatus] = createSignal<string>("Voice");
  let socket: VoiceSessionSocket | undefined;
  let capture: MicrophoneCapture | undefined;
  let playbackContext: AudioContext | undefined;
  let playbackSource: AudioBufferSourceNode | undefined;
  let recognition: { start(): void; stop(): void; onresult: ((event: any) => void) | null; onerror: ((event: any) => void) | null; onend: (() => void) | null } | undefined;
  let utterance = `voice-${Date.now()}`;
  // In-flight decode promises may resolve after stop/interrupt. Generation
  // guards prevent stale audio from starting a new playback session.
  let playbackGeneration = 0;
  function stopVoice() {
    playbackGeneration += 1;
    capture?.stop(); capture = undefined;
    recognition?.stop(); recognition = undefined;
    if (socket) { try { socket.sendControl({ type: "speech_stopped", utterance_id: utterance }); } catch { /* already closed */ } socket.close(); socket = undefined; }
    playbackSource?.stop(); playbackSource = undefined;
    setActive(false); setConnecting(false); setStatus("Voice");
  }
  onCleanup(() => { stopVoice(); void playbackContext?.close(); });
  async function toggle() {
    if (connecting()) return;
    if (active()) { stopVoice(); return; }
    setConnecting(true);
    try {
      socket = new VoiceSessionSocket({
        sessionId: props.sessionId,
        onReady: () => setStatus("Listening"),
        onTranscript: (event) => { if (event.final) props.onFinal(event.text); },
        onPlayback: (bytes, _utterance, interrupted) => {
          if (interrupted) { playbackGeneration += 1; playbackSource?.stop(); playbackSource = undefined; setStatus("Listening"); return; }
          if (!bytes.byteLength) return;
          const generation = playbackGeneration;
          playbackContext ??= new AudioContext();
          const decode = bytes.byteLength >= 4 && new Uint8Array(bytes.slice(0, 4)).every((v, i) => v === [82, 73, 70, 70][i])
            ? playbackContext.decodeAudioData(bytes.slice(0))
            : Promise.resolve((() => {
                const samples = new Int16Array(bytes.slice(0));
                const buffer = playbackContext!.createBuffer(1, samples.length, 16_000);
                const channel = buffer.getChannelData(0);
                for (let i = 0; i < samples.length; i += 1) channel[i] = samples[i] / 32768;
                return buffer;
              })());
          void decode.then((buffer) => {
            if (generation !== playbackGeneration || !active() || !playbackContext) return;
            playbackSource?.stop();
            playbackSource = playbackContext!.createBufferSource();
            playbackSource.buffer = buffer;
            playbackSource.connect(playbackContext!.destination);
            playbackSource.onended = () => { playbackSource = undefined; if (active()) setStatus("Listening"); };
            playbackSource.start(); setStatus("Speaking");
          }).catch(() => { if (generation === playbackGeneration) setStatus("Unsupported voice audio"); });
        },
        onReceipt: (_utterance, receipt) => {
          const provider = typeof receipt.provider === "string" ? receipt.provider : "voice";
          setStatus(`Speaking · ${provider}`);
        },
        onError: (message) => { setStatus(message); setActive(false); setConnecting(false); capture?.stop(); capture = undefined; recognition?.stop(); recognition = undefined; socket?.close(); socket = undefined; },
      });
      await socket.connect();
      utterance = `voice-${Date.now()}`;
      socket.sendControl({ type: "speech_started", utterance_id: utterance });
      capture = await startMicrophone((pcm) => { try { socket?.sendAudio(pcm); } catch (error) { setStatus((error as Error).message); } });
      setActive(true);
      setConnecting(false);
      const Recognition = (globalThis as any).SpeechRecognition ?? (globalThis as any).webkitSpeechRecognition;
      if (Recognition) {
        recognition = new Recognition();
        recognition!.onresult = (event) => {
          const result = event.results[event.results.length - 1];
          if (result?.isFinal) {
            const text = String(result[0]?.transcript ?? "").trim();
            if (text) socket?.sendControl({ type: "transcript", utterance_id: utterance, text, final: true });
          }
        };
        recognition!.onerror = () => setStatus("Local transcription unavailable");
        recognition!.onend = () => { if (active()) { try { recognition?.start(); } catch { /* browser may reject restart */ } } };
        recognition!.start();
      } else setStatus("Listening (audio only)");
    } catch (error) { socket?.close(); socket = undefined; capture?.stop(); capture = undefined; recognition?.stop(); recognition = undefined; setConnecting(false); setStatus((error as Error).message); }
  }
  return <button class="composer-context" classList={{ active: active() }} disabled={connecting()} title="Start governed voice conversation" aria-label={active() ? "Stop voice conversation" : "Start voice conversation"} aria-pressed={active()} aria-busy={connecting()} onClick={() => void toggle()}><span aria-hidden="true">{active() ? "●" : "◉"}</span><span>{connecting() ? "Connecting…" : status()}</span></button>;
}
