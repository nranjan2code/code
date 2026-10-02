import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import * as api from "../../api";
import { closeArtifactCanvas, isRunning, setTranscriptViewId, technicalDetails } from "../../store";
import { relAgo } from "../../time";
import { cadenceWords, canRetryDelivery, deliveryCount, deliveryStatusLabel, nextRunWords, routineState, runStatusLabel } from "../../taskWords";
import type { TaskDef } from "../../types";
import { createLoader, problemWords } from "./createLoader";
import LoadState from "./LoadState";
import type { ViewerProps } from "./types";

const STATE_WORDS = { paused: "Paused", running: "Working now", scheduled: "Scheduled" } as const;
/** How often an open routine checks for news while the window is in front. */
const REFRESH_MS = 10_000;

/**
 * One scheduled routine: when it runs, when it runs next, what it did last and
 * whether the result was delivered, with the controls the task list has for
 * one routine. It reads the task list and keeps itself current while open and
 * in front; a routine that has been deleted says so.
 */
export default function AutomationViewer(props: ViewerProps) {
  const taskId = () => (props.subject.kind === "automation" ? props.subject.taskId : "");
  const [latest, setLatest] = createSignal<TaskDef | null>(null);
  const [deleted, setDeleted] = createSignal(false);
  const [note, setNote] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const find = async (id: string) => (await api.listTasks()).tasks.find((task) => task.id === id) ?? null;
  const loader = createLoader(
    taskId,
    async (id) => {
      const found = await find(id);
      if (!found) throw new Error("This routine no longer exists.");
      setDeleted(false);
      setLatest(found);
      return found;
    },
    undefined,
    { reloadOn: () => props.reloadKey },
  );

  // Kept current without a spinner: a refresh that fails keeps what is shown,
  // and one that no longer finds the routine says it is gone.
  let refreshing = false;
  const refresh = async () => {
    if (refreshing) return;
    refreshing = true;
    try {
      const found = await find(taskId());
      if (found) setLatest(found);
      setDeleted(!found);
    } catch { /* Offline is not a change. */ } finally {
      refreshing = false;
    }
  };
  createEffect(() => {
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") void refresh();
    }, REFRESH_MS);
    onCleanup(() => clearInterval(timer));
  });

  const act = async (work: () => Promise<string | void>) => {
    setBusy(true);
    setNote(null);
    try {
      setNote((await work()) ?? null);
      if (latest()?.mail_calendar_scope) window.dispatchEvent(new Event("vak:mail-calendar-changed"));
    } catch (cause) {
      setNote(problemWords(cause));
    } finally {
      setBusy(false);
      await refresh();
    }
  };

  return (
    <div class="automation-view">
      <LoadState loader={loader} label="Opening the routine…" stable>{() =>
        <Show when={!deleted()} fallback={
          <div class="artifact-canvas-error" role="status"><span>This routine was deleted.</span></div>
        }>
          <Show when={latest()}>{(task) => {
            const running = () => !!task().last_session_id && isRunning(task().last_session_id!);
            const state = () => routineState(task(), running());
            return (
              <div class="automation-body">
                <header>
                  <h2>{task().name}</h2>
                  <span class="automation-state" data-state={state()}>{STATE_WORDS[state()]}</span>
                </header>
                <dl>
                  <dt>Runs</dt>
                  <dd>{cadenceWords(task(), technicalDetails())}<Show when={technicalDetails() && task().timezone}>{(zone) => <> · on {zone()} time</>}</Show></dd>
                  <dt>Next run</dt>
                  <dd>
                    <Show when={task().enabled} fallback="Paused. It will not run until you resume it.">
                      {nextRunWords(task().next_run_at)}
                    </Show>
                  </dd>
                  <dt>What it does</dt>
                  <dd class="automation-what">{task().script ? (technicalDetails() ? task().script : "Runs a script, without an AI model.") : task().prompt}</dd>
                  <Show when={task().last_run_at} fallback={<><dt>Last run</dt><dd>Never run.</dd></>}>
                    {(at) => <>
                      <dt>Last run</dt>
                      <dd>{relAgo(at())}{task().last_run_status ? ` · ${runStatusLabel(task().last_run_status!)}` : ""}</dd>
                    </>}
                  </Show>
                  <Show when={task().last_summary}>
                    <dt>Result</dt>
                    <dd class="automation-summary">{task().last_summary}</dd>
                  </Show>
                  <Show when={task().last_delivery_state}>
                    <dt>Delivery</dt>
                    <dd>{deliveryStatusLabel(task().last_delivery_state!)}{technicalDetails() && task().deliver_to ? ` · ${task().deliver_to}` : ""}</dd>
                  </Show>
                  <Show when={technicalDetails() && task().model_pin}>
                    <dt>Model</dt>
                    <dd>{task().model_pin} · never escalates</dd>
                  </Show>
                </dl>
                <div class="automation-actions">
                  <button type="button" class="artifact-canvas-btn" disabled={busy() || !task().enabled} onClick={() => void act(async () => { await api.runTaskNow(task().id); return "Started. It runs on the server, so you can close this."; })}>Run now</button>
                  <button type="button" class="artifact-canvas-btn" disabled={busy()} onClick={() => void act(async () => { await api.patchTask(task().id, { enabled: !task().enabled }); })}>{task().enabled ? "Pause" : "Resume"}</button>
                  <Show when={canRetryDelivery(task())}>
                    <button type="button" class="artifact-canvas-btn" disabled={busy()} onClick={() => void act(async () => {
                      const result = await api.retryTaskDelivery(task().id);
                      return result.failed ? `${deliveryCount(result.replayed)} sent again; ${result.failed} still waiting` : `${deliveryCount(result.replayed)} sent again`;
                    })}>Retry delivery</button>
                  </Show>
                  <Show when={task().last_session_id}>
                    <button type="button" class="artifact-canvas-btn" onClick={() => {
                      const session = task().last_session_id!;
                      closeArtifactCanvas();
                      setTranscriptViewId(session);
                    }}>See the last run</button>
                  </Show>
                </div>
                <Show when={note()}>{(message) => <p class="automation-note" role="status">{message()}</p>}</Show>
              </div>
            );
          }}</Show>
        </Show>
      }</LoadState>
    </div>
  );
}
