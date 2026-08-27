import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import {
  activeId,
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

const INTERVALS: [number, string][] = [
  [5, "5m"],
  [15, "15m"],
  [30, "30m"],
  [60, "1h"],
  [180, "3h"],
  [720, "12h"],
  [1440, "24h"],
];

/**
 * Runtime task definitions (docs/design/29-personal-os.md): interval or
 * 5-field cron metadata, shell body XOR agent prompt, optional model pin.
 * Server-side `TaskDef::validate` rejections arrive as `{error}` payloads
 * and surface verbatim in the error strip.
 */
export default function TasksModal() {
  const [tasks, setTasks] = createSignal<TaskDef[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [adding, setAdding] = createSignal(false);
  const [name, setName] = createSignal("");
  const [prompt, setPrompt] = createSignal("");
  const [minutes, setMinutes] = createSignal(60);
  const [schedule, setSchedule] = createSignal("");
  const [script, setScript] = createSignal("");
  const [modelPin, setModelPin] = createSignal("");

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

  // Cron wins over the interval picker; prompt and script are mutually
  // exclusive by contract (`TaskDef::validate`), enforced here so the user
  // sees why before the server ever rejects.
  const draftValid = () => {
    const hasPrompt = !!prompt().trim();
    const hasScript = !!script().trim();
    return !!name().trim() && (hasPrompt !== hasScript);
  };

  const resetForm = () => {
    setName("");
    setPrompt("");
    setSchedule("");
    setScript("");
    setModelPin("");
    setMinutes(60);
  };

  const add = async () => {
    if (!draftValid()) return;
    try {
      await api.createTask({
        name: name().trim(),
        prompt: script().trim() ? "" : prompt().trim(),
        interval_secs: minutes() * 60,
        schedule: schedule().trim() || null,
        script: script().trim() || null,
        model_pin: modelPin().trim() || null,
      });
      setError(null);
      resetForm();
      setAdding(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return;
    }
    await refresh();
  };

  const toggle = async (t: TaskDef) => {
    try {
      await api.patchTask(t.id, { enabled: !t.enabled });
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    await refresh();
  };

  const remove = async (t: TaskDef) => {
    try {
      await api.deleteTask(t.id);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    await refresh();
  };

  const cadence = (t: TaskDef): string =>
    t.schedule ? `cron ${t.schedule}` : fmtInterval(t.interval_secs);

  return (
    <Show when={tasksOpen()}>
      <div class="modal-back" onClick={() => setTasksOpen(false)}>
        <div class="modal tasks-modal" role="dialog" aria-modal="true" aria-labelledby="tasks-title" onClick={(e) => e.stopPropagation()}>
          <h3 id="tasks-title">Task definitions</h3>
          <Show when={error()}>
            <div class="gate-err">{error()}</div>
          </Show>

          <For each={tasks()} fallback={<div class="dock-empty">No tasks yet.</div>}>
            {(t) => (
              <div class="task-row">
                <span
                  class="dot"
                  classList={{ idle: !t.enabled }}
                />
                <div class="task-main">
                  <div class="task-name">
                    {t.name}
                    <span class="badge">{cadence(t)}</span>
                    <Show when={t.script}>
                      <span class="badge" title="Shell task definition">script</span>
                    </Show>
                    <Show when={t.model_pin}>
                      <span class="badge" title={`Pinned model — never escalates`}>{t.model_pin}</span>
                    </Show>
                    {!t.enabled && <span class="badge">off</span>}
                  </div>
                  <div class="task-prompt" title={t.script ?? t.prompt}>{t.script ?? t.prompt}</div>
                </div>
                <div class="task-actions">
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
                  disabled={!!script().trim()}
                  value={prompt()}
                  onInput={(e) => setPrompt(e.currentTarget.value)}
                />
                <label class="task-script-row">
                  <textarea
                    rows={2}
                    class="task-script"
                    placeholder="or a shell task body (stored as task metadata)…"
                    disabled={!!prompt().trim()}
                    value={script()}
                    onInput={(e) => setScript(e.currentTarget.value)}
                  />
                  <span class="hint">prompt and script are exclusive</span>
                </label>
                <div class="task-add-row">
                  <input
                    class="schedule-input"
                    placeholder="cron: */15 * * * *"
                    aria-label="Cron schedule"
                    value={schedule()}
                    onInput={(e) => setSchedule(e.currentTarget.value)}
                  />
                  <Show when={!schedule().trim()}>
                    <label class="bo-n">
                      every
                      <select value={String(minutes())} onChange={(e) => setMinutes(Number(e.currentTarget.value))}>
                        <For each={INTERVALS}>{([v, l]) => <option value={String(v)}>{l}</option>}</For>
                      </select>
                    </label>
                  </Show>
                  <input
                    class="model-pin-input"
                    placeholder="pin model (optional)"
                    aria-label="Pinned model for this task"
                    value={modelPin()}
                    onInput={(e) => setModelPin(e.currentTarget.value)}
                  />
                </div>
                <div class="task-add-row">
                  <button class="btn primary" disabled={!draftValid()} onClick={() => void add()}>create</button>
                  <button class="btn" onClick={() => { setAdding(false); setError(null); }}>cancel</button>
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
            <span class="hint">Stored in Runtime SQLite state · enable or disable definitions from any client</span>
            <button class="btn primary" onClick={() => setTasksOpen(false)}>Close</button>
          </div>
        </div>
      </div>
    </Show>
  );
}
