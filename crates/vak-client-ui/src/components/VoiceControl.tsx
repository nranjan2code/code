import { createEffect, createSignal, onCleanup } from "solid-js";
import * as api from "../api";
import { startMicrophone, type MicrophoneCapture } from "../voice-capture";
import { VoiceSessionSocket } from "../voice";
import { setNotice, setSettingsOpen, stripControlScaffolding } from "../store";
import AgentMark, { type CharacterState } from "./AgentMark";

/** Governed voice capture control for the composer. It only streams PCM after
 * an explicit user gesture and keeps interim speech local to the visible UI. */
export default function VoiceControl(props: { sessionId?: string; character?: string; motion?: "subtle" | "expressive" | "off"; running?: boolean; ensureSession(): Promise<string | null>; onFinal(text: string): void }) {
  const [active, setActive] = createSignal(false);
  const [connecting, setConnecting] = createSignal(false);
  const [status, setStatus] = createSignal<string>("Voice");
  const [paused, setPaused] = createSignal(false);
  const [devices, setDevices] = createSignal<MediaDeviceInfo[]>([]);
  const [deviceId, setDeviceId] = createSignal("");
  const [transcript, setTranscript] = createSignal("");
  const [voiceError, setVoiceError] = createSignal(false);
  let socket: VoiceSessionSocket | undefined;
  let capture: MicrophoneCapture | undefined;
  let playbackContext: AudioContext | undefined;
  let playbackSource: AudioBufferSourceNode | undefined;
  let playbackStartedAt = 0;
  let playbackCurrentDurationMs = 0;
  let playbackCompletedMs = 0;
  let recognition: {
    start(): void;
    stop(): void;
    interimResults: boolean;
    continuous: boolean;
    onresult: ((event: any) => void) | null;
    onerror: ((event: any) => void) | null;
    onend: (() => void) | null;
    onspeechstart: (() => void) | null;
  } | undefined;
  let utterance = `voice-${Date.now()}`;
  let playbackUtterance = "";
  // In-flight decode promises may resolve after stop/interrupt. Generation
  // guards prevent stale audio from starting a new playback session.
  let playbackGeneration = 0;
  let voiceGeneration = 0;
  let audioSpeechActive = false;
  let audioSpeechMs = 0;
  let audioSilenceMs = 0;
  let latestSpokenUtterance = "";
  let answerGeneration = 0;
  let voiceSessionId = "";
  let recognitionSpeechActive = false;
  const AUDIO_SAMPLE_RATE = 16_000;
  const SPEECH_THRESHOLD = 0.018;
  const MIN_SPEECH_MS = 240;
  const END_SILENCE_MS = 650;
  function streamDetectedSpeech(pcm: Int16Array) {
    if (!socket || !pcm.length) return;
    let energy = 0;
    for (const sample of pcm) {
      const normalized = sample / 32768;
      energy += normalized * normalized;
    }
    const frameMs = (pcm.length / AUDIO_SAMPLE_RATE) * 1_000;
    const hasVoice = Math.sqrt(energy / pcm.length) >= SPEECH_THRESHOLD;
    if (hasVoice && !audioSpeechActive) {
      answerGeneration += 1;
      if (playbackSource) stopPlayback(true);
      else playbackGeneration += 1;
      utterance = `voice-${Date.now()}-${Math.random().toString(36).slice(2)}`;
      socket.sendControl({ type: "speech_started", utterance_id: utterance });
      audioSpeechActive = true;
      audioSpeechMs = 0;
      audioSilenceMs = 0;
      setStatus("Listening");
    }
    if (!audioSpeechActive) return;
    socket.sendAudio(pcm);
    if (hasVoice) {
      audioSpeechMs += frameMs;
      audioSilenceMs = 0;
      return;
    }
    audioSilenceMs += frameMs;
    if (audioSpeechMs >= MIN_SPEECH_MS && audioSilenceMs >= END_SILENCE_MS) {
      socket.sendControl({ type: "speech_stopped", utterance_id: utterance });
      audioSpeechActive = false;
      audioSpeechMs = 0;
      audioSilenceMs = 0;
      setStatus("Processing");
    }
  }
  function stopVoice() {
    voiceGeneration += 1;
    playbackGeneration += 1;
    setActive(false);
    capture?.stop(); capture = undefined;
    recognition?.stop(); recognition = undefined;
    if (socket) { try { socket.sendControl({ type: "speech_stopped", utterance_id: utterance }); } catch { /* already closed */ } socket.close(); socket = undefined; }
    playbackSource?.stop(); playbackSource = undefined;
    playbackUtterance = "";
    playbackStartedAt = 0;
    playbackCurrentDurationMs = 0;
    playbackCompletedMs = 0;
    audioSpeechActive = false;
    audioSpeechMs = 0;
    audioSilenceMs = 0;
    answerGeneration += 1;
    voiceSessionId = "";
    recognitionSpeechActive = false;
    setTranscript("");
    setVoiceError(false);
    setPaused(false);
    setConnecting(false); setStatus("Voice");
  }
  createEffect(() => {
    if (active() && props.sessionId && props.sessionId !== voiceSessionId) stopVoice();
  });
  onCleanup(() => { stopVoice(); void playbackContext?.close(); });
  async function toggle() {
    if (connecting()) return;
    if (active()) { stopVoice(); return; }
    setConnecting(true);
    setVoiceError(false);
    setTranscript("");
    const generation = ++voiceGeneration;
    try {
      const config = await api.getConfig();
      if (generation !== voiceGeneration) return;
      if (!config.voice?.enabled) {
        setConnecting(false);
        setStatus("Set up voice");
        setSettingsOpen(true);
        return;
      }
      const sessionId = props.sessionId ?? await props.ensureSession();
      if (!sessionId) throw new Error("Could not open a conversation for voice. Try again.");
      if (generation !== voiceGeneration) return;
      voiceSessionId = sessionId;
      socket = new VoiceSessionSocket({
        sessionId,
        onReady: () => setStatus(active() ? "Listening" : "Allow microphone"),
        onTranscript: (event) => {
          if (generation !== voiceGeneration) return;
          setTranscript(event.text);
          if (event.final) {
            setStatus("Processing");
            props.onFinal(event.text);
          }
        },
        onTurnCompleted: (utteranceId, reply) => {
          if (generation !== voiceGeneration || !utteranceId || utteranceId === latestSpokenUtterance) return;
          latestSpokenUtterance = utteranceId;
          if (audioSpeechActive || recognitionSpeechActive) return;
          void speakAnswer(reply);
        },
        onError: (message) => { failVoice(message); },
      });
      await socket.connect();
      if (generation !== voiceGeneration || !socket) return;
      const Recognition = (globalThis as any).SpeechRecognition ?? (globalThis as any).webkitSpeechRecognition;
      setStatus("Allow microphone");
      const startedCapture = await startMicrophone((pcm) => {
        if (generation !== voiceGeneration || Recognition) return;
        try { streamDetectedSpeech(pcm); } catch (error) { failVoice((error as Error).message); }
      });
      if (generation !== voiceGeneration) { startedCapture.stop(); return; }
      capture = startedCapture;
      setActive(true);
      setConnecting(false);
      if (Recognition) {
        utterance = `voice-${Date.now()}`;
        socket.sendControl({ type: "speech_started", utterance_id: utterance });
        recognition = new Recognition();
        recognition!.interimResults = true;
        recognition!.continuous = true;
        recognition!.onresult = (event) => {
          const result = event.results[event.results.length - 1];
          const text = String(result?.[0]?.transcript ?? "").trim();
          if (text) {
            setTranscript(text);
            if (result.isFinal) {
              recognitionSpeechActive = false;
              socket?.sendControl({ type: "transcript", utterance_id: utterance, text, final: true });
              utterance = `voice-${Date.now()}-${Math.random().toString(36).slice(2)}`;
              socket?.sendControl({ type: "speech_started", utterance_id: utterance });
            }
          }
        };
        recognition!.onspeechstart = () => {
          recognitionSpeechActive = true;
          answerGeneration += 1;
          if (playbackSource) stopPlayback(true);
          else playbackGeneration += 1;
          setStatus("Listening");
        };
        recognition!.onerror = () => failVoice("Live transcription became unavailable. You can keep working by typing.");
        recognition!.onend = () => { if (active()) { try { recognition?.start(); } catch { /* browser may reject restart */ } } };
        recognition!.start();
      } else setStatus("Listening");
    } catch (error) { failVoice((error as Error).message); }
  }
  async function speakAnswer(text: string) {
    const answer = stripControlScaffolding(text).trim();
    if (!answer || !active()) return;
    if (answer.startsWith("error:")) {
      setStatus("Listening");
      setNotice({ kind: "error", text: answer });
      return;
    }
    const generation = ++answerGeneration;
    try {
      setStatus("Preparing voice");
      const blob = await api.speak(answer, { sessionId: voiceSessionId });
      if (!active() || generation !== answerGeneration) return;
      const bytes = await blob.arrayBuffer();
      if (!active() || generation !== answerGeneration) return;
      if (!bytes.byteLength) throw new Error("Voice provider returned no audio");
      onAnswerAudio(bytes, blob.type);
    } catch (error) {
      if (generation !== answerGeneration) return;
      setStatus("Listening");
      setNotice({ kind: "error", text: `The answer is ready as text, but voice playback failed: ${(error as Error).message}` });
    }
  }
  function onAnswerAudio(bytes: ArrayBuffer, mime: string) {
    playbackUtterance = `answer-${Date.now()}`;
    playbackCompletedMs = 0;
    const generation = playbackGeneration;
    playbackContext ??= new AudioContext();
    const isRawPcm = mime === "audio/pcm" || mime === "audio/L16";
    const decode = isRawPcm
      ? Promise.resolve((() => {
          const samples = new Int16Array(bytes);
          const buffer = playbackContext!.createBuffer(1, samples.length, AUDIO_SAMPLE_RATE);
          const channel = buffer.getChannelData(0);
          for (let index = 0; index < samples.length; index += 1) channel[index] = samples[index] / 32768;
          return buffer;
        })())
      : playbackContext.decodeAudioData(bytes.slice(0));
    void decode.then((buffer) => {
      if (generation !== playbackGeneration || !active()) return;
      startPlaybackBuffer(buffer, generation);
    }).catch(() => {
      if (generation === playbackGeneration) setNotice({ kind: "error", text: "The answer is ready as text, but this audio format cannot be played here." });
      if (active()) setStatus("Listening");
    });
  }
  function failVoice(message: string) {
    voiceGeneration += 1;
    answerGeneration += 1;
    setActive(false);
    voiceSessionId = "";
    recognitionSpeechActive = false;
    socket?.close(); socket = undefined;
    capture?.stop(); capture = undefined;
    recognition?.stop(); recognition = undefined;
    playbackGeneration += 1;
    playbackSource?.stop(); playbackSource = undefined;
    setConnecting(false); setVoiceError(true); setStatus("Voice unavailable");
    setNotice({ kind: "error", text: message });
  }
  function stopPlayback(reportInterruption: boolean) {
    const currentElapsedMs = playbackContext && playbackStartedAt
      ? Math.min(playbackCurrentDurationMs, Math.max(0, (playbackContext.currentTime - playbackStartedAt) * 1000))
      : 0;
    const emittedMs = Math.round(playbackCompletedMs + currentElapsedMs);
    playbackGeneration += 1;
    playbackSource?.stop(); playbackSource = undefined;
    setPaused(false);
    if (reportInterruption && playbackUtterance) {
      try { socket?.sendControl({ type: "playback", utterance_id: playbackUtterance, emitted_ms: emittedMs, interrupted: true }); } catch { /* closed transport */ }
    }
    playbackUtterance = "";
    playbackStartedAt = 0;
    playbackCurrentDurationMs = 0;
    playbackCompletedMs = 0;
    if (active()) setStatus("Listening");
  }
  function startPlaybackBuffer(buffer: AudioBuffer, generation: number) {
    if (!playbackContext || generation !== playbackGeneration || !active()) return;
    const source = playbackContext.createBufferSource();
    source.buffer = buffer;
    source.connect(playbackContext.destination);
    playbackSource = source;
    playbackStartedAt = playbackContext.currentTime;
    playbackCurrentDurationMs = buffer.duration * 1000;
    source.onended = () => {
      if (generation !== playbackGeneration || playbackSource !== source) return;
      playbackCompletedMs += playbackCurrentDurationMs;
      playbackStartedAt = 0;
      playbackCurrentDurationMs = 0;
      playbackSource = undefined;
      if (active()) setStatus("Listening");
    };
    source.start();
    setStatus("Speaking");
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
  const visibleStatus = () => active() && props.running && status() === "Listening" ? "Processing" : status();
  const state = () => connecting() ? "connecting" : paused() ? "paused" : status().toLowerCase().startsWith("speaking") ? "speaking" : visibleStatus() === "Processing" ? "processing" : active() ? "listening" : "idle";
  const characterState = (): CharacterState => {
    if (state() === "listening") return "listening";
    if (state() === "processing" || state() === "connecting") return "thinking";
    if (state() === "paused") return "waiting";
    if (state() === "speaking") return "acknowledge";
    return "idle";
  };
  return <span class="voice-control" data-state={state()} role="group" aria-label="Voice conversation controls">
    <button class="composer-context" classList={{ active: active() }} disabled={connecting()} title="Start governed voice conversation" aria-label={active() ? "Stop voice conversation" : "Start voice conversation"} aria-pressed={active()} aria-busy={connecting()} onClick={() => void toggle()}><AgentMark character={props.character} motion={props.motion} size={22} state={characterState()} interactive /><span>{connecting() ? "Connecting…" : visibleStatus()}</span></button>
    {transcript() && <span class="voice-transcript" aria-live="polite">{transcript()}</span>}
    {active() && <><button class="composer-context" disabled={!playbackSource} aria-label={paused() ? "Resume voice playback" : "Pause voice playback"} onClick={togglePause}>{paused() ? "Resume" : "Pause"}</button><button class="composer-context" disabled={!playbackSource} aria-label="Stop voice playback" onClick={() => stopPlayback(true)}>Stop audio</button><select class="composer-context" aria-label="Voice output device" value={deviceId()} onFocus={() => void refreshDevices()} onChange={(e) => void selectDevice(e.currentTarget.value)}><option value="">Default output</option>{devices().map((d) => <option value={d.deviceId}>{d.label || "Audio output"}</option>)}</select></>}
    {voiceError() && <button class="composer-context voice-fallback" onClick={() => window.dispatchEvent(new CustomEvent("vak:focus-composer"))}>Type instead</button>}
  </span>;
}
