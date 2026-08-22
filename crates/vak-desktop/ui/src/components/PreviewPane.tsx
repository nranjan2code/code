import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import { activeId } from "../store";
import * as api from "../api";

interface ServerCfg {
  name: string;
  cmd: string;
  args: string[];
  port: number | null;
  running: boolean;
}

export default function PreviewPane() {
  const [servers, setServers] = createSignal<ServerCfg[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [active, setActive] = createSignal<string | null>(null);
  const [url, setUrl] = createSignal("");
  const [urlInput, setUrlInput] = createSignal("");
  const [showLogs, setShowLogs] = createSignal(false);
  const [logs, setLogs] = createSignal<string[]>([]);
  let pollLogs: ReturnType<typeof setInterval> | null = null;

  // Preview binds to the active session's workspace.
  const sid = () => activeId();

  const refresh = async () => {
    const id = sid();
    if (!id) return;
    try {
      const res = await api.getLaunch(id);
      setError(res.error ?? null);
      setServers((res.servers as ServerCfg[]) ?? []);
      const runningActive = (res.servers as ServerCfg[])?.find(
        (s) => s.name === active(),
      );
      if (!runningActive?.running) setActive(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  createEffect(() => {
    void sid();
    void refresh();
  });

  const start = async (name: string) => {
    const id = sid();
    if (!id) return;
    setError(null);
    try {
      const res = await api.startLaunch(id, name);
      if (res.error) setError(res.error);
      await refresh();
      const cfg = servers().find((s) => s.name === name);
      if (cfg?.port) {
        setActive(name);
        const u = `http://127.0.0.1:${cfg.port}`;
        setUrl(u);
        setUrlInput(u);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const stop = async (name: string) => {
    const id = sid();
    if (!id) return;
    await api.stopLaunch(id, name).catch(() => {});
    if (active() === name) {
      setActive(null);
      setUrl("");
    }
    await refresh();
  };

  const tailLogs = async () => {
    const id = sid();
    const name = active();
    if (!id || !name || !showLogs()) return;
    const res = await api.launchLogs(id, name).catch(() => null);
    if (res) setLogs(res.lines ?? []);
  };

  createEffect(() => {
    if (showLogs() && active()) {
      void tailLogs();
      pollLogs = setInterval(() => void tailLogs(), 2000);
    } else if (pollLogs) {
      clearInterval(pollLogs);
      pollLogs = null;
    }
  });
  onCleanup(() => pollLogs && clearInterval(pollLogs));

  return (
    <div class="prevpane">
      <div class="dock-head">
        <span>Preview</span>
        <Show when={active()}>
          <input
            class="prev-url"
            value={urlInput()}
            onChange={(e) => {
              setUrlInput(e.currentTarget.value);
              setUrl(e.currentTarget.value);
            }}
            spellcheck={false}
          />
        </Show>
        <button
          class="chip sm"
          classList={{ on: showLogs() }}
          disabled={!active()}
          onClick={() => setShowLogs((v) => !v)}
        >
          logs
        </button>
        <button class="chip sm" onClick={() => void refresh()}>refresh</button>
      </div>

      <Show when={!error()} fallback={<div class="dock-empty">{error()}</div>}>
        <div class="prev-servers">
          <For each={servers()} fallback={<div class="hint" style="padding:4px 10px">No dev server configured — add .vakcoder/launch.toml</div>}>
            {(s) => (
              <div class="prev-row">
                <span
                  class="dot"
                  classList={{ run: s.running }}
                />
                <span class="prev-name">{s.name}</span>
                <code class="prev-cmd">{[s.cmd, ...s.args].join(" ")}</code>
                <Show when={s.port}>
                  <span class="badge">:{s.port}</span>
                </Show>
                <Show
                  when={s.running}
                  fallback={
                    <button class="btn primary sm" onClick={() => void start(s.name)}>
                      start
                    </button>
                  }
                >
                  <button class="btn danger sm" onClick={() => void stop(s.name)}>
                    stop
                  </button>
                </Show>
              </div>
            )}
          </For>
        </div>

        <div class="prev-frame-wrap">
          <Show
            when={url()}
            fallback={<div class="dock-empty">Start a server to preview.</div>}
          >
            <iframe class="prev-frame" src={url()} title="preview" sandbox="allow-scripts allow-forms allow-same-origin" />
          </Show>
        </div>

        <Show when={showLogs()}>
          <pre class="prev-logs">{logs().join("\n")}</pre>
        </Show>
      </Show>
    </div>
  );
}
