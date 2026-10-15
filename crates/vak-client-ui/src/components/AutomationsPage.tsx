import { createEffect, createResource, createSignal, For, onCleanup, Show } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { activeAgentId, activeId, setAutomationsOpen, automationsOpen, automationFocusId, setTranscriptViewId, technicalDetails } from "../store";
import { activate } from "../App";
import * as api from "../api";
import type { Repeat, Trigger } from "../types";
import { missedWords, runStatusLabel } from "../automationWords";
import { relAgo } from "../time";
import AutomationCard from "./AutomationCard";
import SchedulePicker from "./SchedulePicker";
import Icon from "./Icon";

/** One automation's runs, from its run records, each opening the
 * conversation its result went to. */
function History(props: { trigger: Trigger }) {
  const [open, setOpen] = createSignal(false);
  const [runs] = createResource(() => (open() ? `${props.trigger.id}:${props.trigger.last_run?.id ?? ""}` : null), () => api.triggerRuns(props.trigger.id, 20));
  return <details class="automation-history" onToggle={(event) => setOpen(event.currentTarget.open)}>
    <summary>History</summary>
    <Show when={!runs.error} fallback={<p class="automation-meta">Its runs could not be read.</p>}>
      <ul>
        <For each={runs()?.runs ?? []} fallback={<li class="automation-meta">{runs.loading ? "Loading…" : "Not run yet"}</li>}>
          {(run) => <li>
            <span class="automation-run-status">{runStatusLabel(run.status)}</span> {relAgo(run.opened_at)}
            <Show when={run.reason}><span class="automation-meta"> · {run.reason}</span></Show>
            <Show when={missedWords(run, (at) => new Date(at).toLocaleString())}>{(words) => <span class="automation-meta"> · {words()}</span>}</Show>
            <Show when={technicalDetails() && run.sessions?.[0]}>{(session) => <button type="button" class="chip sm" onClick={() => setTranscriptViewId(session())}>How it got there</button>}</Show>
          </li>}
        </For>
      </ul>
    </Show>
  </details>;
}

/** Every automation (docs/design/94 §5): plain words, next run, last
 * result, history, and making one with a schedule picker. Its buttons are
 * the card's, so the page, the card and the Agent's tool do the same
 * things. */
export default function AutomationsPage() {
  const [all, setAll] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [making, setMaking] = createSignal(false);
  const [name, setName] = createSignal("");
  const [work, setWork] = createSignal("");
  const [script, setScript] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [fetched, { refetch }] = createResource(() => automationsOpen() || null, () => api.listTriggers().then((list) => list.triggers));
  // Kept by id, so a refresh updates each card rather than making it anew
  // (a card's open picker or confirmation survives the refresh).
  const [list, setList] = createStore<Trigger[]>([]);
  createEffect(() => { const next = fetched(); if (next) setList(reconcile(next, { key: "id" })); });
  const [zone] = createResource(() => api.getConfig().then((config) => config.owner?.timezone).catch(() => undefined));
  createEffect(() => {
    if (!automationsOpen()) return;
    const timer = setInterval(() => void refetch(), 15_000);
    onCleanup(() => clearInterval(timer));
  });
  createEffect(() => {
    const id = automationFocusId();
    if (id && list.some((trigger) => trigger.id === id)) requestAnimationFrame(() => document.getElementById(`automation-${id}`)?.scrollIntoView({ block: "nearest" }));
  });
  const shown = () => list.filter((trigger) => !trigger.scope && (all() || trigger.agent === activeAgentId()));
  const create = async (repeat: Repeat) => {
    setBusy(true);
    setError(null);
    try {
      await api.createTrigger({
        name: name().trim(),
        agent: activeAgentId(),
        repeat,
        action: script() ? { kind: "script", command: work().trim() } : { kind: "prompt", text: work().trim() },
        home: activeId() || null,
      });
      setMaking(false);
      setName("");
      setWork("");
      await refetch();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return <div class="library-page automations-page">
    <header class="library-top">
      <button type="button" class="artifact-back" onClick={() => setAutomationsOpen(false)}><Icon name="chevron" size={13} /> Back to chat</button>
      <h2>Automations</h2>
      <p class="library-meta">What runs on its own, when, and what it last did. Results arrive in the conversation each one belongs to.</p>
    </header>
    <div class="automations-bar">
      <label class="automations-all"><input type="checkbox" checked={all()} onChange={(event) => setAll(event.currentTarget.checked)} /> All Agents</label>
      <button type="button" class="btn primary" aria-expanded={making()} onClick={() => setMaking(!making())}>New automation</button>
    </div>
    <Show when={making()}>
      <section class="automation-card automation-new" aria-label="New automation">
        <label>Name<input value={name()} placeholder="Morning line" onInput={(event) => setName(event.currentTarget.value)} /></label>
        <label>{script() ? "Command to run" : "What to do"}<textarea rows="2" value={work()} placeholder={script() ? "df -h /" : "Give me a one-line quote for the day"} onInput={(event) => setWork(event.currentTarget.value)} /></label>
        <Show when={technicalDetails()}><label class="automations-all"><input type="checkbox" checked={script()} onChange={(event) => setScript(event.currentTarget.checked)} /> A shell command instead</label></Show>
        <p class="automation-meta">Its results go to {activeId() ? "the conversation open now" : "this Agent's conversation"}.</p>
        <Show when={name().trim() && work().trim()} fallback={<p class="automation-meta">Name it and say what it does, then pick when.</p>}>
          <SchedulePicker zone={zone()} busy={busy()} label="Create" onPick={(repeat) => void create(repeat)} />
        </Show>
      </section>
    </Show>
    <Show when={error()}><p class="automation-error" role="alert">{error()}</p></Show>
    <Show when={!fetched.error} fallback={<p class="automation-error" role="alert">Automations could not be read.</p>}>
      <For each={shown()} fallback={<p class="library-meta">{fetched.loading ? "Loading…" : "No automations yet. Ask your Agent (“every weekday at 8, …”) or make one here."}</p>}>
        {(trigger) => <div id={`automation-${trigger.id}`} class="automation-row">
          <AutomationCard trigger={trigger} zone={zone()} onChanged={() => void refetch()} />
          <div class="automation-links">
            <Show when={trigger.home}>{(home) => <button type="button" class="chip sm" onClick={() => { setAutomationsOpen(false); void activate(home().session); }}>Open its conversation</button>}</Show>
            <History trigger={trigger} />
          </div>
        </div>}
      </For>
    </Show>
  </div>;
}
