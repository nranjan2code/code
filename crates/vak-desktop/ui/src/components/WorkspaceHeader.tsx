import { createMemo, Show } from "solid-js";
import {
  activeId,
  dockTab,
  isRunning,
  sessions,
  setBestOfOpen,
  setDockTab,
  setHistoryOpen,
  setSettingsOpen,
  setSideOpen,
  setSidebarOpen,
  sideOpen,
  sidebarOpen,
} from "../store";
import Icon, { type IconName } from "./Icon";

const tools: { id: "preview" | "diff" | "terminal" | "editor" | "pr"; label: string; icon: IconName }[] = [
  { id: "preview", label: "Preview", icon: "preview" },
  { id: "diff", label: "Changes", icon: "diff" },
  { id: "terminal", label: "Terminal", icon: "terminal" },
  { id: "editor", label: "Editor", icon: "code" },
  { id: "pr", label: "Pull request", icon: "git" },
];

export default function WorkspaceHeader() {
  const session = createMemo(() => sessions().find((item) => item.session_id === activeId()));
  const title = createMemo(() => session()?.title || (activeId() ? "Untitled task" : "New task"));

  return (
    <header class="workspace-head">
      <div class="workspace-leading">
        <Show when={!sidebarOpen()}>
          <button class="icon-button has-tooltip" data-tooltip="Show sidebar ⌘B" aria-label="Show sidebar" onClick={() => setSidebarOpen(true)}><Icon name="sidebar" /></button>
        </Show>
        <div class="workspace-title">
        <div class="workspace-title-row">
          <h1>{title()}</h1>
          <Show when={activeId()}>
            <span class="run-state" classList={{ active: isRunning(activeId()) }}>
              <span class="dot" classList={{ run: isRunning(activeId()) }} />
              {isRunning(activeId()) ? "Working" : "Ready"}
            </span>
          </Show>
        </div>
        <div class="workspace-meta">
          {activeId() ? `Task ${activeId()!.slice(0, 8)}` : "Choose a task or start a new one"}
        </div>
        </div>
      </div>
      <div class="workspace-actions">
        <button
          class="icon-button has-tooltip"
          data-tooltip="Side question ⌘;"
          classList={{ on: sideOpen() }}
          aria-pressed={sideOpen()}
          title="Ask a side question (⌘;)"
          aria-label="Ask a side question"
          disabled={!activeId()}
          onClick={() => setSideOpen((value) => !value)}
        >
          <Icon name="chat" />
        </button>
        <button
          class="icon-button has-tooltip"
          data-tooltip="Compare approaches"
          title="Compare multiple approaches"
          aria-label="Compare multiple approaches"
          disabled={!activeId()}
          onClick={() => setBestOfOpen(true)}
        >
          <Icon name="layers" />
        </button>
        <button
          class="icon-button has-tooltip"
          data-tooltip="Time travel ⌘H"
          title="Restore a workspace snapshot (⌘H)"
          aria-label="Time travel"
          disabled={!activeId()}
          onClick={() => setHistoryOpen(true)}
        >
          <Icon name="history" />
        </button>
        <span class="action-separator" />
        {tools.map((tool) => (
          <button
            class={`icon-button has-tooltip tool-${tool.id}`}
            data-tooltip={tool.label}
            classList={{ on: dockTab() === tool.id }}
            aria-pressed={dockTab() === tool.id}
            title={tool.label}
            aria-label={tool.label}
            onClick={() => setDockTab((current) => (current === tool.id ? null : tool.id))}
          >
            <Icon name={tool.icon} />
          </button>
        ))}
        <span class="action-separator" />
        <button
          class="icon-button has-tooltip"
          data-tooltip="Settings ⌘,"
          title="Settings (⌘,)"
          aria-label="Open settings"
          onClick={() => setSettingsOpen(true)}
        >
          <Icon name="gear" />
        </button>
      </div>
    </header>
  );
}
