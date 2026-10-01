import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import * as api from "../api";
import { activeId, openArtifactCanvas } from "../store";

type Server = Awaited<ReturnType<typeof api.getLaunch>>["servers"][number];

/**
 * The dev servers a workspace's `.vak/launch.toml` names: start, stop, read
 * their log, and open one in the Canvas, which is where a page is shown.
 */
export default function LivePreviewPanel() {
  const [servers, setServers] = createSignal<Server[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [logsFor, setLogsFor] = createSignal<string | null>(null);
  const [logs, setLogs] = createSignal<string[]>([]);

  const refresh = async () => {
    const id = activeId();
    if (!id) return;
    try {
      const reply = await api.getLaunch(id);
      setError((reply as { error?: string }).error ?? null);
      setServers(reply.servers ?? []);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };
  createEffect(() => {
    void activeId();
    void refresh();
  });

  const start = async (name: string) => {
    const id = activeId();
    if (!id) return;
    setError(null);
    try {
      const reply = await api.startLaunch(id, name);
      if (reply.error) setError(reply.error);
      await refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
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
      setError(cause instanceof Error ? cause.message : String(cause));
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
        setError(null);
      } catch (cause) {
        setError(`Logs unavailable: ${cause instanceof Error ? cause.message : String(cause)}`);
      }
    };
    void poll();
    const timer = setInterval(() => void poll(), 2000);
    onCleanup(() => clearInterval(timer));
  });

  return (
    <div class="prevpane">
      <div class="dock-head">
        <strong>Live preview</strong>
        <button type="button" class="chip sm" onClick={() => void refresh()}>refresh</button>
      </div>
      <Show when={error()}>{(message) => <div class="dock-empty">{message()}</div>}</Show>
      <div class="prev-servers">
        <For each={servers()} fallback={<div class="hint" style="padding:4px 10px">No dev server configured. Add .vak/launch.toml to preview one here.</div>}>
          {(server) => (
            <div class="prev-row">
              <span class="dot" classList={{ run: server.running }} />
              <span class="prev-name">{server.name}</span>
              <code class="prev-cmd">{[server.cmd, ...server.args].join(" ")}</code>
              <Show when={!server.available && !server.running}>
                <span class="hint" title={server.unavailable_reason}>
                  {server.availability === "port_in_use" ? "port in use" : server.availability === "needs_preparation" ? "needs preparation" : "needs setup"}
                </span>
              </Show>
              <button type="button" class="btn primary sm" disabled={!server.available && !server.running} title="Show it in the Canvas" onClick={() => show(server.name)}>show</button>
              <Show when={server.running} fallback={
                <button type="button" class="btn sm" disabled={!server.available} title={server.available ? "Start without showing it" : server.unavailable_reason} onClick={() => void start(server.name)}>start</button>
              }>
                <button type="button" class="chip sm" classList={{ on: logsFor() === server.name }} onClick={() => setLogsFor(logsFor() === server.name ? null : server.name)}>logs</button>
                <button type="button" class="btn danger sm" onClick={() => void stop(server.name)}>stop</button>
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
