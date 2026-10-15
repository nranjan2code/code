import { createSignal, Show } from "solid-js";
import type { Repeat, Trigger } from "../types";
import * as api from "../api";
import { relAgo } from "../time";
import Icon from "./Icon";
import SchedulePicker from "./SchedulePicker";

/** An automation a turn made or changed, drawn from its own record under
 * that turn (docs/design/94 §5): plain words, next run, last result, and
 * the same operations the Automations page and the Agent's tool use. */
export default function AutomationCard(props: { trigger: Trigger; zone?: string; onChanged: () => void }) {
  const [busy, setBusy] = createSignal(false);
  const [picking, setPicking] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [gone, setGone] = createSignal(false);
  const act = async (work: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try { await work(); props.onChanged(); } catch (e) { setError(e instanceof Error ? e.message : String(e)); } finally { setBusy(false); }
  };
  const t = () => props.trigger;
  const next = () => t().enabled && t().next_run_at ? new Date(t().next_run_at!).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "numeric", minute: "2-digit" }) : null;
  const last = () => t().last_run;
  const lastWords = () => {
    const run = last();
    if (!run) return "Not run yet";
    const when = relAgo(run.opened_at);
    if (run.status === "completed") return `Ran ${when}`;
    if (run.status === "running") return "Running now";
    return `Did not finish ${when}${run.reason ? `: ${run.reason}` : ""}`;
  };
  const repeatTo = (repeat: Repeat) => act(async () => { await api.putTrigger(t().id, { ...api.draftOf(t()), kind: undefined, repeat }); setPicking(false); });
  return <Show when={!gone()} fallback={<div class="automation-card removed">Automation “{t().name}” removed.</div>}>
    <section class="automation-card" aria-label={`Automation ${t().name}`}>
      <header><Icon name="timer" /><strong>{t().name}</strong><Show when={!t().enabled}><span class="automation-state">Paused</span></Show></header>
      <p class="automation-when">{t().when_words ?? ""}</p>
      <p class="automation-meta"><Show when={next()}>Next {next()} · </Show>{lastWords()}</p>
      <div class="automation-actions">
        <button type="button" class="btn" disabled={busy()} onClick={() => void act(() => api.putTrigger(t().id, api.draftOf(t(), { enabled: !t().enabled })))}>{t().enabled ? "Pause" : "Resume"}</button>
        <button type="button" class="btn" disabled={busy()} onClick={() => void act(() => api.runTrigger(t().id))}>Run now</button>
        <button type="button" class="btn" disabled={busy()} aria-expanded={picking()} onClick={() => setPicking(!picking())}>Change time</button>
        <button type="button" class="btn danger" disabled={busy()} onClick={() => { if (window.confirm(`Delete “${t().name}”? It will not run again.`)) void act(async () => { await api.deleteTrigger(t().id); setGone(true); }); }}>Delete</button>
      </div>
      <Show when={picking()}><SchedulePicker zone={props.zone} busy={busy()} onPick={(repeat) => void repeatTo(repeat)} /></Show>
      <Show when={error()}><p class="automation-error" role="alert">{error()}</p></Show>
    </section>
  </Show>;
}
