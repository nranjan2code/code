import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import {
  activeId,
  diffTarget,
  isRunning,
  setDiffTarget,
  setDockTab,
  setTasksOpen,
  tasksOpen,
} from "../store";
import * as api from "../api";
import type { TaskDef } from "../types";

function fmtInterval(s: number): string {
  if (s % 3600 === 0) return `${s / 3600}h`;
  if (s % 60 === 0) return `${s / 60}m`;
  return `${s}s`;
}

export default function TasksModal() {
  const [tasks, setTasks] = createSignal<TaskDef[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [adding, setAdding] = createSignal(false);
  const [name, setName] = createSignal("");
  const [prompt, setPrompt] = createSignal("");
  const [minutes, setMinutes] = createSignal(60);

  const refresh = async () => {
    try {
      setTasks((await api.listTasks()).tasks);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  createEffect(() => {
    if (!tasksOpen()) return;
    void refresh();
    const t = setInterval(() => void refresh(), 15_000);
    onCleanup(() => clearInterval(t));
  });

  const add = async () => {
    if (!name().trim() || !prompt().trim()) return;
    await api.createTask(name().trim(), prompt().trim(), minutes() * 60).catch((e) => setError(String(e)));
    setName("");
    setPrompt("");
    setAdding(false);
    await refresh();
  };

  const toggle = async (t: TaskDef) => {
    await api.patchTask(t.id, { enabled: !t.enabled }).catch(() => {});
    await refresh();
  };

  const remove = async (t: TaskDef) => {
    await api.deleteTask(t.id).catch(() => {});
    if (diffTarget() && t.last_session_id === diffTarget()) setDiffTarget(null);
    await refresh();
  };

  const runNow = async (t: TaskDef) => {
    await api.runTaskNow(t.id).catch((e) => setError(e instanceof Error ? e.message : String(e)));
    await refresh();
  };

  const openDiff = (t: TaskDef) => {
    if (!t.last_session_id) return;
    setDiffTarget(t.last_session_id);
    setDockTab("diff");
  };

  return (
    <Show when={tasksOpen()}>
      <div class="modal-back" onClick={() => setTasksOpen(false)}>
        <div class="modal tasks-modal" onClick={(e) => e.stopPropagation()}>
          <h3>Scheduled tasks — recurring runs in isolated worktrees</h3>
          <Show when={error()}>
            <div class="gate-err">{error()}</div>
          </Show>

          <For each={tasks()} fallback={<div class="dock-empty">No tasks yet.</div>}>
            {(t) => (
              <div class="task-row">
                <span
                  class="dot"
                  classList={{
                    run: !!t.last_session_id && isRunning(t.last_session_id),
                    idle: !t.enabled,
                  }}
                />
                <div class="task-main">
                  <div class="task-name">
                    {t.name}
                    <span class="badge">{fmtInterval(t.interval_secs)}</span>
                    {!t.enabled && <span class="badge">off</span>}
                  </div>
                  <div class="task-prompt" title={t.prompt}>{t.prompt}</div>
                  <Show when={t.last_summary}>
                    <div class="task-last">last: {t.last_summary}</div>
                  </Show>
                </div>
                <div class="task-actions">
                  <button class="chip sm" onClick={() => void runNow(t)}>run now</button>
                  <Show when={t.last_session_id}>
                    <button class="chip sm" onClick={() => openDiff(t)}>diff</button>
                  </Show>
                  <button class="chip sm" onClick={() => void toggle(t)}>
                    {t.enabled ? "pause" : "resume"}
                  </button>
                  <button class="chip sm danger-chip" onClick={() => void remove(t)}>delete</button>
                </div>
              </div>
            )}
          </For>

          <Show
            when={!adding()}
            fallback={
              <div class="task-add-form">
                <input placeholder="name" value={name()} onInput={(e) => setName(e.currentTarget.value)} />
                <textarea
                  rows={2}
                  placeholder="prompt the agent will run…"
                  value={prompt()}
                  onInput={(e) => setPrompt(e.currentTarget.value)}
                />
                <div class="task-add-row">
                  <label class="bo-n">
                    every
                    <select value={String(minutes())} onChange={(e) => setMinutes(Number(e.currentTarget.value))}>
                      {[[5, "5m"], [15, "15m"], [30, "30m"], [60, "1h"], [180, "3h"], [720, "12h"], [1440, "24h"]].map(([v, l]) => (
                        <option value={String(v)}>{l}</option>
                      ))}
                    </select>
                  </label>
                  <button class="btn primary" onClick={() => void add()}>create</button>
                  <button class="btn" onClick={() => setAdding(false)}>cancel</button>
                </div>
              </div>
            }
          >
            <div class="task-add-row">
              <button class="btn primary" disabled={!activeId()} onClick={() => setAdding(true)}>
                + New task
              </button>
            </div>
          </Show>

          <div class="bo-foot" style="margin-top:10px">
            <span class="hint">runs fire while the app is open · latest worktree kept for review</span>
            <button class="btn primary" onClick={() => setTasksOpen(false)}>Close</button>
          </div>
        </div>
      </div>
    </Show>
  );
}
