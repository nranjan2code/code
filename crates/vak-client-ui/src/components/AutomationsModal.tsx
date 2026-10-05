import { createEffect, createResource, createSignal, For, onCleanup, Show } from "solid-js";
import {
  activeId,
  diffTarget,
  openArtifactCanvas,
  setDiffTarget,
  setDockTab,
  setTasksOpen,
  setTaskFocusId,
  setTranscriptViewId,
  tasksOpen,
  taskFocusId,
} from "../store";
import * as api from "../api";
import type { Trigger } from "../types";
import { relAgo } from "../time";
import {
  actionText,
  cadenceBadge,
  canRetryDelivery,
  deliveryCount,
  deliveryStatusLabel,
  nextRunWords,
  runStatusLabel,
  scheduleZone,
} from "../automationWords";
import Icon from "./Icon";
import Sheet from "./Sheet";

const INTERVALS: [number, string][] = [
  [5, "5m"],
  [15, "15m"],
  [30, "30m"],
  [60, "1h"],
  [180, "3h"],
  [720, "12h"],
  [1440, "24h"],
];

/** An automation's recent runs, read from the run records (plan M4.3). */
function RunsPanel(props: { trigger: Trigger; open: boolean }) {
  const [shown, setShown] = createSignal(props.open);
  const [runs] = createResource(
    () => (shown() ? props.trigger.id + (props.trigger.last_run?.id ?? "") : null),
    () => api.triggerRuns(props.trigger.id),
  );
  return (
    <details class="task-run" open={props.open} onToggle={(e) => setShown(e.currentTarget.open)}>
      <summary>
        <Icon name="history" size={12} />
        <span>runs</span>
        <Show when={props.trigger.last_run}>
          {(last) => (
            <span class="task-run-when">
              {runStatusLabel(last().status)} · {relAgo(last().opened_at)}
            </span>
          )}
        </Show>
      </summary>
      <div class="task-run-body">
        <Show when={!runs.error} fallback={<span class="task-run-none">Runs could not be read.</span>}>
          <For each={runs()?.runs ?? []} fallback={<span class="task-run-none">{runs.loading ? "Loading…" : "Not run yet"}</span>}>
            {(run) => (
              <div class="task-run-line">
                <span class="badge">{runStatusLabel(run.status)}</span>
                {" "}
                {relAgo(run.opened_at)} · {new Date(run.opened_at).toLocaleString()}
                <Show when={run.reason}>
                  <p class="task-run-summary">{run.reason}</p>
                </Show>
                <Show when={run.sessions?.[0]}>
                  {(session) => (
                    <button
                      type="button"
                      class="chip sm"
                      title="Open the conversation this run wrote"
                      onClick={() => setTranscriptViewId(session())}
                    >
                      open conversation
                    </button>
                  )}
                </Show>
              </div>
            )}
          </For>
        </Show>
      </div>
    </details>
  );
}

/**
 * Automations (docs/design/29-personal-os.md P2, plan M4.3): an interval or
 * 5-field cron schedule, an Agent prompt XOR a watchdog script, an optional
 * model pin. What each one's runs did is its Runs panel, read from the run
 * records. Server validation rejections arrive as `{error}` payloads and
 * surface verbatim in the error strip.
 */
