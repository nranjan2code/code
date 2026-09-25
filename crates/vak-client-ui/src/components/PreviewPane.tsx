import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import { activeId, activeComponentPreview, setActiveComponentPreview } from "../store";
import * as api from "../api";
import Icon from "./Icon";
import { sandboxedSrcdoc } from "../safeUrl";
import { artifactPreviewHtml } from "../artifactPreview";

interface ServerCfg {
  name: string;
  cmd: string;
  args: string[];
  port: number | null;
  running: boolean;
  available: boolean;
  availability: "ready" | "needs_setup" | "needs_preparation" | "port_in_use";
  unavailable_reason?: string;
}

export default function PreviewPane() {
  const [activeTab, setActiveTab] = createSignal<"component" | "server">("component");
  const [servers, setServers] = createSignal<ServerCfg[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [activeServer, setActiveServer] = createSignal<string | null>(null);
  const [url, setUrl] = createSignal("");
  const [urlInput, setUrlInput] = createSignal("");
  const [showLogs, setShowLogs] = createSignal(false);
  const [logs, setLogs] = createSignal<string[]>([]);
  const [componentHtml, setComponentHtml] = createSignal<string>("");
  let componentRequest = 0;
  onCleanup(() => { componentRequest += 1; });
  const [componentLoading, setComponentLoading] = createSignal(false);
  const [componentError, setComponentError] = createSignal<string | null>(null);
  const [reloadKey, setReloadKey] = createSignal(0);
  let pollLogs: ReturnType<typeof setInterval> | null = null;

  // Preview binds to the active session's workspace.
  const sid = () => activeId();

  // If a component preview is activated, automatically switch to component view
  createEffect(() => {
    const cp = activeComponentPreview();
    if (cp) {
      setActiveTab("component");
      loadComponentContent(cp.artifactPath, cp.html);
    } else if (!activeServer()) {
      setActiveTab("server");
    }
  });

  const loadComponentContent = async (artifactPath?: string, existingHtml?: string) => {
    const request = ++componentRequest;
    setComponentLoading(true);
    setComponentError(null);
    try {
      const html = existingHtml ?? (artifactPath ? (await api.readFile(artifactPath)).content : undefined);
      if (html === undefined) throw new Error("Preview file is unavailable. Reload to try again.");
      const prepared = artifactPath ? await artifactPreviewHtml(artifactPath, html) : sandboxedSrcdoc(html);
      if (request === componentRequest) setComponentHtml(prepared);
    } catch (error) {
      if (request === componentRequest) setComponentError(error instanceof Error ? error.message : String(error));
    } finally {
      if (request === componentRequest) setComponentLoading(false);
    }
  };

  const refreshServers = async () => {
    const id = sid();
    if (!id) return;
    try {
      const res = await api.getLaunch(id);
      setError(res.error ?? null);
      setServers((res.servers as ServerCfg[]) ?? []);
      const runningActive = (res.servers as ServerCfg[])?.find(
        (s) => s.name === activeServer(),
      );
      if (!runningActive?.running) setActiveServer(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  createEffect(() => {
    void sid();
    void refreshServers();
  });

  const startServer = async (name: string) => {
    const id = sid();
    if (!id) return;
    setError(null);
    try {
      const res = await api.startLaunch(id, name);
      if (res.error) {
        setError(res.error);
        return;
      }
      await refreshServers();
      const cfg = servers().find((s) => s.name === name);
      setActiveServer(name);
      setActiveTab("server");
      if (cfg?.port) {
        const u = `http://127.0.0.1:${cfg.port}`;
        setUrl(u);
        setUrlInput(u);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const stopServer = async (name: string) => {
    const id = sid();
    if (!id) return;
    try {
      await api.stopLaunch(id, name);
      if (activeServer() === name) {
        setActiveServer(null);
        setUrl("");
      }
      await refreshServers();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const tailLogs = async () => {
    const id = sid();
    const name = activeServer();
    if (!id || !name || !showLogs()) return;
    try {
      const res = await api.launchLogs(id, name);
      setLogs(res.lines ?? []);
      setError(null);
    } catch (e) {
      // Keep the last known log lines; an unavailable poll is not an empty
      // log and must not erase evidence from the running preview.
      setError(`Preview logs unavailable: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  createEffect(() => {
    if (showLogs() && activeServer()) {
      void tailLogs();
      pollLogs = setInterval(() => void tailLogs(), 2000);
    } else if (pollLogs) {
      clearInterval(pollLogs);
      pollLogs = null;
    }
  });
  onCleanup(() => pollLogs && clearInterval(pollLogs));

  const reloadComponent = () => {
    setReloadKey((k) => k + 1);
    const cp = activeComponentPreview();
    if (cp) {
      void loadComponentContent(cp.artifactPath, cp.html);
    }
  };

  const popoutComponent = () => {
    const html = componentHtml();
    if (!html) return;
    const blob = new Blob([html], { type: "text/html" });
    const u = URL.createObjectURL(blob);
    window.open(u, "_blank", "noopener,noreferrer");
  };

  const clearComponentPreview = () => {
    setActiveComponentPreview(null);
    setComponentHtml("");
    if (servers().length > 0) {
      setActiveTab("server");
    }
  };

  return (
    <div class="prevpane">
      {/* Header with View Selector & Controls */}
      <div class="dock-head">
        <div style="display: flex; align-items: center; gap: 6px;">
          <button
            type="button"
            class="chip sm"
            classList={{ on: activeTab() === "component" }}
            onClick={() => setActiveTab("component")}
            title="Preview interactive UI / HTML components"
          >
            Component
          </button>
          <button
            type="button"
            class="chip sm"
            classList={{ on: activeTab() === "server" }}
            onClick={() => setActiveTab("server")}
            title="Preview local development servers from .vak/launch.toml"
          >
            Dev Server
          </button>
        </div>

        <Show when={activeTab() === "server"}>
          <Show when={activeServer()}>
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
            disabled={!activeServer()}
            onClick={() => setShowLogs((v) => !v)}
          >
            logs
          </button>
          <button type="button" class="chip sm" onClick={() => void refreshServers()}>
            refresh
          </button>
        </Show>

        <Show when={activeTab() === "component"}>
          <div style="display: flex; gap: 4px; align-items: center;">
            <button
              type="button"
              class="chip sm"
              disabled={!componentHtml()}
              onClick={reloadComponent}
              title="Reload component preview"
            >
              reload
            </button>
            <button
              type="button"
              class="chip sm"
              disabled={!componentHtml()}
              onClick={popoutComponent}
              title="Open preview in new window"
            >
              popout
            </button>
            <Show when={activeComponentPreview()}>
              <button
                type="button"
                class="chip sm"
                onClick={clearComponentPreview}
                title="Clear current component preview"
              >
                clear
              </button>
            </Show>
          </div>
        </Show>
      </div>

      {/* Component Preview Mode */}
      <Show when={activeTab() === "component"}>
        <Show when={activeComponentPreview()} fallback={
          <div class="dock-empty" style="padding: 32px 16px; text-align: center;">
            <div style="font-size: 14px; font-weight: 600; margin-bottom: 6px; color: var(--text);">
              No active component preview
            </div>
            <div class="hint" style="max-width: 260px; line-height: 1.5;">
              Open an interactive HTML preview or static site to inspect live UI here.
            </div>
          </div>
        }>
          {(cp) => (
            <div style="display: flex; flex-direction: column; flex: 1; min-height: 0;">
              {/* Component Info Bar */}
              <div
                style="padding: 6px 10px; background: var(--surface); border-bottom: 1px solid var(--border-soft); display: flex; align-items: center; justify-content: space-between; font-size: 12px;"
              >
                <div style="display: flex; align-items: center; gap: 6px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                  <span class="card-badge badge-indigo" style="font-size: 12px; padding: 1px 6px;">
                    Sandbox
                  </span>
                  <strong style="color: var(--text);">{cp().title}</strong>
                  <span style="color: var(--muted); font-size: 12px;">
                    ({cp().artifactPath})
                  </span>
                </div>
                <span
                  style="color: var(--faint); font-size: 12px;"
                  title="Strict CSP network policy"
                >
                  net: {cp().connectSrc ?? "blocked"}
                </span>
              </div>

              {/* Iframe View */}
              <Show when={componentLoading()}>
                <div style="padding: 24px; text-align: center; color: var(--muted); font-size: 13px;">
                  Loading component…
                </div>
              </Show>

              <Show when={componentError()}>
                <div style="padding: 14px; color: var(--red); background: color-mix(in srgb, var(--red) 8%, transparent); font-size: 12px;">
                  {componentError()}
                </div>
              </Show>

              <Show when={!componentLoading() && !componentError()}>
                <div class="prev-frame-wrap">
                  <iframe
                    class="prev-frame"
                    srcdoc={componentHtml()}
                    title={cp().title}
                    sandbox="allow-scripts"
                  />
                </div>
              </Show>
            </div>
          )}
        </Show>
      </Show>

      {/* Dev Server Mode */}
      <Show when={activeTab() === "server"}>
        <Show when={!error()} fallback={<div class="dock-empty">{error()}</div>}>
          <div class="prev-servers">
            <For
              each={servers()}
              fallback={
                <div class="hint" style="padding:4px 10px">
                  No dev server configured — add .vak/launch.toml
                </div>
              }
            >
              {(s) => (
                <div class="prev-row">
                  <span class="dot" classList={{ run: s.running }} />
                  <span class="prev-name">{s.name}</span>
                  <code class="prev-cmd">{[s.cmd, ...s.args].join(" ")}</code>
                  <Show when={!s.available}>
                    <span class="hint" title={s.unavailable_reason}>
                      {s.availability === "port_in_use" ? "port in use" : s.availability === "needs_preparation" ? "needs preparation" : "needs setup"}
                    </span>
                  </Show>
                  <Show when={s.port}>
                    <span class="badge">:{s.port}</span>
                  </Show>
                  <Show
                    when={s.running}
                    fallback={
                      <button
                        class="btn primary sm"
                        disabled={!s.available}
                        title={s.available ? "Start preview" : s.unavailable_reason}
                        onClick={() => void startServer(s.name)}
                      >
                        start
                      </button>
                    }
                  >
                    <button
                      class="btn danger sm"
                      onClick={() => void stopServer(s.name)}
                    >
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
              <iframe
                class="prev-frame"
                src={url()}
                title="preview"
                sandbox="allow-scripts allow-forms allow-same-origin"
              />
            </Show>
          </div>

          <Show when={showLogs()}>
            <pre class="prev-logs">{logs().join("\n")}</pre>
          </Show>
        </Show>
      </Show>
    </div>
  );
}
