/// Work › Runs (plan M4.2): every unit of work Vakyartha did, whatever
/// started it, and how it ended. A run is opened before its work and
/// settled after it; one whose process stopped reads as interrupted. The
/// detail shows each conversation the run wrote, through the same trail
/// the Operations Center draws.

import { createResource, createSignal, For, Show } from "solid-js";
import { api } from "./api";
import { navigate } from "./store";
import type { RunRecord, RunStatus } from "./types";
import { SessionTrail } from "./OperationsCenter";

const STATUS_WORDS: Record<RunStatus, string> = {
  running: "Running",
  completed: "Done",
  failed: "Didn't finish",
  cancelled: "Stopped",
  abandoned: "Interrupted",
  skipped: "Skipped",
};

const STATUS_TONE: Record<RunStatus, string> = {
  running: "chip-tone-info",
  completed: "chip-tone-success",
  failed: "chip-tone-danger",
  cancelled: "",
  abandoned: "chip-tone-warning",
  skipped: "",
};

/** What started a run, in the words a person would use. */
export function runCause(run: RunRecord): string {
  const cause = run.trace?.cause;
  switch (cause?.kind) {
    case "user": return "You asked";
    case "channel": return `Message on ${(cause.endpoint ?? "a channel").split(":")[0]}`;
    case "schedule": return "On schedule";
    case "trigger": return "Started by an automation";
    case "delegation": return "Worker for another run";
    case "revision": return "Revising a result";
    case "heartbeat": return "Check-in";
    case "system": return "Vakyartha";
    default: return "Unknown";
  }
}

function when(value?: string): string {
  return value ? new Date(value).toLocaleString() : "—";
}

function took(run: RunRecord): string {
  if (!run.settled_at) return "";
  const ms = new Date(run.settled_at).getTime() - new Date(run.opened_at).getTime();
  if (ms < 1000) return "under a second";
  const secs = Math.round(ms / 1000);
  return secs < 120 ? `${secs}s` : `${Math.round(secs / 60)} min`;
}

function Status(props: { status: RunStatus }) {
  return <span class={`chip ${STATUS_TONE[props.status]}`}>{STATUS_WORDS[props.status]}</span>;
}

function RunList() {
  const [status, setStatus] = createSignal<string>("");
  const [runs] = createResource(status, (value) => api.runs(value || undefined));
  return (
    <section class="panel run-list">
      <div class="panel-title-row">
        <label class="run-filter">
          <span class="dim">Show</span>
          <select value={status()} onChange={(event) => setStatus(event.currentTarget.value)}>
            <option value="">All runs</option>
            <For each={Object.entries(STATUS_WORDS)}>{([value, label]) => <option value={value}>{label}</option>}</For>
          </select>
        </label>
      </div>
      <Show when={!runs.error} fallback={<p class="dim">Could not load runs: {`${runs.error}`}</p>}>
        <Show when={(runs()?.runs ?? []).length > 0} fallback={<p class="dim">{runs.loading ? "Loading runs…" : "No runs yet."}</p>}>
          <For each={runs()?.runs ?? []}>
            {(run) => (
              <button class="run-row" onClick={() => navigate(`#/runs/${encodeURIComponent(run.id)}`)}>
                <Status status={run.status} />
                <span class="run-row-main">
                  <strong>{runCause(run)}</strong>
                  <span class="dim">{when(run.opened_at)}{took(run) ? ` · took ${took(run)}` : ""}</span>
                </span>
                <Show when={run.reason}><span class="dim run-row-reason">{run.reason}</span></Show>
              </button>
            )}
          </For>
        </Show>
      </Show>
    </section>
  );
}

function RunDetail(props: { id: string }) {
  const [run] = createResource(() => props.id, (id) => api.run(id));
  return (
    <Show when={!run.error} fallback={<section class="panel"><p class="dim">This run could not be loaded: {`${run.error}`}</p></section>}>
      <Show when={run()} fallback={<section class="panel"><p class="dim">Loading run…</p></section>}>
        {(record) => (
          <>
            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <span class="eyebrow">Run</span>
                  <h2>{runCause(record())}</h2>
                  <p class="dim">Started {when(record().opened_at)}{record().settled_at ? ` · ended ${when(record().settled_at)}` : ""}</p>
                </div>
                <Status status={record().status} />
              </div>
              <Show when={record().reason}><p>{record().reason}</p></Show>
              <Show when={record().status === "abandoned"}>
                <p class="dim">The process running this stopped before it finished, so nothing settled it.</p>
              </Show>
              <Show when={record().trace?.cause.kind === "delegation" && record().trace?.cause.parent_run}>
                <button class="ghost small" onClick={() => navigate(`#/runs/${encodeURIComponent(record().trace!.cause.parent_run!)}`)}>Open the run that asked for it</button>
              </Show>
              <details class="run-technical">
                <summary>Technical details</summary>
                <dl class="ops-detail-grid">
                  <div><dt>Run id</dt><dd class="mono">{record().id}</dd></div>
                  <div><dt>Agent</dt><dd class="mono">{record().trace?.agent ?? "—"}</dd></div>
                  <div><dt>Cause</dt><dd class="mono">{record().trace?.cause.kind ?? "—"}</dd></div>
                  <div><dt>Attempt</dt><dd>{record().attempt}</dd></div>
                  <div><dt>Process</dt><dd class="mono">{record().holder ?? "—"}</dd></div>
                  <Show when={record().noticed_by}><div><dt>Noticed by</dt><dd class="mono">{record().noticed_by}</dd></div></Show>
                  <Show when={record().result_id}><div><dt>Answer entry</dt><dd class="mono">{record().result_id}</dd></div></Show>
                </dl>
              </details>
            </section>
            <Show when={(record().sessions ?? []).length > 0} fallback={<section class="panel"><p class="dim">This run wrote no conversation.</p></section>}>
              <For each={record().sessions ?? []}>{(session) => <SessionTrail sessionId={session} />}</For>
            </Show>
          </>
        )}
      </Show>
    </Show>
  );
}

export default function Runs(props: { id?: string }) {
  return (
    <div class="page">
      <header class="page-header">
        <div>
          <h1>Runs</h1>
          <p class="dim">Everything Vakyartha did, whatever started it: a message, a schedule, a worker or a check-in, and how each one ended.</p>
        </div>
        <Show when={props.id}><button class="ghost small" onClick={() => navigate("#/runs")}>All runs</button></Show>
      </header>
      <Show when={props.id} fallback={<RunList />}>{(id) => <RunDetail id={id()} />}</Show>
    </div>
  );
}
