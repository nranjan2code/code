import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import * as api from "../../api";
import { closeArtifactCanvas, setTranscriptViewId, technicalDetails } from "../../store";
import { relAgo } from "../../time";
import { actionText, automationState, cadenceWords, canRetryDelivery, deliveryCount, deliveryStatusLabel, nextRunWords, runStatusLabel, scheduleZone } from "../../automationWords";
import type { Trigger } from "../../types";
import { createLoader, problemWords } from "./createLoader";
import LoadState from "./LoadState";
import type { ViewerProps } from "./types";

const STATE_WORDS = { paused: "Paused", running: "Working now", scheduled: "Scheduled" } as const;
/** How often an open automation checks for news while the window is in front. */
const REFRESH_MS = 10_000;

/**
 * One automation: when it runs, when it runs next, how its last run went
 * (read from the run records) and whether the result was delivered, with the
 * controls the Automations sheet has for one. It keeps itself current while
 * open and in front; one that has been deleted says so.
 */
export default function AutomationViewer(props: ViewerProps) {
  const taskId = () => (props.subject.kind === "automation" ? props.subject.taskId : "");
  const [latest, setLatest] = createSignal<Trigger | null>(null);
  const [deleted, setDeleted] = createSignal(false);
  const [note, setNote] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const find = async (id: string) => (await api.listTriggers()).triggers.find((trigger) => trigger.id === id) ?? null;
  const loader = createLoader(
    taskId,
    async (id) => {
      const found = await find(id);
      if (!found) throw new Error("This automation no longer exists.");
      setDeleted(false);
      setLatest(found);
      return found;
    },
    undefined,
    { reloadOn: () => props.reloadKey },
  );

  // Kept current without a spinner: a refresh that fails keeps what is shown,
  // and one that no longer finds the automation says it is gone.
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
      if (latest()?.scope) window.dispatchEvent(new Event("vak:mail-calendar-changed"));
    } catch (cause) {
      setNote(problemWords(cause));
    } finally {
      setBusy(false);
      await refresh();
    }
  };

  return (
    <div class="automation-view">
      <LoadState loader={loader} label="Opening the automation…" stable>{() =>
        <Show when={!deleted()} fallback={
          <div class="artifact-canvas-error" role="status"><span>This automation was deleted.</span></div>
        }>
          <Show when={latest()}>{(task) => {
            const state = () => automationState(task());
            const lastSession = () => task().last_run?.sessions?.[0];
            return (
              <div class="automation-body">
                <header>
                  <h2>{task().name}</h2>
                  <span class="automation-state" data-state={state()}>{STATE_WORDS[state()]}</span>
                </header>
                <dl>
                  <dt>Runs</dt>
                  <dd>{cadenceWords(task(), technicalDetails())}<Show when={technicalDetails() && scheduleZone(task())}>{(zone) => <> · on {zone()} time</>}</Show></dd>
                  <dt>Next run</dt>
                  <dd>
                    <Show when={task().enabled} fallback="Paused. It will not run until you resume it.">
                      {nextRunWords(task().next_run_at)}
                    </Show>
                  </dd>
                  <dt>What it does</dt>
                  <dd class="automation-what">{task().action.kind === "script" ? (technicalDetails() ? actionText(task()) : "Runs a script, without an AI model.") : actionText(task())}</dd>
                  <Show when={task().last_run} fallback={<><dt>Last run</dt><dd>Never run.</dd></>}>
                    {(run) => <>
                      <dt>Last run</dt>
                      <dd>{relAgo(run().opened_at)} · {runStatusLabel(run().status)}</dd>
                      <Show when={run().reason}>
                        <dt>Why</dt>
                        <dd class="automation-summary">{run().reason}</dd>
                      </Show>
                    </>}
                  </Show>
                  <Show when={task().delivery_state}>
                    <dt>Delivery</dt>
                    <dd>{deliveryStatusLabel(task().delivery_state!)}{technicalDetails() && task().deliver_to ? ` · ${task().deliver_to}` : ""}</dd>
                  </Show>
                  <Show when={technicalDetails() && (() => { const action = task().action; return action.kind === "prompt" ? action.model_pin : null; })()}>
                    {(pin) => <>
                      <dt>Model</dt>
                      <dd>{pin()} · never escalates</dd>
                    </>}
                  </Show>
                </dl>
                <div class="automation-actions">
                  <button type="button" class="artifact-canvas-btn" disabled={busy() || !task().enabled} onClick={() => void act(async () => { await api.runTrigger(task().id); return "Started. It runs on the server, so you can close this."; })}>Run now</button>
                  <button type="button" class="artifact-canvas-btn" disabled={busy()} onClick={() => void act(async () => { await api.putTrigger(task().id, api.draftOf(task(), { enabled: !task().enabled })); })}>{task().enabled ? "Pause" : "Resume"}</button>
                  <Show when={canRetryDelivery(task())}>
                    <button type="button" class="artifact-canvas-btn" disabled={busy()} onClick={() => void act(async () => {
                      const result = await api.retryTriggerDelivery(task().id);
                      return result.failed ? `${deliveryCount(result.replayed)} sent again; ${result.failed} still waiting` : `${deliveryCount(result.replayed)} sent again`;
                    })}>Retry delivery</button>
                  </Show>
                  <Show when={lastSession()}>
                    {(session) => <button type="button" class="artifact-canvas-btn" onClick={() => {
                      closeArtifactCanvas();
                      setTranscriptViewId(session());
                    }}>See the last run</button>}
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
