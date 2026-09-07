import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import {
  workbenchExecutions,
  activeExecutionId,
  setActiveExecutionId,
  setWorkbenchExecutions,
  type WorkbenchExecution,
  activeId,
} from "../store";
import * as api from "../api";
import Icon from "./Icon";

export default function WorkbenchPanel() {
  const [tab, setTab] = createSignal<"execution" | "artifacts">("execution");
  const [selectedArtifact, setSelectedArtifact] = createSignal<string | null>(null);
  const [artifactContent, setArtifactContent] = createSignal<string | null>(null);
  const [artifactDataUrl, setArtifactDataUrl] = createSignal<string | null>(null);
  const [loadingArtifact, setLoadingArtifact] = createSignal(false);
  const [artifactError, setArtifactError] = createSignal<string | null>(null);
  const [copied, setCopied] = createSignal(false);

  const executions = () => workbenchExecutions();
  const currentExec = () => {
    const active = activeExecutionId();
    if (active) {
      const found = executions().find((e) => e.id === active);
      if (found) return found;
    }
    return executions()[executions().length - 1] ?? null;
  };

  const allArtifacts = () => {
    const list: Array<{
      path: string;
      mimeType: string;
      sizeBytes: number;
      execCommand: string;
      timestamp: string;
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
    return list;
  };

  const isAnyRunning = () => executions().some((e) => e.status === "running");

  const clearExecutions = () => {
    setWorkbenchExecutions([]);
    setActiveExecutionId(null);
    setSelectedArtifact(null);
  };

  const copyCommand = async (cmd: string) => {
    try {
      await navigator.clipboard.writeText(cmd);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // fallback
    }
  };

  const inspectArtifact = async (path: string) => {
    setSelectedArtifact(path);
    setLoadingArtifact(true);
    setArtifactError(null);
    setArtifactContent(null);
    setArtifactDataUrl(null);
    try {
      const res = await api.readFile(path);
      if (res.data_url) {
        setArtifactDataUrl(res.data_url);
      } else if (res.content !== undefined) {
        setArtifactContent(res.content);
      } else {
        setArtifactError("Unable to read file content.");
      }
    } catch (err) {
      setArtifactError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoadingArtifact(false);
    }
  };

  return (
    <div class="workbench-panel">
      {/* Workbench Header */}
      <div class="workbench-header">
        <div class="workbench-header-left">
          <Icon name="terminal" size={16} />
          <span class="workbench-title">Workbench Sandbox</span>
          <Show when={isAnyRunning()}>
            <span class="workbench-status-badge running">
              <span class="pulse-dot" /> Running
            </span>
          </Show>
          <Show when={executions().length > 0}>
            <span class="workbench-count-pill">{executions().length} run{executions().length === 1 ? "" : "s"}</span>
          </Show>
        </div>
        <div class="workbench-header-right">
          <div class="workbench-nav-tabs">
            <button
              class="workbench-nav-btn"
              classList={{ active: tab() === "execution" }}
              onClick={() => setTab("execution")}
            >
              Live Execution
            </button>
            <button
              class="workbench-nav-btn"
              classList={{ active: tab() === "artifacts" }}
              onClick={() => setTab("artifacts")}
            >
              Artifacts ({allArtifacts().length})
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

      {/* Main Content */}
      <Show when={executions().length === 0}>
        <div class="workbench-empty-state">
          <Icon name="terminal" size={32} />
          <p class="empty-title">No Sandbox Executions Yet</p>
          <p class="empty-desc">
            When the agent runs bash commands, tests, scripts, or installs packages,
            live stdout/stderr streams, exit codes, and artifacts appear here in real time.
          </p>
        </div>
      </Show>

      <Show when={executions().length > 0}>
        <Show when={tab() === "execution"}>
          <div class="workbench-body">
            {/* Left list of runs */}
            <div class="workbench-runs-sidebar">
              <For each={executions()}>
                {(item) => {
                  const isSelected = () => (currentExec()?.id ?? "") === item.id;
                  return (
                    <button
                      class="workbench-run-item"
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
                            failed: item.status === "failed" || (item.exitCode !== undefined && item.exitCode !== 0),
                          }}
                        >
                          {item.status === "running" ? "●" : item.exitCode === 0 ? "✓" : "✗"}
                        </span>
                        <span class="run-time">{item.timestamp}</span>
                        <Show when={item.durationMs !== undefined}>
                          <span class="run-duration">{item.durationMs}ms</span>
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
                            <span>{copied() ? "Copied!" : "Copy"}</span>
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

                    {/* Terminal Stream Console */}
                    <div class="exec-terminal">
                      <div class="exec-terminal-header">
                        <span class="terminal-dot red" />
                        <span class="terminal-dot yellow" />
                        <span class="terminal-dot green" />
                        <span class="terminal-title">Output Stream</span>
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
                                exit code {exec().exitCode ?? 0}
                              </span>
                            }
                          >
                            <span class="status-running">Streaming live…</span>
                          </Show>
                        </div>
                      </div>
                      <div class="exec-terminal-content">
                        <Show
                          when={exec().stdout || exec().stderr}
                          fallback={
                            <div class="terminal-idle">
                              {exec().status === "running" ? "Waiting for output…" : "(no output)"}
                            </div>
                          }
                        >
                          <Show when={exec().stdout}>
                            <pre class="stdout-chunk">{exec().stdout}</pre>
                          </Show>
                          <Show when={exec().stderr}>
                            <pre class="stderr-chunk">{exec().stderr}</pre>
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
                              <button
                                class="artifact-card"
                                onClick={() => inspectArtifact(art.path)}
                              >
                                <Icon name="file" size={14} />
                                <span class="artifact-path">{art.path}</span>
                                <span class="artifact-meta">{art.mimeType} · {art.sizeBytes} B</span>
                              </button>
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
          <div class="workbench-artifacts-tab">
            <div class="artifacts-list-sidebar">
              <Show
                when={allArtifacts().length > 0}
                fallback={<div class="empty-list">No artifacts generated in this session.</div>}
              >
                <For each={allArtifacts()}>
                  {(art) => {
                    const isSelected = () => selectedArtifact() === art.path;
                    return (
                      <button
                        class="artifact-sidebar-item"
                        classList={{ selected: isSelected() }}
                        onClick={() => inspectArtifact(art.path)}
                      >
                        <Icon name="file" size={14} />
                        <div class="art-info">
                          <span class="art-name">{art.path.split("/").pop()}</span>
                          <span class="art-sub">{art.mimeType} · {art.timestamp}</span>
                        </div>
                      </button>
                    );
                  }}
                </For>
              </Show>
            </div>

            {/* Artifact Preview Viewer */}
            <div class="artifact-viewer">
              <Show
                when={selectedArtifact()}
                fallback={
                  <div class="viewer-placeholder">
                    Select an artifact from the list to preview.
                  </div>
                }
              >
                <div class="viewer-header">
                  <span class="viewer-path">{selectedArtifact()}</span>
                  <Show when={loadingArtifact()}>
                    <span class="viewer-loading">Loading…</span>
                  </Show>
                </div>
                <div class="viewer-content">
                  <Show when={artifactError()}>
                    <div class="viewer-error">{artifactError()}</div>
                  </Show>
                  <Show when={artifactDataUrl()}>
                    <div class="image-preview">
                      <img src={artifactDataUrl()!} alt={selectedArtifact()!} />
                    </div>
                  </Show>
                  <Show when={artifactContent() !== null}>
                    <pre class="code-preview"><code>{artifactContent()}</code></pre>
                  </Show>
                </div>
              </Show>
            </div>
          </div>
        </Show>
      </Show>
    </div>
  );
}
