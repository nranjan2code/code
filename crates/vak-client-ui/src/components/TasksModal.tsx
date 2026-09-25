import { trapFocus } from "../focusTrap";
import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import {
  activeId,
  diffTarget,
  isRunning,
  setDiffTarget,
  setDockTab,
  setTasksOpen,
  setTranscriptViewId,
  tasksOpen,
} from "../store";
import * as api from "../api";
import type { TaskDef } from "../types";
import { relAgo } from "../time";
import Icon from "./Icon";

function fmtInterval(s: number): string {
  if (s % 3600 === 0) return `${s / 3600}h`;
  if (s % 60 === 0) return `${s / 60}m`;
  return `${s}s`;
}

function runStatusLabel(status: string): string {
  return ({
    working: "Working now",
    complete: "Finished",
    failed: "Couldn’t finish",
    interrupted: "Paused by restart",
  } as Record<string, string>)[status] ?? status;
}

function deliveryStatusLabel(status: string): string {
  return ({
    pending: "Delivery waiting",
    delivered: "Sent",
    queued: "Queued for delivery",
    inbox: "Saved in Inbox",
  } as Record<string, string>)[status] ?? status;
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
 * Scheduled tasks (docs/design/29-personal-os.md P2): interval or 5-field
 * cron ticks, watchdog `script:` XOR agent `prompt`, optional model pin.
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
  const [agents, setAgents] = createSignal<api.Agent[]>([]);
  const [agentId, setAgentId] = createSignal("");

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
    void api.listAgents().then((result) => setAgents(result.agents)).catch(() => { /* Agent selection is optional */ });
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
    setAgentId("");
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
        agent_id: agentId() || null,
        agent_revision: agents().find((agent) => agent.id === agentId())?.revision ?? null,
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
    if (diffTarget() && t.last_session_id === diffTarget()) setDiffTarget(null);
    await refresh();
  };

  const runNow = async (t: TaskDef) => {
    try {
      await api.runTaskNow(t.id);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    await refresh();
  };

  const retryDelivery = async (t: TaskDef) => {
    try {
      const result = await api.retryTaskDelivery(t.id);
      setError(result.failed
        ? `${result.replayed} delivery${result.replayed === 1 ? "" : "ies"} replayed; ${result.failed} still waiting`
        : `${result.replayed} delivery${result.replayed === 1 ? "" : "ies"} replayed`);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const openDiff = (t: TaskDef) => {
    if (!t.last_session_id) return;
    setDiffTarget(t.last_session_id);
    setDockTab("diff");
  };

  const cadence = (t: TaskDef): string =>
    t.schedule ? `cron ${t.schedule}` : fmtInterval(t.interval_secs);

  return (
    <Show when={tasksOpen()}>
      <div class="modal-back" onClick={() => setTasksOpen(false)}>
        <div class="modal tasks-modal" role="dialog" aria-modal="true" aria-labelledby="tasks-title" onClick={(e) => e.stopPropagation()} use:trapFocus>
          <h3 id="tasks-title">Scheduled tasks — recurring runs in isolated worktrees</h3>
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
                    <span class="badge">{cadence(t)}</span>
                    <Show when={t.script}>
                      <span class="badge" title="Watchdog script task — runs shell, not the model">script</span>
                    </Show>
                    <Show when={t.model_pin}>
                      <span class="badge" title={`Pinned model — never escalates`}>{t.model_pin}</span>
                    </Show>
                    <Show when={t.agent_id}>
                      <span class="badge" title={`Agent revision ${t.agent_revision ?? 1}`}>Agent · {agents().find((agent) => agent.id === t.agent_id)?.name ?? "saved Agent"}{(() => { const current = agents().find((agent) => agent.id === t.agent_id); return current && t.agent_revision && current.revision !== t.agent_revision ? " · updated" : ""; })()}</span>
                    </Show>
                    {!t.enabled && <span class="badge">off</span>}
                  </div>
                  <div class="task-prompt" title={t.script ?? t.prompt}>{t.script ?? t.prompt}</div>
                  <div class="task-schedule-note">
                    <Show when={t.next_run_at} fallback="Next run is calculated when the scheduler is available.">
                      {(next) => <>Next run {new Date(next()).toLocaleString()} · {t.timezone ?? "workspace local time"}</>}
                    </Show>
                  </div>
                  <Show when={t.last_run_at || t.last_summary || t.last_session_id}>
                    <details class="task-run">
                      <summary>
                        <Icon name="history" size={12} />
                        <span>last run</span>
                        <Show when={t.last_run_at}>{(at) => <span class="task-run-when">{relAgo(at())}</span>}</Show>
                      </summary>
                      <div class="task-run-body">
                        <Show when={t.last_run_at} fallback={<Show when={!t.last_summary}><span class="task-run-none">never run</span></Show>}>
                          <div class="task-run-line">ran {relAgo(t.last_run_at)} · {new Date(t.last_run_at!).toLocaleString()}</div>
                        </Show>
                        <Show when={t.last_summary}>
                          <p class="task-run-summary">{t.last_summary}</p>
                        </Show>
                        <Show when={t.last_session_id}>
                          <button
                            class="chip sm"
                            title={`Open the transcript of run ${t.last_session_id!.slice(0, 8)}`}
                            onClick={() => setTranscriptViewId(t.last_session_id!)}
                          >
                            open transcript · {t.last_session_id!.slice(0, 8)}
                          </button>
                        </Show>
                        <Show when={t.last_result_id}>
                          <button
                            class="chip sm"
                            title="Open the conversation containing this result"
                            onClick={() => t.last_session_id && setTranscriptViewId(t.last_session_id)}
                          >
                            result · {t.last_result_id!.slice(0, 8)}
                          </button>
                        </Show>
                        <Show when={t.last_run_status}>
                          <span class="badge">{runStatusLabel(t.last_run_status!)}</span>
                        </Show>
                        <Show when={t.last_delivery_state}>
                          <span class="badge">{deliveryStatusLabel(t.last_delivery_state!)}</span>
                        </Show>
                      </div>
                    </details>
                  </Show>
                </div>
                <div class="task-actions">
                  <button type="button" class="chip sm" onClick={() => void runNow(t)}>run now</button>
                  <Show when={t.last_delivery_state === "pending" || t.last_delivery_state === "queued"}>
                    <button type="button" class="chip sm" onClick={() => void retryDelivery(t)}>retry delivery</button>
                  </Show>
                  <Show when={t.last_session_id}>
                    <button type="button" class="chip sm" onClick={() => openDiff(t)}>diff</button>
                  </Show>
                  <button type="button" class="chip sm" onClick={() => void toggle(t)}>
                    {t.enabled ? "pause" : "resume"}
                  </button>
                  <button type="button" class="chip sm danger-chip" onClick={() => void remove(t)}>delete</button>
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
                <label class="watchdog-row">
                  <textarea
                    rows={2}
                    class="watchdog-script"
                    placeholder="or a watchdog script (shell; empty output = silent tick, zero tokens)…"
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
                  <Show when={agents().length > 0}>
                    <select class="model-pin-input" aria-label="Agent for this task" value={agentId()} onChange={(e) => setAgentId(e.currentTarget.value)}>
                    <option value="">Agent: Vakyartha decides</option>
                      <For each={agents()}>{(agent) => <option value={agent.id}>{agent.name}{agents().filter((candidate) => candidate.name.toLowerCase() === agent.name.toLowerCase()).length > 1 ? ` · ${agent.id.slice(0, 6)}` : ""}</option>}</For>
                    </select>
                  </Show>
                </div>
                <div class="task-add-row">
                  <button type="button" class="btn primary" disabled={!draftValid()} onClick={() => void add()}>create</button>
                  <button type="button" class="btn" onClick={() => { setAdding(false); setError(null); }}>cancel</button>
                </div>
              </div>
            }
          >
            <div class="task-add-row">
              <button type="button" class="btn primary" disabled={!activeId()} onClick={() => setAdding(true)}>
                + New task
              </button>
            </div>
          </Show>

          <div class="bo-foot" style="margin-top:10px">
            <span class="hint">runs fire on the gateway server, not this window · cron uses local time · latest worktree kept for review</span>
            <button type="button" class="btn primary" onClick={() => setTasksOpen(false)}>Close</button>
          </div>
        </div>
      </div>
    </Show>
  );
}
