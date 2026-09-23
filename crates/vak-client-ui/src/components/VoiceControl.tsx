import { createEffect, createSignal, onCleanup } from "solid-js";
import * as api from "../api";
import { startMicrophone, type MicrophoneCapture } from "../voice-capture";
import { SpeechDetector } from "../voice-activity";
import { VoiceSessionSocket, type DiscardReason } from "../voice";
import { setNotice, setPendingSettingsPage, setPendingSettingsSection, setSettingsOpen } from "../store";
import { spokenReplyText } from "../structured";
import AgentMark, { type CharacterState } from "./AgentMark";

const SAMPLE_RATE_HZ = 16_000;
const RECORDING_CHUNK_SAMPLES = 512;

function newUtteranceId(): string {
  return `voice-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

/** Governed voice capture control for the composer. Audio leaves the device
 * only after an explicit press, only while speech is detected, and only over
 * the voice session socket, which alone turns it into an Agent turn. */
export default function VoiceControl(props: { sessionId?: string; character?: string; motion?: "subtle" | "expressive" | "off"; running?: boolean; ensureSession(): Promise<string | null>; onFinal(text: string): void }) {
  const [active, setActive] = createSignal(false);
  const [connecting, setConnecting] = createSignal(false);
  const [status, setStatus] = createSignal<string>("Voice");
  const [paused, setPaused] = createSignal(false);
  const [playing, setPlaying] = createSignal(false);
  const [devices, setDevices] = createSignal<MediaDeviceInfo[]>([]);
  const [deviceId, setDeviceId] = createSignal("");
  const [transcript, setTranscript] = createSignal("");
  const [voiceError, setVoiceError] = createSignal(false);
  let socket: VoiceSessionSocket | undefined;
  let capture: MicrophoneCapture | undefined;
  let detector = new SpeechDetector();
  let voiceSessionId = "";
  /** The utterance currently streaming to the server. */
  let openUtterance = "";
  /** The utterance whose turn owns the input until its answer is ready. */
  let awaitingUtterance = "";
  let lastAnsweredUtterance = "";
  // Stale-async guards: a promise that resolves after stop, interruption or
  // a newer answer must not start audio or change state.
  let voiceGeneration = 0;
  let answerGeneration = 0;
  let playbackContext: AudioContext | undefined;
  let playbackSource: AudioBufferSourceNode | undefined;
  let playbackUtterance = "";
  let playbackStartedAt = 0;
  let playbackDurationMs = 0;

  const idleStatus = () => capture ? "Listening" : "Done";

  function onMicrophoneChunk(pcm: Int16Array) {
    if (!socket) return;
    // One request owns the input until its answer is ready, so room sound
    // during provider work cannot become a competing turn. Input reopens for
    // spoken interruption while the answer plays.
    if (awaitingUtterance && !openUtterance && !playbackSource) return;
    const event = detector.push(pcm, Boolean(playbackSource));
    switch (event.kind) {
      case "idle": return;
      case "started":
        answerGeneration += 1;
        if (playbackSource) stopPlayback(true);
        awaitingUtterance = "";
        openUtterance = newUtteranceId();
        socket.sendControl({ t: "speech_started", utterance_id: openUtterance });
        for (const lead of event.lead) socket.sendAudio(lead);
        setTranscript("");
        setStatus("Listening");
        return;
      case "continuing":
        socket.sendAudio(pcm);
        return;
      case "stopped":
        socket.sendAudio(pcm);
        socket.sendControl({ t: "speech_stopped", utterance_id: openUtterance });
        awaitingUtterance = openUtterance;
        openUtterance = "";
        setStatus("Processing");
    }
  }

  function resetState() {
    capture?.stop(); capture = undefined;
    socket?.close(); socket = undefined;
    playbackSource?.stop(); playbackSource = undefined;
    detector = new SpeechDetector();
    voiceSessionId = "";
    openUtterance = "";
    awaitingUtterance = "";
    playbackUtterance = "";
    playbackStartedAt = 0;
    playbackDurationMs = 0;
    setPlaying(false);
    setPaused(false);
    setActive(false);
    setConnecting(false);
  }

  /** Stopping never submits an unfinished utterance: capture closes without
   * a `speech_stopped` frame, so the server discards the partial audio. */
  function stopVoice() {
    voiceGeneration += 1;
    answerGeneration += 1;
    if (playbackSource) stopPlayback(true);
    resetState();
    setTranscript("");
    setVoiceError(false);
    setStatus("Voice");
  }

  function failVoice(message: string) {
    voiceGeneration += 1;
    answerGeneration += 1;
    resetState();
    setVoiceError(true);
    setStatus("Voice unavailable");
    setNotice({ kind: "error", text: message });
  }

  createEffect(() => {
    if (active() && props.sessionId && props.sessionId !== voiceSessionId) stopVoice();
  });
  onCleanup(() => { stopVoice(); void playbackContext?.close(); });

  function onDiscarded(utteranceId: string, reason: DiscardReason) {
    if (utteranceId !== awaitingUtterance) return;
    awaitingUtterance = "";
    if (reason === "rate_limited") setNotice({ kind: "error", text: "Voice is paused: this minute's voice request budget is spent. Try again shortly or type instead." });
    setStatus(capture ? "Listening" : "No speech heard");
  }

  async function toggle(recording?: File) {
    if (connecting()) return;
    if (active()) { stopVoice(); return; }
    // Unlock output while this call still holds the user's gesture; the
    // answer arrives much later, when browsers may refuse to start audio.
    try {
      playbackContext ??= new AudioContext();
      void playbackContext.resume().catch(() => { /* surfaced when an answer is played */ });
    } catch { /* capture reports its own failure */ }
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
        setPendingSettingsPage("general");
        setPendingSettingsSection("voice");
        setSettingsOpen(true);
        return;
      }
      const sessionId = props.sessionId ?? await props.ensureSession();
      if (!sessionId) throw new Error("Could not open a conversation for voice. Try again.");
      if (generation !== voiceGeneration) return;
      voiceSessionId = sessionId;
      socket = new VoiceSessionSocket({
        sessionId,
        onReady: () => { if (generation === voiceGeneration) setStatus(active() ? "Listening" : "Allow microphone"); },
        onTranscript: (utteranceId, text) => {
          if (generation !== voiceGeneration) return;
          setTranscript(text);
          if (utteranceId === awaitingUtterance) setStatus("Processing");
          props.onFinal(text);
        },
        onDiscarded: (utteranceId, reason) => { if (generation === voiceGeneration) onDiscarded(utteranceId, reason); },
        onTurnCompleted: (utteranceId, reply) => {
          if (generation !== voiceGeneration || utteranceId === lastAnsweredUtterance) return;
          lastAnsweredUtterance = utteranceId;
          // A newer utterance superseded this one; its answer stays as text.
          if (utteranceId !== awaitingUtterance || openUtterance) return;
          void speakAnswer(utteranceId, reply);
        },
        onError: (message) => { if (generation === voiceGeneration) failVoice(message); },
      });
      await socket.connect();
      if (generation !== voiceGeneration || !socket) return;
      if (recording) await sendRecording(recording, config.voice?.max_audio_bytes ?? 16 * 1024 * 1024, generation);
      else {
        setStatus("Allow microphone");
        const started = await startMicrophone((pcm) => {
          if (generation !== voiceGeneration) return;
          try { onMicrophoneChunk(pcm); } catch (error) { failVoice((error as Error).message); }
        });
        if (generation !== voiceGeneration) { started.stop(); return; }
        capture = started;
        setActive(true);
        setConnecting(false);
        setStatus("Listening");
      }
    } catch (error) { if (generation === voiceGeneration) failVoice((error as Error).message); }
  }

  /** A recording is one utterance: decoded and resampled to the socket's
   * PCM, then judged by the same server evidence gate as live speech. */
  async function sendRecording(recording: File, maxAudioBytes: number, generation: number) {
    setStatus("Preparing recording");
    const source = await playbackContext!.decodeAudioData(await recording.arrayBuffer());
    if (generation !== voiceGeneration || !socket) return;
    const sampleCount = Math.ceil(source.duration * SAMPLE_RATE_HZ);
    if (sampleCount * 2 > maxAudioBytes) throw new Error("Recording exceeds the voice audio budget");
    const offline = new OfflineAudioContext(1, sampleCount, SAMPLE_RATE_HZ);
    const input = offline.createBufferSource();
    input.buffer = source;
    input.connect(offline.destination);
    input.start();
    const samples = (await offline.startRendering()).getChannelData(0);
    if (generation !== voiceGeneration || !socket) return;
    setActive(true);
    setConnecting(false);
    setStatus("Processing");
    const utteranceId = newUtteranceId();
    socket.sendControl({ t: "speech_started", utterance_id: utteranceId });
    for (let offset = 0; offset < samples.length; offset += RECORDING_CHUNK_SAMPLES) {
      const frame = samples.subarray(offset, offset + RECORDING_CHUNK_SAMPLES);
      const pcm = new Int16Array(frame.length);
      for (let index = 0; index < frame.length; index += 1) pcm[index] = Math.max(-1, Math.min(1, frame[index])) * 32767;
      socket.sendAudio(pcm);
    }
    socket.sendControl({ t: "speech_stopped", utterance_id: utteranceId });
    awaitingUtterance = utteranceId;
  }

  async function speakAnswer(utteranceId: string, text: string) {
    const answer = spokenReplyText(text);
    if (!answer || answer.startsWith("error:")) {
      awaitingUtterance = "";
      setStatus(idleStatus());
      if (answer) setNotice({ kind: "error", text: answer });
      return;
    }
    const generation = ++answerGeneration;
    try {
      setStatus("Preparing voice");
      const blob = await api.speak(answer, { sessionId: voiceSessionId });
      const bytes = await blob.arrayBuffer();
      if (!active() || generation !== answerGeneration) return;
      if (!bytes.byteLength) throw new Error("Voice provider returned no audio");
      const buffer = await decodeAnswer(bytes, blob.type);
      if (!active() || generation !== answerGeneration) return;
      await playbackContext!.resume();
      if (playbackContext!.state !== "running") throw new Error("Audio output is blocked. Press Voice again to allow playback.");
      if (!active() || generation !== answerGeneration) return;
      startPlayback(utteranceId, buffer);
    } catch (error) {
      if (generation !== answerGeneration) return;
      awaitingUtterance = "";
      setStatus(idleStatus());
      setNotice({ kind: "error", text: `The answer is ready as text, but voice playback failed: ${(error as Error).message}` });
    }
  }

  function decodeAnswer(bytes: ArrayBuffer, mime: string): Promise<AudioBuffer> {
    playbackContext ??= new AudioContext();
    if (mime !== "audio/pcm") return playbackContext.decodeAudioData(bytes);
    const samples = new Int16Array(bytes);
    const buffer = playbackContext.createBuffer(1, samples.length, SAMPLE_RATE_HZ);
    const channel = buffer.getChannelData(0);
    for (let index = 0; index < samples.length; index += 1) channel[index] = samples[index] / 32768;
    return Promise.resolve(buffer);
  }

  function startPlayback(utteranceId: string, buffer: AudioBuffer) {
    const context = playbackContext!;
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.connect(context.destination);
    playbackSource = source;
    playbackUtterance = utteranceId;
    playbackStartedAt = context.currentTime;
    playbackDurationMs = buffer.duration * 1_000;
    source.onended = () => {
      if (playbackSource !== source) return;
      reportPlayback(Math.round(playbackDurationMs), false);
      finishPlayback();
    };
    source.start();
    setPlaying(true);
    setStatus("Speaking");
  }

  function finishPlayback() {
    playbackSource = undefined;
    playbackUtterance = "";
    playbackStartedAt = 0;
    playbackDurationMs = 0;
    awaitingUtterance = "";
    setPlaying(false);
    setPaused(false);
    if (active()) setStatus(idleStatus());
  }

  /** The append-only `voice_playback` receipt records what was actually
   * emitted for this utterance's answer, never what was requested. */
  function reportPlayback(emittedMs: number, interrupted: boolean) {
    if (!playbackUtterance) return;
    try { socket?.sendControl({ t: "playback", utterance_id: playbackUtterance, emitted_ms: emittedMs, interrupted }); } catch { /* transport already closed */ }
  }

  function stopPlayback(interrupted: boolean) {
    const source = playbackSource;
    if (!source) return;
    const elapsedMs = playbackContext && playbackStartedAt ? (playbackContext.currentTime - playbackStartedAt) * 1_000 : 0;
    reportPlayback(Math.round(Math.min(playbackDurationMs, Math.max(0, elapsedMs))), interrupted);
    playbackSource = undefined;
    source.stop();
    if (paused()) void playbackContext?.resume();
    finishPlayback();
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
    if (sink) { try { await sink.call(ctx, id); } catch { setNotice({ kind: "error", text: "That audio output device is unavailable." }); } }
  }
  function togglePause() {
    if (!playbackContext || !playbackSource) return;
    if (paused()) { void playbackContext.resume(); setPaused(false); setStatus("Speaking"); }
    else { void playbackContext.suspend(); setPaused(true); setStatus("Paused"); }
  }

  const visibleStatus = () => active() && props.running && status() === "Listening" ? "Processing" : status();
  const state = () => connecting() ? "connecting" : paused() ? "paused" : status() === "Speaking" ? "speaking" : visibleStatus() === "Processing" || status() === "Preparing voice" ? "processing" : active() ? "listening" : "idle";
  const characterState = (): CharacterState => {
    if (state() === "listening") return "listening";
    if (state() === "processing" || state() === "connecting") return "thinking";
    if (state() === "paused") return "waiting";
    if (state() === "speaking") return "acknowledge";
    return "idle";
  };
  return <span class="voice-control" data-state={state()} role="group" aria-label="Voice conversation controls">
    <button class="composer-context" classList={{ active: active() }} disabled={connecting()} title="Start governed voice conversation" aria-label={active() ? "Stop voice conversation" : "Start voice conversation"} aria-pressed={active()} aria-busy={connecting()} onClick={() => void toggle()}><AgentMark character={props.character} motion={props.motion} size={22} state={characterState()} interactive /><span>{connecting() && status() === "Voice" ? "Connecting…" : visibleStatus()}</span></button>
    {!active() && !connecting() && <label class="composer-context voice-recording-entry">Use recording<input type="file" accept="audio/*" aria-label="Use an audio recording" onChange={(event) => { const file = event.currentTarget.files?.[0]; event.currentTarget.value = ""; if (file) void toggle(file); }} /></label>}
    {transcript() && <span class="voice-transcript" aria-live="polite">{transcript()}</span>}
    {active() && <><button class="composer-context" disabled={!playing()} aria-label={paused() ? "Resume voice playback" : "Pause voice playback"} onClick={togglePause}>{paused() ? "Resume" : "Pause"}</button><button class="composer-context" disabled={!playing()} aria-label="Stop voice playback" onClick={() => stopPlayback(true)}>Stop audio</button><select class="composer-context" aria-label="Voice output device" value={deviceId()} onFocus={() => void refreshDevices()} onChange={(e) => void selectDevice(e.currentTarget.value)}><option value="">Default output</option>{devices().map((d) => <option value={d.deviceId}>{d.label || "Audio output"}</option>)}</select></>}
    {voiceError() && <button class="composer-context voice-fallback" onClick={() => window.dispatchEvent(new CustomEvent("vak:focus-composer"))}>Type instead</button>}
  </span>;
}