export default function AutomationsModal() {
  const [triggers, setTriggers] = createSignal<Trigger[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [adding, setAdding] = createSignal(false);
  const [name, setName] = createSignal("");
  const [prompt, setPrompt] = createSignal("");
  const [minutes, setMinutes] = createSignal(60);
  const [schedule, setSchedule] = createSignal("");
  const [script, setScript] = createSignal("");
  const [kind, setKind] = createSignal<"prompt" | "script">("prompt");
  const [modelPin, setModelPin] = createSignal("");
  const [agents, setAgents] = createSignal<api.Agent[]>([]);
  const [agentId, setAgentId] = createSignal("");
  const scrolledIds = new Set<string>();

  const refresh = async () => {
    try {
      setTriggers((await api.listTriggers()).triggers);
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

  createEffect(() => {
    const id = taskFocusId();
    if (!id || !tasksOpen() || scrolledIds.has(id) || !triggers().some((trigger) => trigger.id === id)) return;
    scrolledIds.add(id);
    requestAnimationFrame(() => document.getElementById(`automation-${id}`)?.scrollIntoView({ block: "nearest" }));
  });

  // A prompt and a script are exclusive by contract; enforced here so the
  // person sees why before the server rejects it.
  const draftValid = () => !!name().trim() && !!(kind() === "prompt" ? prompt().trim() : script().trim());

  const resetForm = () => {
    setName("");
    setPrompt("");
    setSchedule("");
    setScript("");
    setKind("prompt");
    setModelPin("");
    setAgentId("");
    setMinutes(60);
  };

  const act = async (work: () => Promise<unknown>) => {
    try {
      await work();
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    await refresh();
  };

  const add = async () => {
    if (!draftValid()) return;
    const cron = schedule().trim();
    try {
      await api.createTrigger({
        name: name().trim(),
        agent: agentId() || null,
        agent_revision: agents().find((agent) => agent.id === agentId())?.revision ?? null,
        kind: {
          kind: "schedule",
          schedule: cron
            ? { kind: "cron", expr: cron, timezone: Intl.DateTimeFormat().resolvedOptions().timeZone }
            : { kind: "interval", every_secs: minutes() * 60, anchor: new Date().toISOString() },
        },
        action: kind() === "script"
          ? { kind: "script", command: script().trim() }
          : { kind: "prompt", text: prompt().trim(), model_pin: modelPin().trim() || null },
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

  const remove = (t: Trigger) =>
    act(async () => {
      await api.deleteTrigger(t.id);
      if (diffTarget() && t.last_run?.sessions?.[0] === diffTarget()) setDiffTarget(null);
    });

  const retryDelivery = async (t: Trigger) => {
    try {
      const result = await api.retryTriggerDelivery(t.id);
      setError(result.failed
        ? `${deliveryCount(result.replayed)} sent again; ${result.failed} still waiting`
        : `${deliveryCount(result.replayed)} sent again`);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const openDiff = (session: string) => {
    setDiffTarget(session);
    setDockTab("diff");
  };

  const openInCanvas = (t: Trigger) => {
    setTasksOpen(false);
    setTaskFocusId(null);
    openArtifactCanvas({ kind: "automation", title: t.name, taskId: t.id });
  };

  return (
    <Show when={tasksOpen()}>
      <Sheet size="wide" class="tasks-modal" title="Automations" subtitle="Prompt automations need a Git project. Scripts can run without Git." onClose={() => { const focused = taskFocusId(); if (focused) scrolledIds.delete(focused); setTasksOpen(false); setTaskFocusId(null); }}>
          <Show when={error()}>
            <div class="gate-err">{error()}</div>
          </Show>

          <For each={triggers()} fallback={<div class="dock-empty">No automations yet.</div>}>
            {(t) => (
              <div id={`automation-${t.id}`} class="task-row" classList={{ focused: taskFocusId() === t.id }}>
                <span class="dot" classList={{ run: !!t.running, idle: !t.enabled }} />
                <div class="task-main">
                  <div class="task-name">
                    {t.name}
                    <span class="badge">{cadenceBadge(t)}</span>
                    <Show when={t.action.kind === "script"}>
                      <span class="badge" title="Runs a shell command, not the model">script</span>
                    </Show>
                    <Show when={t.action.kind === "prompt" && t.action.model_pin}>
                      {(pin) => <span class="badge" title="Pinned model; never escalates">{pin()}</span>}
                    </Show>
                    <Show when={t.agent !== "vak"}>
                      <span class="badge" title={`Agent revision ${t.agent_revision ?? 1}`}>Agent · {agents().find((agent) => agent.id === t.agent)?.name ?? "saved Agent"}{(() => { const current = agents().find((agent) => agent.id === t.agent); return current && t.agent_revision && current.revision !== t.agent_revision ? " · updated" : ""; })()}</span>
                    </Show>
                    {!t.enabled && <span class="badge">off</span>}
                  </div>
                  <div class="task-prompt" title={actionText(t)}>{actionText(t)}</div>
                  <div class="task-schedule-note">
                    Next run {nextRunWords(t.next_run_at)}<Show when={scheduleZone(t)}>{(zone) => <> · runs on {zone()} time</>}</Show>
                    <Show when={t.delivery_state}>{(state) => <> · {deliveryStatusLabel(state())}</>}</Show>
                  </div>
                  <RunsPanel trigger={t} open={taskFocusId() === t.id} />
                </div>
                <div class="task-actions">
                  <button type="button" class="chip sm" onClick={() => openInCanvas(t)} title="See this automation in the Canvas">open</button>
                  <button type="button" class="chip sm" onClick={() => void act(() => api.runTrigger(t.id))}>run now</button>
                  <Show when={canRetryDelivery(t)}>
                    <button type="button" class="chip sm" onClick={() => void retryDelivery(t)}>retry delivery</button>
                  </Show>
                  <Show when={t.last_run?.sessions?.[0]}>
                    {(session) => <button type="button" class="chip sm" onClick={() => openDiff(session())}>diff</button>}
                  </Show>
                  <button type="button" class="chip sm" onClick={() => void act(() => api.putTrigger(t.id, api.draftOf(t, { enabled: !t.enabled })))}>
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
                <div class="task-kind-switch" role="group" aria-label="What it does">
                  <button type="button" class={kind() === "prompt" ? "btn primary" : "btn"} aria-pressed={kind() === "prompt"} onClick={() => setKind("prompt")}>Ask Vakyartha</button>
                  <button type="button" class={kind() === "script" ? "btn primary" : "btn"} aria-pressed={kind() === "script"} onClick={() => setKind("script")}>Run a script</button>
                </div>
                <Show when={kind() === "prompt"}>
                  <textarea
                    rows={2}
                    placeholder="What should Vakyartha do each time this runs?"
                    value={prompt()}
                    onInput={(e) => setPrompt(e.currentTarget.value)}
                  />
                  <span class="hint">Runs in a separate Git project copy so changes can be reviewed.</span>
                </Show>
                <Show when={kind() === "script"}>
                  <label class="watchdog-row">
                  <textarea
                    rows={2}
                    class="watchdog-script"
                    placeholder="A short shell command, such as: printf 'All clear\\n'"
                    value={script()}
                    onInput={(e) => setScript(e.currentTarget.value)}
                  />
                  <span class="hint">Runs without an AI model or Git project.</span>
                  </label>
                </Show>
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
                  <Show when={kind() === "prompt"}>
                    <input
                      class="model-pin-input"
                      placeholder="pin model (optional)"
                      aria-label="Pinned model for this automation"
                      value={modelPin()}
                      onInput={(e) => setModelPin(e.currentTarget.value)}
                    />
                  </Show>
                  <Show when={agents().length > 0}>
                    <select class="model-pin-input" aria-label="Agent for this automation" value={agentId()} onChange={(e) => setAgentId(e.currentTarget.value)}>
                    <option value="">Agent: Vakyartha</option>
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
                + New automation
              </button>
            </div>
          </Show>

          <div class="bo-foot" style="margin-top:10px">
            <span class="hint">Runs on the server, even when this window is closed. Cron times use this device’s time zone.</span>
          </div>
      </Sheet>
    </Show>
  );
}
