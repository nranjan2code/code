import { createSignal, onCleanup } from "solid-js";
import { startMicrophone, type MicrophoneCapture } from "../voice-capture";
import { VoiceSessionSocket } from "../voice";

/** Governed voice capture control for the composer. It only streams PCM after
 * an explicit user gesture and reports final text back into the draft. */
export default function VoiceControl(props: { sessionId?: string; onFinal(text: string): void }) {
  const [active, setActive] = createSignal(false);
  const [connecting, setConnecting] = createSignal(false);
  const [status, setStatus] = createSignal<string>("Voice");
  const [paused, setPaused] = createSignal(false);
  const [devices, setDevices] = createSignal<MediaDeviceInfo[]>([]);
  const [deviceId, setDeviceId] = createSignal("");
  let socket: VoiceSessionSocket | undefined;
  let capture: MicrophoneCapture | undefined;
  let playbackContext: AudioContext | undefined;
  let playbackSource: AudioBufferSourceNode | undefined;
  let playbackQueue: AudioBuffer[] = [];
  let recognition: { start(): void; stop(): void; onresult: ((event: any) => void) | null; onerror: ((event: any) => void) | null; onend: (() => void) | null } | undefined;
  let utterance = `voice-${Date.now()}`;
  // In-flight decode promises may resolve after stop/interrupt. Generation
  // guards prevent stale audio from starting a new playback session.
  let playbackGeneration = 0;
  let voiceGeneration = 0;
  function stopVoice() {
    voiceGeneration += 1;
    playbackGeneration += 1;
    capture?.stop(); capture = undefined;
    recognition?.stop(); recognition = undefined;
    if (socket) { try { socket.sendControl({ type: "speech_stopped", utterance_id: utterance }); } catch { /* already closed */ } socket.close(); socket = undefined; }
    playbackSource?.stop(); playbackSource = undefined;
    playbackQueue = [];
    setPaused(false);
    setActive(false); setConnecting(false); setStatus("Voice");
  }
  onCleanup(() => { stopVoice(); void playbackContext?.close(); });
  async function toggle() {
    if (connecting()) return;
    if (active()) { stopVoice(); return; }
    setConnecting(true);
    const generation = ++voiceGeneration;
    try {
      socket = new VoiceSessionSocket({
        sessionId: props.sessionId,
        onReady: () => setStatus("Listening"),
        onTranscript: (event) => { if (generation === voiceGeneration && event.final) props.onFinal(event.text); },
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
            if (playbackSource) { playbackQueue.push(buffer); return; }
            playbackSource = playbackContext!.createBufferSource();
            playbackSource.buffer = buffer;
            playbackSource.connect(playbackContext!.destination);
            playbackSource.onended = () => {
              playbackSource = undefined;
              const next = playbackQueue.shift();
              if (next && active() && playbackContext) {
                const source = playbackContext.createBufferSource();
                source.buffer = next; source.connect(playbackContext.destination); playbackSource = source;
                source.onended = playbackSource.onended;
                source.start(); setStatus("Speaking");
              } else if (active()) setStatus("Listening");
            };
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
      if (generation !== voiceGeneration || !socket) return;
      utterance = `voice-${Date.now()}`;
      socket.sendControl({ type: "speech_started", utterance_id: utterance });
      const startedCapture = await startMicrophone((pcm) => { if (generation !== voiceGeneration) return; try { socket?.sendAudio(pcm); } catch (error) { setStatus((error as Error).message); } });
      if (generation !== voiceGeneration) { startedCapture.stop(); return; }
      capture = startedCapture;
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
  async function refreshDevices() {
    if (!navigator.mediaDevices?.enumerateDevices) return;
    const all = await navigator.mediaDevices.enumerateDevices();
    setDevices(all.filter((d) => d.kind === "audiooutput" && d.deviceId));
  }
  async function selectDevice(id: string) {
    setDeviceId(id);
    const ctx = playbackContext;
    const sink = (ctx as AudioContext & { setSinkId?: (id: string) => Promise<void> })?.setSinkId;
    if (sink) { try { await sink.call(ctx, id); } catch { setStatus("Output device unavailable"); } }
  }
  function togglePause() {
    if (!playbackContext) return;
    if (paused()) { void playbackContext.resume(); setPaused(false); setStatus("Speaking"); }
    else { void playbackContext.suspend(); setPaused(true); setStatus("Paused"); }
  }
  return <span class="voice-control" role="group" aria-label="Voice conversation controls">
    <button class="composer-context" classList={{ active: active() }} disabled={connecting()} title="Start governed voice conversation" aria-label={active() ? "Stop voice conversation" : "Start voice conversation"} aria-pressed={active()} aria-busy={connecting()} onClick={() => void toggle()}><span aria-hidden="true">{active() ? "●" : "◉"}</span><span>{connecting() ? "Connecting…" : status()}</span></button>
    {active() && <><button class="composer-context" disabled={!playbackSource} aria-label={paused() ? "Resume voice playback" : "Pause voice playback"} onClick={togglePause}>{paused() ? "Resume" : "Pause"}</button><button class="composer-context" disabled={!playbackSource} aria-label="Stop voice playback" onClick={() => { playbackGeneration += 1; playbackSource?.stop(); playbackSource = undefined; playbackQueue = []; setPaused(false); setStatus("Listening"); }}>Stop audio</button><select class="composer-context" aria-label="Voice output device" value={deviceId()} onFocus={() => void refreshDevices()} onChange={(e) => void selectDevice(e.currentTarget.value)}><option value="">Default output</option>{devices().map((d) => <option value={d.deviceId}>{d.label || "Audio output"}</option>)}</select></>}
  </span>;
}
