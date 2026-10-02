import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import * as api from "../../api";
import { liveServerOrigin, subjectKey, subjectPath } from "../../canvasSubject";
import { previewSandbox } from "../../safeUrl";
import { createLoader } from "./createLoader";
import DeviceViewport from "./DeviceViewport";
import LoadState from "./LoadState";
import type { ViewerProps } from "./types";

/** A view renews its lease well inside the server's limit (`LAUNCH_LEASE_TTL`). */
const RENEW_MS = 20_000;

/**
 * A dev server the session's launch configuration names. The viewer starts it,
 * or joins it when it is already running (shown in the other app, another tab,
 * or started from the Live preview list), and holds a lease on it while it is
 * shown. Letting go never stops it outright: the server stops a dev server a
 * little after the last view has let go, so moving between tabs,
 * conversations or apps never restarts it.
 */
export default function ServerViewer(props: ViewerProps) {
  const [needsPreparation, setNeedsPreparation] = createSignal(false);
  const [stopped, setStopped] = createSignal(false);
  const target = () => (props.subject.kind === "live_server" ? props.subject : null);

  const loader = createLoader(
    () => subjectKey(props.subject),
    async (_key, current) => {
      const subject = target();
      if (!subject) throw new Error("This is not a live preview.");
      const { sessionId, serverName, candidateId } = subject;
      setNeedsPreparation(false);
      setStopped(false);
      // A loopback port on the server's machine cannot be reached from a browser that is elsewhere.
      if (!liveServerOrigin(api.backendHostname(), 0)) throw new Error("A live preview needs Vakyartha running on this computer.");
      const readiness = await api.getLaunch(sessionId, candidateId);
      const configured = readiness.servers.find((server) => server.name === serverName);
      if (!configured) throw new Error(`There is no live preview called "${serverName}" for this version.`);
      if (!configured.available && !configured.running) {
        setNeedsPreparation(configured.availability === "needs_preparation");
        throw new Error(configured.unavailable_reason ?? "This live preview isn't ready to start.");
      }
      // Each start holds a lease of its own, so letting an earlier one go
      // never lets go of this one.
      const viewer = crypto.randomUUID();
      const started = await api.startLaunch(sessionId, serverName, candidateId, viewer);
      const held = { sessionId, serverName, candidateId, viewer };
      if (!current()) {
        void api.releaseLaunch(sessionId, serverName, candidateId, viewer);
        throw new Error("Superseded.");
      }
      if (!started.port) {
        void api.releaseLaunch(sessionId, serverName, candidateId, viewer);
        throw new Error("This live preview started, but it does not say which port it listens on. Add a port to its entry in .vak/launch.toml.");
      }
      return { port: started.port, ...held };
    },
    (held) => void api.releaseLaunch(held.sessionId, held.serverName, held.candidateId, held.viewer),
  );

  // Renew the lease while shown; a server someone stopped is said to be
  // stopped, never quietly started again.
  createEffect(() => {
    const held = loader.data();
    if (!held) return;
    const timer = setInterval(() => {
      api.leaseLaunch(held.sessionId, held.serverName, held.candidateId, held.viewer).catch((cause) => {
        if (cause instanceof api.ApiError && cause.status === 404) setStopped(true);
      });
    }, RENEW_MS);
    onCleanup(() => clearInterval(timer));
  });

  /** Where the page is framed from: the other loopback name than the app's, so they are two sites. */
  const address = () => {
    const port = loader.data()?.port;
    return port ? liveServerOrigin(api.backendHostname(), port) ?? undefined : undefined;
  };
  createEffect(() => {
    const base = address();
    props.register(base && !stopped() ? { popout: () => window.open(base, "_blank", "noopener,noreferrer") } : null);
  });
  onCleanup(() => props.register(null));

  const prepare = async () => {
    const subject = target();
    if (!subject?.candidateId) return;
    try {
      await api.prepareLaunch(subject.sessionId, subject.serverName, subject.candidateId);
      loader.reload();
    } catch {
      /* The next load reports why it is still not ready. */
    }
  };

  const source = (origin: string) => {
    const subject = target();
    const draftPath = subject?.candidateId && subjectPath(subject)
      ? `/${subjectPath(subject).split("/").map(encodeURIComponent).join("/")}`
      : "";
    return `${origin}${draftPath}`;
  };

  return (
    <LoadState
      loader={loader}
      label="Starting the live preview…"
      extra={needsPreparation() ? <button type="button" class="artifact-canvas-btn" onClick={() => void prepare()}>Install what it needs</button> : undefined}
    >{() => {
      // Shown once it has drawn, so the stage does not flash between colours.
      const [drawn, setDrawn] = createSignal(false);
      const showAnyway = setTimeout(() => setDrawn(true), 1500);
      onCleanup(() => clearTimeout(showAnyway));
      let frame: HTMLIFrameElement | undefined;
      // Reload shows the page again from the server; the server keeps running.
      createEffect((previous: number | undefined) => {
        const key = props.reloadKey;
        if (previous !== undefined && key !== previous && frame && address()) frame.src = source(address()!);
        return key;
      });
      return (
        <Show when={!stopped()} fallback={
          <div class="artifact-canvas-error" role="status">
            <span>This live preview was stopped.</span>
            <button type="button" class="artifact-canvas-btn" onClick={() => loader.reload()}>Start it again</button>
          </div>
        }>
          <DeviceViewport>
            <iframe ref={frame} class="artifact-canvas-frame" classList={{ drawing: !drawn() }} onLoad={() => setDrawn(true)} src={source(address()!)} title={props.subject.title} sandbox={previewSandbox("origin")} />
          </DeviceViewport>
        </Show>
      );
    }}</LoadState>
  );
}
