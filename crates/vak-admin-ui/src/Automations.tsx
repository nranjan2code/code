/// Work › Automations (plan M4.3, doc 74 A5): what Vakyartha starts on its
/// own, on a schedule or when you run it. Each automation's last run is read
/// from the run records and opens its Run page; nothing about a run is
/// stored on the automation.

import { createResource, createSignal, For, Show } from "solid-js";
import { api, AuthRequired } from "./api";
import { navigate, pushToast, setAuthed } from "./store";
import type { AutomationItem, RunStatus } from "./types";

const RUN_WORDS: Record<RunStatus, string> = {
  running: "Working now",
  completed: "Finished",
  failed: "Didn't finish",
  cancelled: "Stopped",
  abandoned: "Interrupted",
  skipped: "Didn't run",
};

/** When an automation runs, in words. */
export function cadence(item: Pick<AutomationItem, "kind">): string {
  if (item.kind.kind === "manual") return "Only when you run it";
  const schedule = item.kind.schedule;
  if (schedule.kind === "cron") return `On a schedule (${schedule.expr})`;
  if (schedule.kind === "once") return `Once, ${new Date(schedule.at).toLocaleString()}`;
  const minutes = Math.round(schedule.every_secs / 60);
  return minutes % 60 === 0 ? `Every ${minutes / 60 === 1 ? "hour" : `${minutes / 60} hours`}` : `Every ${minutes} minutes`;
}

/** What an automation does, in words. */
export function doing(item: Pick<AutomationItem, "action">): string {
  return item.action.kind === "prompt" ? item.action.text : `Runs a script: ${item.action.command}`;
}

export default function Automations() {
  const [items, { refetch }] = createResource(() => api.triggers());
  const [busy, setBusy] = createSignal("");
  const [name, setName] = createSignal("");
  const [kind, setKind] = createSignal<"prompt" | "script">("prompt");
  const [text, setText] = createSignal("");
  const [cron, setCron] = createSignal("");
  const [everyMinutes, setEveryMinutes] = createSignal(60);

  const act = async (id: string, work: () => Promise<unknown>, done: string) => {
    if (busy()) return;
    setBusy(id);
    try {
      await work();
      await refetch();
      pushToast("info", done);
    } catch (error) {
      if (error instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${error}`);
    } finally {
      setBusy("");
    }
  };

  const create = () =>
    act("new", async () => {
      await api.createTrigger({
        name: name().trim(),
        kind: {
          kind: "schedule",
          schedule: cron().trim()
            ? { kind: "cron", expr: cron().trim(), timezone: Intl.DateTimeFormat().resolvedOptions().timeZone }
            : { kind: "interval", every_secs: everyMinutes() * 60, anchor: new Date().toISOString() },
        },
        action: kind() === "prompt" ? { kind: "prompt", text: text().trim() } : { kind: "script", command: text().trim() },
      });
      setName("");
      setText("");
      setCron("");
    }, `Added “${name().trim()}”`);

  return (
    <div class="page">
      <header class="page-header">
        <div>
          <h1>Automations</h1>
          <p class="dim">What Vakyartha starts on its own, with nobody watching. Each run uses the same permission mode as any other turn, so one that needs an approval waits for it. A run that asks Vakyartha works in a copy of its folder, and what it changes waits for your review.</p>
        </div>
      </header>
      <section class="panel automation-list">
        <Show when={!items.error} fallback={<p class="dim">Could not load automations: {`${items.error}`}</p>}>
          <Show when={(items()?.triggers ?? []).length > 0} fallback={<p class="dim">{items.loading ? "Loading automations…" : "Nothing is automated yet. Add one below: a nightly digest, a weekly check."}</p>}>
            <For each={items()?.triggers ?? []}>
              {(item) => (
                <div class="automation-row">
                  <div class="automation-row-main">
                    <strong>{item.name}</strong>
                    <span class="dim">{cadence(item)}{item.enabled && item.next_run_at ? ` · next ${new Date(item.next_run_at).toLocaleString()}` : ""}{item.enabled ? "" : " · paused"}</span>
                    <span class="dim automation-row-what">{doing(item)}</span>
                    <Show when={item.last_run}>
                      {(run) => (
                        <button class="link-button" onClick={() => navigate(`#/runs/${encodeURIComponent(run().id)}`)}>
                          Last run: {RUN_WORDS[run().status]} · {new Date(run().opened_at).toLocaleString()}
                        </button>
                      )}
                    </Show>
                  </div>
                  <div class="row-gap">
                    <button class="ghost small" disabled={!!busy()} onClick={() => void act(item.id, () => api.runTrigger(item.id), `Started “${item.name}”`)}>Run now</button>
                    <button class="ghost small" disabled={!!busy()} onClick={() => void act(item.id, () => api.putTrigger(item.id, { ...item, enabled: !item.enabled }), item.enabled ? "Paused" : "Resumed")}>{item.enabled ? "Pause" : "Resume"}</button>
                    <button class="danger small" disabled={!!busy()} onClick={() => { if (confirm(`Delete “${item.name}”? Its past runs stay.`)) void act(item.id, () => api.deleteTrigger(item.id), "Deleted"); }}>Delete</button>
                  </div>
                </div>
              )}
            </For>
          </Show>
        </Show>
      </section>
      <section class="panel automation-new">
        <h2>Add an automation</h2>
        <div class="automation-form">
          <label>Name<input value={name()} onInput={(e) => setName(e.currentTarget.value)} /></label>
          <label>It should
            <select value={kind()} onChange={(e) => setKind(e.currentTarget.value as "prompt" | "script")}>
              <option value="prompt">Ask Vakyartha</option>
              <option value="script">Run a script</option>
            </select>
          </label>
          <label>{kind() === "prompt" ? "What to ask" : "Shell command"}<textarea rows={2} value={text()} onInput={(e) => setText(e.currentTarget.value)} /></label>
          <label>Schedule (optional cron)<input placeholder="0 9 * * 1-5" value={cron()} onInput={(e) => setCron(e.currentTarget.value)} /></label>
          <Show when={!cron().trim()}>
            <label>Or every
              <select value={String(everyMinutes())} onChange={(e) => setEveryMinutes(Number(e.currentTarget.value))}>
                <For each={[15, 30, 60, 180, 720, 1440]}>{(minutes) => <option value={String(minutes)}>{minutes < 60 ? `${minutes} minutes` : `${minutes / 60} hours`}</option>}</For>
              </select>
            </label>
          </Show>
          <button class="primary" disabled={!name().trim() || !text().trim() || !!busy()} onClick={() => void create()}>Add</button>
        </div>
      </section>
    </div>
  );
}
