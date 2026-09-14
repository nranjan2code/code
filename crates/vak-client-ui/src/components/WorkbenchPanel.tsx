import { createEffect, createMemo, createSignal, For, Show, onCleanup } from "solid-js";
import {
  workbenchExecutions,
  workbenchLoadError,
  activeExecutionId,
  setActiveExecutionId,
  setWorkbenchExecutions,
  type WorkbenchExecution,
  activeId,
  requestedArtifact,
  setRequestedArtifact,
  openArtifactCanvas,
  openArtifactPathInCanvas,
  workbenchTab,
  setWorkbenchTab,
} from "../store";
import * as api from "../api";
import Icon from "./Icon";
import { artifactPreviewHtml } from "../artifactPreview";

function formatBytes(bytes?: number): string {
  if (!bytes || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function foldCarriageReturns(text: string): string {
  if (!text.includes("\r")) return text;
  const lines = text.split("\n");
  const result: string[] = [];
  for (const line of lines) {
    if (line.includes("\r")) {
      const parts = line.split("\r").filter((p) => p.length > 0);
      result.push(parts[parts.length - 1] ?? "");
    } else {
      result.push(line);
    }
  }
  return result.join("\n");
}

function renderAnsiToHtml(rawText: string): string {
  if (!rawText) return "";
  const text = foldCarriageReturns(rawText);
  const ansiRegex = /\x1b\[([0-9;]*)m/g;
  let html = "";
  let currentIndex = 0;
  let openSpans = 0;

  const colorMap: Record<string, string> = {
    "30": "#64748b",
    "31": "#f87171",
    "32": "#4ade80",
    "33": "#facc15",
    "34": "#60a5fa",
    "35": "#c084fc",
    "36": "#22d3ee",
    "37": "#e2e8f0",
    "90": "#94a3b8",
    "91": "#fca5a5",
    "92": "#86efac",
    "93": "#fde047",
    "94": "#93c5fd",
    "95": "#d8b4fe",
    "96": "#67e8f9",
    "97": "#ffffff",
  };

  function escapeHtml(str: string): string {
    return str
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }

  let match: RegExpExecArray | null;
  while ((match = ansiRegex.exec(text)) !== null) {
    const rawChunk = text.slice(currentIndex, match.index);
    if (rawChunk) {
      html += escapeHtml(rawChunk);
    }
    currentIndex = ansiRegex.lastIndex;

    const codes = match[1] ? match[1].split(";") : ["0"];
    for (const code of codes) {
      if (code === "0" || code === "") {
        while (openSpans > 0) {
          html += "</span>";
          openSpans--;
        }
      } else if (code === "1") {
        html += '<span style="font-weight: 600;">';
        openSpans++;
      } else if (code === "2") {
        html += '<span style="opacity: 0.7;">';
        openSpans++;
      } else if (colorMap[code]) {
        html += `<span style="color: ${colorMap[code]};">`;
        openSpans++;
      }
    }
  }

  const remaining = text.slice(currentIndex);
  if (remaining) {
    html += escapeHtml(remaining);
  }
  while (openSpans > 0) {
    html += "</span>";
    openSpans--;
  }

  return html;
}

export default function WorkbenchPanel() {
  const tab = workbenchTab;
  const setTab = setWorkbenchTab;
  const [selectedArtifact, setSelectedArtifact] = createSignal<string | null>(null);
  const [artifactPreview, setArtifactPreview] = createSignal("");
  const [artifactContent, setArtifactContent] = createSignal<string | null>(null);
  const [artifactDataUrl, setArtifactDataUrl] = createSignal<string | null>(null);
  const [loadingArtifact, setLoadingArtifact] = createSignal(false);
  const [artifactError, setArtifactError] = createSignal<string | null>(null);
  const [copiedCmd, setCopiedCmd] = createSignal(false);
  const [copiedLog, setCopiedLog] = createSignal(false);
  const [stopping, setStopping] = createSignal(false);
  const [candidate, setCandidate] = createSignal<Awaited<ReturnType<typeof api.exportSandboxCandidate>> | null>(null);
  const [candidateBusy, setCandidateBusy] = createSignal(false);
  const [promotionMessage, setPromotionMessage] = createSignal<string | null>(null);
  const [controlError, setControlError] = createSignal<string | null>(null);
  const [pulse, setPulse] = createSignal(0);

  // A quiet interval is still meaningful feedback while the provider or a
  // tool is between events. It also makes the status accessible to screen
  // readers without duplicating the transcript.
  const pulseTimer = window.setInterval(() => setPulse((value) => value + 1), 1000);
  onCleanup(() => window.clearInterval(pulseTimer));
  onCleanup(() => {
    artifactRequest += 1;
    const url = artifactDataUrl();
    if (url?.startsWith("blob:")) URL.revokeObjectURL(url);
  });

  let terminalRef: HTMLDivElement | undefined;
  let artifactRequest = 0;

  createEffect(() => {
    void activeId();
    artifactRequest += 1;
    setSelectedArtifact(null);
    setArtifactContent(null);
    setArtifactError(null);
    setCandidate(null);
    setPromotionMessage(null);
  });

  const executions = () => workbenchExecutions();
  const currentExec = () => {
    const active = activeExecutionId();
    if (active) {
      const found = executions().find((e) => e.id === active);
      if (found) return found;
    }
    return executions()[executions().length - 1] ?? null;
  };

  createEffect(() => {
    const cur = currentExec();
    if (cur && (cur.stdout || cur.stderr) && terminalRef) {
      terminalRef.scrollTop = terminalRef.scrollHeight;
    }
  });

  const allArtifacts = () => {
    const list: Array<{
      path: string;
      mimeType: string;
      sizeBytes: number;
      execCommand: string;
      timestamp: string;
      revision?: number;
    }> = [];
    for (const e of executions()) {
      for (const a of e.artifacts) {
        list.push({
          ...a,
          execCommand: e.command,
          timestamp: e.timestamp,
        });
      }
    }
    return [...new Map(list.map((artifact) => [artifact.path, artifact])).values()];
  };

  const isAnyRunning = () => executions().some((e) => e.status === "running");
  const runningExec = () => executions().find((e) => e.status === "running");

  const clearExecutions = () => {
    setWorkbenchExecutions([]);
    setActiveExecutionId(null);
    setSelectedArtifact(null);
  };

  const copyCommand = async (cmd: string) => {
    try {
      await navigator.clipboard.writeText(cmd);
      setCopiedCmd(true);
      setTimeout(() => setCopiedCmd(false), 2000);
    } catch (error) {
      setControlError(error instanceof Error ? error.message : String(error));
    }
  };

  const copyLog = async (exec: WorkbenchExecution) => {
    setControlError(null);
    try {
      const fullLog = `${exec.stdout ? `[stdout]\n${exec.stdout}\n` : ""}${
        exec.stderr ? `[stderr]\n${exec.stderr}\n` : ""
      }`;
      await navigator.clipboard.writeText(fullLog);
      setCopiedLog(true);
      setTimeout(() => setCopiedLog(false), 2000);
    } catch (error) {
      setControlError(error instanceof Error ? error.message : String(error));
    }
  };

  const stopExecution = async () => {
    const sid = activeId();
    if (!sid) return;
    setStopping(true);
    setControlError(null);
    try {
      await api.cancelRun(sid);
    } catch (error) {
      setControlError(error instanceof Error ? error.message : String(error));
    } finally {
      setTimeout(() => setStopping(false), 1000);
    }
  };

  const inspectArtifact = async (path: string) => {
    const request = ++artifactRequest;
    // Artifact links are an intent to inspect, so move to the artifact
    // viewer immediately instead of leaving the operator on live logs.
    setTab("artifacts");
    setSelectedArtifact(path);
    setLoadingArtifact(true);
    setArtifactError(null);
    setArtifactContent(null);
    const previousUrl = artifactDataUrl();
    if (previousUrl?.startsWith("blob:")) URL.revokeObjectURL(previousUrl);
    setArtifactDataUrl(null);
    try {
      const res = await api.readFile(path);
      if (request !== artifactRequest) return;
      if (res.data_url) {
        setArtifactDataUrl(res.data_url);
      } else if (res.content !== undefined) {
        const preview = /\.html?$/i.test(path) ? await artifactPreviewHtml(path, res.content) : "";
        if (request !== artifactRequest) return;
        setArtifactContent(res.content);
        setArtifactPreview(preview);
      } else {
        // Large/opaque formats use the authenticated raw endpoint instead of
        // forcing every renderer through a base64 JSON response.
        const rawUrl = await api.readFileRaw(path);
        if (request !== artifactRequest) {
          URL.revokeObjectURL(rawUrl);
          return;
        }
        setArtifactDataUrl(rawUrl);
      }
    } catch (err) {
      if (request !== artifactRequest) return;
      setArtifactError(err instanceof Error ? err.message : String(err));
    } finally {
      if (request === artifactRequest) setLoadingArtifact(false);
    }
  };

  createEffect(() => {
    const path = requestedArtifact();
    if (path) { void inspectArtifact(path); setRequestedArtifact(null); }
  });

  const artifactVersion = createMemo(() => allArtifacts().find((artifact) => artifact.path === selectedArtifact())?.revision ?? 0);
  let shownVersion = 0;
  createEffect(() => {
    const version = artifactVersion();
    const path = selectedArtifact();
    if (path && version > shownVersion) void inspectArtifact(path);
    shownVersion = version;
  });

  // A produced file is the outcome of the turn, not an implementation detail.
  // Open the newest outcome automatically when the panel has no selection;
  // users can still switch to activity when they want the mechanics.
  createEffect(() => {
    const artifacts = allArtifacts();
    if (artifacts.length > 0 && selectedArtifact() === null) {
      void inspectArtifact(artifacts[artifacts.length - 1].path);
    }
  });

  const isHtmlArtifact = (path: string) =>
    /\.html?$/i.test(path);

  const isImageArtifact = (path: string, mime?: string) =>
    mime?.startsWith("image/") ||
    /\.(png|jpg|jpeg|gif|svg|webp)$/i.test(path);

  const isPdfArtifact = (path: string, mime?: string) =>
    mime === "application/pdf" || path.toLowerCase().endsWith(".pdf");

  const isAudioArtifact = (path: string, mime?: string) =>
    mime?.startsWith("audio/") || /\.(mp3|wav|ogg|m4a|aac)$/i.test(path);

  const isVideoArtifact = (path: string, mime?: string) =>
    mime?.startsWith("video/") || /\.(mp4|webm|mov|m4v)$/i.test(path);

  const reviewCandidate = async () => {
    const exec = currentExec();
    if (!exec || exec.artifacts.length === 0) return;
    setCandidateBusy(true);
    setPromotionMessage(null);
    try {
      setCandidate(await api.exportSandboxCandidate(`candidate-${exec.id}`, exec.scratchDir, "."));
    } catch (error) {
      setPromotionMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setCandidateBusy(false);
    }
  };

  const promoteCandidate = async () => {
    const value = candidate();
    if (!value) return;
    setCandidateBusy(true);
    try {
      const receipt = await api.promoteSandboxCandidate(value);
      setPromotionMessage(`Applied and verified ${receipt.verification?.length ?? 0} file(s).`);
    } catch (error) {
      setPromotionMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setCandidateBusy(false);
    }
  };

  return (
    <div class="workbench-panel">
      <Show when={controlError()}>
        <div class="inline-error" role="alert">Could not stop this execution: {controlError()}</div>
      </Show>
      {/* Workbench Header */}
      <div class="workbench-header">
        <div class="workbench-header-left">
          <Icon name="terminal" size={16} />
          <span class="workbench-title">Your result</span>
          <Show when={isAnyRunning()}>
            <span class="workbench-status-badge running">
              <span class="pulse-dot" /> Running
            </span>
          </Show>
          <Show when={executions().length > 0}>
            <span class="workbench-count-pill">
              {executions().length} run{executions().length === 1 ? "" : "s"}
            </span>
          </Show>
        </div>
        <div class="workbench-header-right">
          <div class="workbench-nav-tabs" role="tablist" aria-label="Workbench views">
            <button
              class="workbench-nav-btn"
              role="tab"
              id="workbench-tab-execution"
              aria-controls="workbench-panel-execution"
              tabIndex={tab() === "execution" ? 0 : -1}
              aria-selected={tab() === "execution"}
              classList={{ active: tab() === "execution" }}
              onClick={() => setTab("execution")}
            >
              Build activity
            </button>
            <button
              class="workbench-nav-btn"
              role="tab"
              id="workbench-tab-artifacts"
              aria-controls="workbench-panel-artifacts"
              tabIndex={tab() === "artifacts" ? 0 : -1}
              aria-selected={tab() === "artifacts"}
              classList={{ active: tab() === "artifacts" }}
              onClick={() => setTab("artifacts")}
            >
              Files<Show when={allArtifacts().length > 0}> ({allArtifacts().length})</Show>
            </button>
          </div>
          <Show when={executions().length > 0}>
            <button
              class="workbench-clear-btn"
              onClick={clearExecutions}
              title="Clear execution history"
            >
              <Icon name="trash" size={14} />
            </button>
          </Show>
        </div>
      </div>
      <Show when={isAnyRunning()}>
        <div class="workbench-live-strip" aria-live="polite">
          <span class="pulse-dot" />
          <strong>Still working</strong>
          <span class="workbench-live-detail">
            {runningExec()?.stdout || runningExec()?.stderr ? "Receiving live output" : "Waiting for the next event"}
          </span>
          <span class="workbench-live-tick">{pulse() % 2 === 0 ? "·" : "…"}</span>
          <button class="btn subtle" onClick={() => void stopExecution()} disabled={stopping()}>{stopping() ? "Stopping…" : "Stop"}</button>
        </div>
      </Show>

      {/* Main Content */}
      <Show when={executions().length === 0 && !selectedArtifact()}>
        <div class="workbench-empty-state">
          <Icon name="terminal" size={32} />
          <Show when={workbenchLoadError()}>
            <p class="error-state" role="alert">Sandbox telemetry unavailable: {workbenchLoadError()}. Reopen this task to retry.</p>
          </Show>
          <p class="empty-title">No activity yet</p>
          <p class="empty-desc">
            Files and execution details will appear here as Vak works.
          </p>
        </div>
      </Show>

      <Show when={executions().length > 0 || selectedArtifact()}>
        <Show when={tab() === "execution"}>
          <div id="workbench-panel-execution" role="tabpanel" aria-labelledby="workbench-tab-execution" class="workbench-body">
            {/* Left list of runs */}
            <div class="workbench-runs-sidebar">
              <For each={executions()}>
                {(item) => {
                  const isSelected = () => (currentExec()?.id ?? "") === item.id;
                  return (
                    <button
                      class="workbench-run-item"
                      aria-pressed={isSelected()}
                      aria-label={`Execution ${item.timestamp}, ${item.status}${item.exitCode !== undefined ? `, exit code ${item.exitCode}` : ""}`}
                      classList={{
                        selected: isSelected(),
                        failed: item.status === "failed",
                        running: item.status === "running",
                      }}
                      onClick={() => setActiveExecutionId(item.id)}
                    >
                      <div class="run-item-header">
                        <span
                          class="status-indicator"
                          classList={{
                            running: item.status === "running",
                            success: item.status === "completed" && item.exitCode === 0,
                            failed:
                              item.status === "failed" ||
                              (item.exitCode !== undefined && item.exitCode !== 0),
                          }}
                        >
                          {item.status === "running" ? "●" : item.exitCode === 0 ? "✓" : "✗"}
                        </span>
                        <span class="run-time">{item.timestamp}</span>
                        <Show when={item.durationMs !== undefined}>
                          <span class="run-duration">
                            {item.durationMs! < 1000
                              ? `${item.durationMs!}ms`
                              : `${(item.durationMs! / 1000).toFixed(1)}s`}
                          </span>
                        </Show>
                      </div>
                      <div class="run-cmd-snippet" title={item.command}>
                        {item.command}
                      </div>
                    </button>
                  );
                }}
              </For>
            </div>

            {/* Execution Detail View */}
            <div class="workbench-run-detail">
              <Show when={currentExec()}>
                {(exec) => (
                  <div class="exec-detail-container">
                    {/* Command Banner */}
                    <div class="exec-banner">
                      <div class="exec-banner-top">
                        <div class="exec-cmd-info">
                          <span class="badge-lang">{exec().language}</span>
                          <span class="exec-tool-tag">{exec().tool}</span>
                          <Show when={exec().scratchDir}>
                            <span class="exec-cwd-tag" title={exec().scratchDir}>
                              {exec().scratchDir}
                            </span>
                          </Show>
                        </div>
                        <div class="exec-banner-actions">
                          <button
                            class="copy-btn"
                            onClick={() => copyCommand(exec().command)}
                            title="Copy command to clipboard"
                          >
                            <Icon name="copy" size={12} />
                            <span>{copiedCmd() ? "Copied!" : "Copy"}</span>
                          </button>
                        </div>
                      </div>
                      <pre class="exec-cmd-code"><code>{exec().command}</code></pre>
                    </div>

                    {/* Installed Packages Chips */}
                    <Show when={exec().packages.length > 0}>
                      <div class="exec-packages-card">
                        <span class="packages-label">Packages Installed:</span>
                        <div class="packages-chips">
                          <For each={exec().packages}>
                            {(pkg) => <span class="package-chip">{pkg}</span>}
                          </For>
                        </div>
                      </div>
                    </Show>

                    <Show when={exec().artifacts.length > 0}>
                      <div class="exec-packages-card">
                        <span class="packages-label">Workspace promotion</span>
                        <button class="tool-open" onClick={() => void reviewCandidate()} disabled={candidateBusy()}>
                          {candidateBusy() ? "Preparing review…" : "Review candidate"}
                        </button>
                        <Show when={candidate()}>
                          {(review) => (
                            <div style={{ "margin-top": "8px", width: "100%" }}>
                              <div class="artifact-meta">{review().files.length} file(s), hashed against the workspace base</div>
                              <button class="tool-open" onClick={() => void promoteCandidate()} disabled={candidateBusy()}>
                                Apply reviewed candidate
                              </button>
                            </div>
                          )}
                        </Show>
                        <Show when={promotionMessage()}>
                          {(message) => <div class="artifact-meta">{message()}</div>}
                        </Show>
                      </div>
                    </Show>

                    {/* Terminal Stream Console */}
                    <div class="exec-terminal">
                      <div class="exec-terminal-header">
                        <span class="terminal-dot red" />
                        <span class="terminal-dot yellow" />
                        <span class="terminal-dot green" />
                        <span class="terminal-title">Terminal Stream</span>

                        {/* Live Telemetry Badges */}
                        <div style={{ display: "flex", "align-items": "center", gap: "8px", "margin-left": "auto" }}>
                          <Show when={exec().durationMs !== undefined}>
                            <span style={{ "font-size": "10.5px", color: "var(--muted)", "font-family": "monospace" }}>
                              ⏱ {exec().durationMs! < 1000
                                ? `${exec().durationMs}ms`
                                : `${(exec().durationMs! / 1000).toFixed(1)}s`}
                            </span>
                          </Show>

                          <Show when={exec().memoryBytes && exec().memoryBytes! > 0}>
                            <span style={{ "font-size": "10.5px", color: "var(--muted)", "font-family": "monospace" }}>
                              RAM: {formatBytes(exec().memoryBytes)}
                            </span>
                          </Show>

                          <Show when={exec().status === "running"}>
                            <button
                              onClick={stopExecution}
                              disabled={stopping()}
                              style={{
                                display: "inline-flex",
                                "align-items": "center",
                                gap: "4px",
                                background: "rgba(239, 68, 68, 0.2)",
                                color: "#f87171",
                                border: "1px solid rgba(239, 68, 68, 0.4)",
                                "border-radius": "4px",
                                padding: "2px 6px",
                                "font-size": "10.5px",
                                "font-weight": "600",
                                cursor: "pointer",
                              }}
                              title="Stop running command"
                            >
                              <Icon name="stop" size={10} />
                              <span>{stopping() ? "Stopping…" : "Stop"}</span>
                            </button>
                          </Show>

                          <button
                            class="copy-btn"
                            onClick={() => copyLog(exec())}
                            title="Copy full output log"
                          >
                            <Icon name="copy" size={11} />
                            <span>{copiedLog() ? "Copied!" : "Log"}</span>
                          </button>

                          <div class="terminal-status-tag">
                            <Show
                              when={exec().status === "running"}
                              fallback={
                                <span
                                  class="status-code"
                                  classList={{
                                    ok: exec().exitCode === 0,
                                    err: exec().exitCode !== 0,
                                  }}
                                >
                                  exit {exec().exitCode ?? 0}
                                </span>
                              }
                            >
                              <span class="status-running">Running…</span>
                            </Show>
                          </div>
                        </div>
                      </div>

                      <div class="exec-terminal-content" ref={terminalRef}>
                        <Show
                          when={exec().stdout || exec().stderr}
                          fallback={
                            <div class="terminal-idle">
                              {exec().status === "running" ? "Waiting for output…" : "(no output)"}
                            </div>
                          }
                        >
                          <Show when={exec().stdout}>
                            <pre
                              class="stdout-chunk"
                              innerHTML={renderAnsiToHtml(exec().stdout)}
                            />
                          </Show>
                          <Show when={exec().stderr}>
                            <pre
                              class="stderr-chunk"
                              innerHTML={renderAnsiToHtml(exec().stderr)}
                            />
                          </Show>
                          <Show when={exec().outputTruncated}>
                            <div class="terminal-truncated" role="status">
                              Output truncated after 1 MiB; the process continued safely.
                            </div>
                          </Show>
                        </Show>
                      </div>
                    </div>

                    {/* Artifacts generated in this run */}
                    <Show when={exec().artifacts.length > 0}>
                      <div class="exec-artifacts-section">
                        <div class="artifacts-title">Generated Artifacts:</div>
                        <div class="artifacts-grid">
                          <For each={exec().artifacts}>
                            {(art) => (
                              <div class="artifact-card-row">
                                <button
                                  class="artifact-card"
                                  onClick={() => inspectArtifact(art.path)}
                                >
                                  <Icon name="file" size={14} />
                                  <span class="artifact-path">{art.path}</span>
                                  <span class="artifact-meta">
                                    {art.mimeType} · {formatBytes(art.sizeBytes)}
                                  </span>
                                </button>
                                <button
                                  type="button"
                                  class="artifact-card-popout-btn"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    openArtifactPathInCanvas(art.path);
                                  }}
                                  title="Open in Artifact Canvas"
                                >
                                  <Icon name="preview" size={12} />
                                </button>
                              </div>
                            )}
                          </For>
                        </div>
                      </div>
                    </Show>
                  </div>
                )}
              </Show>
            </div>
          </div>
        </Show>

        {/* Artifacts Tab */}
        <Show when={tab() === "artifacts"}>
          <div id="workbench-panel-artifacts" role="tabpanel" aria-labelledby="workbench-tab-artifacts" class="workbench-artifacts-tab">
            <div class="result-intro">
              <div class="result-intro-icon"><Icon name="preview" size={18} /></div>
              <div>
                <h2>What Vak made</h2>
                <p>Open a live preview, review the file, or continue refining it in chat.</p>
              </div>
            </div>
            <div class="result-workspace">
              <Show when={allArtifacts().length > 0}><div class="artifacts-list-sidebar">
              <Show
                when={allArtifacts().length > 0}
                  fallback={<div class="empty-list">Your finished files will appear here.</div>}
              >
                <For each={allArtifacts()}>
                  {(art) => {
                    const isSelected = () => selectedArtifact() === art.path;
                    return (
                      <div class="artifact-sidebar-item-row">
                        <button
                          class="artifact-sidebar-item"
                          aria-pressed={isSelected()}
                          aria-label={`Preview artifact ${art.path}`}
                          classList={{ selected: isSelected() }}
                          onClick={() => inspectArtifact(art.path)}
                        >
                          <Icon name="file" size={14} />
                          <div class="art-info">
                            <span class="art-name">{art.path.split("/").pop()}</span>
                            <span class="art-sub">
                              {art.mimeType} · {formatBytes(art.sizeBytes)}
                            </span>
                          </div>
                        </button>
                        <button
                          type="button"
                          class="artifact-popout-btn"
                          onClick={(e) => {
                            e.stopPropagation();
                            openArtifactPathInCanvas(art.path);
                          }}
                          title="Open in Artifact Canvas"
                          aria-label={`Open ${art.path} in Artifact Canvas`}
                        >
                          <Icon name="preview" size={13} />
                        </button>
                      </div>
                    );
                  }}
                </For>
              </Show>
              </div>

              {/* Artifact Preview Viewer */}
              </Show><div class="artifact-viewer">
              <Show
                when={selectedArtifact()}
                fallback={
                  <div class="viewer-placeholder">
                  Select a result to open its preview.
                  </div>
                }
              >
                <div class="viewer-header">
                  <span class="viewer-path">{selectedArtifact()}</span>
                  <Show when={loadingArtifact()}>
                    <span class="viewer-loading">Loading…</span>
                  </Show>
                  <button
                    type="button"
                    class="pill-action-btn primary"
                    onClick={() => {
                      const p = selectedArtifact();
                      if (p) {
                        openArtifactCanvas({
                          id: p,
                          title: p.split("/").pop() || "Artifact Preview",
                          artifactPath: p,
                          html: artifactContent() || undefined,
                          timestamp: Date.now(),
                        });
                      }
                    }}
                    title="Open in full/split Artifact Canvas"
                    style={{ "margin-left": "auto" }}
                  >
                    <Icon name="preview" size={12} /> Open in Canvas
                  </button>
                </div>
                <div class="viewer-content">
                  <Show when={artifactError()}>
                    <div class="viewer-error">{artifactError()}</div>
                  </Show>

                  {/* HTML Live Sandboxed Web Preview */}
                  <Show when={isHtmlArtifact(selectedArtifact()!) && artifactContent() !== null}>
                    <div style={{ width: "100%", height: "100%", "min-height": "400px" }}>
                      <iframe
                        srcdoc={artifactPreview()}
                        sandbox="allow-scripts"
                        style={{
                          width: "100%",
                          height: "100%",
                          "min-height": "400px",
                          border: "none",
                          background: "#ffffff",
                          "border-radius": "6px",
                        }}
                        title="Sandbox HTML Preview"
                      />
                    </div>
                  </Show>

                  {/* Image Preview */}
                  <Show when={isImageArtifact(selectedArtifact()!) && artifactDataUrl()}>
                    <div class="image-preview" style={{ "text-align": "center", padding: "16px" }}>
                      <img
                        src={artifactDataUrl()!}
                        alt={selectedArtifact()!}
                        style={{ "max-width": "100%", "max-height": "500px", "border-radius": "4px" }}
                      />
                    </div>
                  </Show>

                  <Show when={isPdfArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) && artifactDataUrl()}>
                    <iframe
                      src={artifactDataUrl()!}
                      title="PDF artifact preview"
                      style={{ width: "100%", height: "100%", "min-height": "520px", border: "none" }}
                    />
                  </Show>

                  <Show when={isAudioArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) && artifactDataUrl()}>
                    <div class="media-preview"><audio src={artifactDataUrl()!} controls preload="metadata" /></div>
                  </Show>

                  <Show when={isVideoArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) && artifactDataUrl()}>
                    <div class="media-preview"><video src={artifactDataUrl()!} controls preload="metadata" /></div>
                  </Show>

                  {/* Code / Text Preview */}
                  <Show
                    when={
                      !isHtmlArtifact(selectedArtifact()!) &&
                      !isImageArtifact(selectedArtifact()!) &&
                      !isPdfArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) &&
                      !isAudioArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) &&
                      !isVideoArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) &&
                      artifactContent() !== null
                    }
                  >
                    <pre class="code-preview"><code>{artifactContent()}</code></pre>
                  </Show>

                  <Show
                    when={
                      artifactDataUrl() !== null &&
                      !isHtmlArtifact(selectedArtifact()!) &&
                      !isImageArtifact(selectedArtifact()!) &&
                      !isPdfArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) &&
                      !isAudioArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) &&
                      !isVideoArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType)
                    }
                  >
                    <div class="artifact-fallback">
                      <Icon name="file" size={24} />
                      <strong>This file is ready</strong>
                      <span>This format cannot be previewed here yet.</span>
                      <a class="btn primary sm" href={artifactDataUrl()!} download={selectedArtifact()!.split("/").pop()}>Download file</a>
                    </div>
                  </Show>
                </div>
              </Show>
              </div>
            </div>
          </div>
        </Show>
      </Show>
    </div>
  );
}
