import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import * as api from "../api";
import { activeId, openArtifactCanvas, technicalDetails } from "../store";
import { problemWords } from "./canvas/createLoader";

type Server = Awaited<ReturnType<typeof api.getLaunch>>["servers"][number];

/**
 * The dev servers a workspace's `.vak/launch.toml` names: start, stop, read
 * their log, and show one in the Canvas, which is where a page is shown. One
 * started here keeps running until it is stopped here; one the Canvas started
 * stops a little after nothing shows it.
 */
export default function LivePreviewPanel() {
  const [servers, setServers] = createSignal<Server[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [logsFor, setLogsFor] = createSignal<string | null>(null);
  const [logs, setLogs] = createSignal<string[]>([]);
  let asked = 0;

  const refresh = async () => {
    const id = activeId();
    if (!id) return;
    // An answer about the conversation the reader has since left is dropped.
    const mine = ++asked;
    try {
      const reply = await api.getLaunch(id);
      if (mine !== asked || id !== activeId()) return;
      setError(reply.error ? problemWords(new Error(reply.error)) : null);
      setServers(reply.servers ?? []);
    } catch (cause) {
      if (mine === asked) setError(problemWords(cause));
    }
  };
  createEffect(() => {
    void activeId();
    setServers([]);
    setLogsFor(null);
    void refresh();
  });

  const start = async (name: string) => {
    const id = activeId();
    if (!id) return;
    setError(null);
    try {
      await api.startLaunch(id, name);
      await refresh();
    } catch (cause) {
      setError(problemWords(cause));
    }
  };
  const stop = async (name: string) => {
    const id = activeId();
    if (!id) return;
    try {
      await api.stopLaunch(id, name);
      if (logsFor() === name) setLogsFor(null);
      await refresh();
    } catch (cause) {
      setError(problemWords(cause));
    }
  };
  const show = (name: string) => {
    const sessionId = activeId();
    if (sessionId) openArtifactCanvas({ kind: "live_server", title: name, serverName: name, sessionId });
  };

  // Keep the last lines when a poll fails: an unavailable poll is not an empty log.
  createEffect(() => {
    const name = logsFor();
    const id = activeId();
    if (!name || !id) return;
    const poll = async () => {
      try {
        setLogs((await api.launchLogs(id, name)).lines ?? []);
      } catch (cause) {
        setError(`Its log isn't available: ${problemWords(cause)}`);
      }
    };
    void poll();
    const timer = setInterval(() => void poll(), 2000);
    onCleanup(() => clearInterval(timer));
  });

  const state = (server: Server) => {
    if (!server.running) return null;
    const elsewhere = server.viewers ?? 0;
    if (server.pinned) return "Running";
    return elsewhere === 1 ? "Shown in 1 place" : `Shown in ${elsewhere} places`;
  };

  return (
    <div class="prevpane">
      <div class="dock-head">
        <strong>Live preview</strong>
        <button type="button" class="chip sm" onClick={() => void refresh()}>Refresh</button>
      </div>
      <Show when={error()}>{(message) => <div class="dock-empty">{message()}</div>}</Show>
      <div class="prev-servers">
        <For each={servers()} fallback={<div class="hint" style="padding:4px 10px">No live preview is set up for this folder. Add one in .vak/launch.toml.</div>}>
          {(server) => (
            <div class="prev-row">
              <span class="dot" classList={{ run: server.running }} />
              <span class="prev-name">{server.name}</span>
              <Show when={technicalDetails()}>
                <code class="prev-cmd">{[server.cmd, ...server.args].join(" ")}</code>
              </Show>
              <Show when={state(server)}>{(words) => <span class="hint">{words()}</span>}</Show>
              <Show when={!server.available && !server.running}>
                <span class="hint" title={server.unavailable_reason}>
                  {server.availability === "port_in_use" ? "Its port is in use" : server.availability === "needs_preparation" ? "Needs its dependencies installed" : "Needs setting up"}
                </span>
              </Show>
              <button type="button" class="btn primary sm" disabled={!server.available && !server.running} title="Show it in the Canvas" onClick={() => show(server.name)}>Show</button>
              <Show when={server.running} fallback={
                <button type="button" class="btn sm" disabled={!server.available} title={server.available ? "Start it and keep it running" : server.unavailable_reason} onClick={() => void start(server.name)}>Start</button>
              }>
                <button type="button" class="chip sm" classList={{ on: logsFor() === server.name }} onClick={() => setLogsFor(logsFor() === server.name ? null : server.name)}>Log</button>
                <button type="button" class="btn danger sm" onClick={() => void stop(server.name)}>Stop</button>
              </Show>
            </div>
          )}
        </For>
      </div>
      <Show when={logsFor()}>
        <pre class="prev-logs">{logs().join("\n")}</pre>
      </Show>
    </div>
  );
}
