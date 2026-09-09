import { host } from "./host";

export interface MicrophoneCapture {
  readonly sampleRate: number;
  stop(): void;
}

/** Capture mono 16-bit PCM from the browser microphone. The callback is
 * frame-oriented so a WebSocket transport can apply its own packet policy. */
export async function startMicrophone(onFrame: (pcm16: Int16Array) => void): Promise<MicrophoneCapture> {
  if (!host.can("microphone")) throw new Error("This host cannot access a microphone");
  if (typeof navigator === "undefined" || !navigator.mediaDevices?.getUserMedia) {
    throw new Error("Microphone capture is unavailable in this browser");
  }
  let stream: MediaStream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
  } catch (error) {
    const name = error instanceof DOMException ? error.name : "";
    if (name === "NotAllowedError" || name === "SecurityError") throw new Error("Microphone permission was denied. Allow microphone access and try again.");
    if (name === "NotFoundError") throw new Error("No microphone was found. Connect a microphone and try again.");
    throw new Error(`Microphone capture failed: ${error instanceof Error ? error.message : String(error)}`);
  }
  let context: AudioContext;
  try { context = new AudioContext({ sampleRate: 16_000 }); }
  catch (error) { stream.getTracks().forEach((track) => track.stop()); throw new Error(`Microphone audio is unavailable: ${error instanceof Error ? error.message : String(error)}`); }
  const source = context.createMediaStreamSource(stream);
  const processor = context.createScriptProcessor(320, 1, 1);
  processor.onaudioprocess = (event) => {
    const input = event.inputBuffer.getChannelData(0);
    const pcm = new Int16Array(input.length);
    for (let i = 0; i < input.length; i += 1) pcm[i] = Math.max(-1, Math.min(1, input[i])) * 32767;
    onFrame(pcm);
  };
  source.connect(processor);
  processor.connect(context.destination);
  return {
    sampleRate: context.sampleRate,
    stop() {
      processor.disconnect(); source.disconnect(); stream.getTracks().forEach((track) => track.stop());
      void context.close();
    },
  };
}
