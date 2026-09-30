import { createEffect, createSignal, onCleanup } from "solid-js";
import * as api from "../../api";
import { liveServerOrigin, subjectPath } from "../../canvasSubject";
import { previewSandbox } from "../../safeUrl";
import { createLoader } from "./createLoader";
import DeviceViewport from "./DeviceViewport";
import LoadState from "./LoadState";
import type { ViewerProps } from "./types";

/**
 * A dev server the session's launch configuration names. The viewer starts it
 * and, when it lets go, stops the one it started — under the identity it
 * started it with, and never one that was already running.
 */
export default function ServerViewer(props: ViewerProps) {
  const [needsPreparation, setNeedsPreparation] = createSignal(false);
  const loader = createLoader(
    () => props.subject,
    async (subject, current) => {
      if (subject.kind !== "live_server") throw new Error("This is not a live preview.");
      const { sessionId, serverName, candidateId } = subject;
      setNeedsPreparation(false);
      // A loopback port on the server's machine cannot be reached from a browser that is elsewhere.
      if (!liveServerOrigin(api.backendHostname(), 0)) throw new Error("A live preview needs Vakyartha running on this computer.");
      const readiness = await api.getLaunch(sessionId, candidateId);
      const configured = readiness.servers.find((server) => server.name === serverName);
      if (!configured) throw new Error(`Dev server "${serverName}" is unavailable for this saved version.`);
      if (!configured.available && !configured.running) {
        setNeedsPreparation(configured.availability === "needs_preparation");
        throw new Error(configured.unavailable_reason ?? "Preview environment is not ready.");
      }
      const started = await api.startLaunch(sessionId, serverName, candidateId);
      if (started.error && !started.error.toLowerCase().includes("already running")) throw new Error(started.error);
      const owned = !started.error;
      const stop = () => { if (owned) void api.stopLaunch(sessionId, serverName, candidateId); };
      if (!current()) {
        stop();
        throw new Error("Superseded.");
      }
      const running = (await api.getLaunch(sessionId, candidateId)).servers.find((server) => server.name === serverName);
      if (!running?.port) {
        stop();
        throw new Error(`Dev server "${serverName}" started, but did not report a listening port.`);
      }
      return { port: running.port, stop };
    },
    (server) => server.stop(),
  );

  /** Where the page is framed from: the other loopback name than the app's, so they are two sites. */
  const address = () => {
    const port = loader.data()?.port;
    return port ? liveServerOrigin(api.backendHostname(), port) ?? undefined : undefined;
  };
  createEffect(() => {
    const base = address();
    props.register(base ? { popout: () => window.open(base, "_blank", "noopener,noreferrer") } : null);
  });
  onCleanup(() => props.register(null));

  const prepare = async () => {
    const subject = props.subject;
    if (subject.kind !== "live_server" || !subject.candidateId) return;
    try {
      await api.prepareLaunch(subject.sessionId, subject.serverName, subject.candidateId);
      loader.reload();
    } catch {
      /* The next load reports why it is still not ready. */
    }
  };

  const source = (origin: string) => {
    const subject = props.subject;
    const draftPath = subject.kind === "live_server" && subject.candidateId && subjectPath(subject)
      ? `/${subjectPath(subject).split("/").map(encodeURIComponent).join("/")}`
      : "";
    return `${origin}${draftPath}?_k=${props.reloadKey}`;
  };

  return (
    <LoadState
      loader={loader}
      extra={needsPreparation() ? <button type="button" class="artifact-canvas-btn" onClick={() => void prepare()}>Prepare dependencies</button> : undefined}
    >{() =>
      <DeviceViewport>
        <iframe class="artifact-canvas-frame" src={source(address()!)} title={props.subject.title} sandbox={previewSandbox("origin")} />
      </DeviceViewport>
    }</LoadState>
  );
}
