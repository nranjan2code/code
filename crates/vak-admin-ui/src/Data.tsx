/// Operate › Data (plan M7a-c, docs/design/74 §6.2 A7 and A8): what
/// Vakyartha keeps, measured, and what its retention would remove now.
/// Retention only observes at this stage: the plan is the one it would
/// run, shown and not carried out, and a kind of data nothing watches yet
/// is named as such rather than counted as nothing due.

import { createResource, createSignal, For, Show } from "solid-js";
import { api } from "./api";
import type { DataPlan, DataStatus, DataTransition, DataUsage } from "./types";

function size(bytes: number): string {
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${bytes} B` : `${value.toFixed(1)} ${units[unit]}`;
}

const words = (name: string) => name.replaceAll("_", " ");
const day = (iso: string) => new Date(iso).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });

const DOES: Record<string, string> = { remove: "Remove", trash: "Move to trash" };
const WHY: Record<string, string> = { age: "Past its keep time", size: "Over its size limit" };
const KEPT: Record<string, string> = { held: "On hold", live: "In use", kept: "Kept by a person" };
const KEEP_DAYS = (secs?: number) => (secs ? `${Math.round(secs / 86400)} days` : "");

function Retention() {
  const [status] = createResource<DataStatus>(() => api.dataStatus());
  const [plan, { refetch }] = createResource<DataPlan>(() => api.dataPlan());
  const [made, { refetch: reread }] = createResource<{ transitions: DataTransition[] }>(() => api.dataTransitions());
  const [running, setRunning] = createSignal(false);
  const [ran, setRan] = createSignal("");
  const acting = () => status()?.mode === "commit";
  const runNow = async () => {
    setRunning(true);
    setRan("");
    try {
      const tick = await api.dataTick();
      setRan(
        tick.mode === "commit"
          ? `Removed ${tick.committed.length} ${tick.committed.length === 1 ? "item" : "items"}, ${size(tick.reclaimed_bytes)}.${tick.failed.length ? ` ${tick.failed.length} could not be removed.` : ""}`
          : "Looked again. Nothing was removed.",
      );
      void refetch();
      void reread();
    } catch (error) {
      setRan(`Could not run retention: ${error}`);
    } finally {
      setRunning(false);
    }
  };
  const done = () => (made()?.transitions ?? []).filter((row) => row.state !== "started");
  return (
    <>
      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>What retention would do now</h2>
            <Show
              when={acting()}
              fallback={<p class="dim">Retention is observing only: this is the plan it would run. Nothing here has been removed.</p>}
            >
              <p class="dim">
                Retention removes what is due for: {(status()?.committed ?? []).map(words).join(", ")}. It runs every ten minutes; other kinds are shown and left alone.
              </p>
            </Show>
          </div>
          <button class="ghost small" disabled={plan.loading || running()} onClick={() => void runNow()}>
            {acting() ? "Run now" : "Look again"}
          </button>
        </div>
        <Show when={ran()}><p role="status">{ran()}</p></Show>
        <Show when={!plan.error} fallback={<p class="dim">Could not read the plan: {`${plan.error}`}</p>}>
          <Show when={plan()} fallback={<p class="dim">Reading the data home…</p>}>
            {(planned) => (
              <>
                <p>
                  <strong>{planned().actions.length}</strong> {planned().actions.length === 1 ? "item is" : "items are"} due, {size(planned().reclaimable_bytes)} in all.
                  {" "}<strong>{planned().guarded.length}</strong> past their time are kept back.
                </p>
                <Show when={planned().actions.length > 0}>
                  <div class="ops-table-wrap">
                    <table class="ops-table">
                      <thead><tr><th>Would</th><th>Kind</th><th>Why</th><th>Due since</th><th>Size</th><th>Item</th></tr></thead>
                      <tbody>
                        <For each={planned().actions}>
                          {(action) => (
                            <tr>
                              <td>{DOES[action.does] ?? action.does}</td>
                              <td>{words(action.class)}</td>
                              <td>{WHY[action.reason] ?? action.reason}</td>
                              <td>{day(action.due)}</td>
                              <td>{size(action.bytes)}</td>
                              <td class="mono">{action.item}</td>
                            </tr>
                          )}
                        </For>
                      </tbody>
                    </table>
                  </div>
                </Show>
                <Show when={planned().guarded.length > 0}>
                  <h3>Kept back</h3>
                  <div class="ops-table-wrap">
                    <table class="ops-table">
                      <thead><tr><th>Kind</th><th>Why it stays</th><th>Item</th></tr></thead>
                      <tbody>
                        <For each={planned().guarded}>
                          {(kept) => (
                            <tr><td>{words(kept.class)}</td><td>{KEPT[kept.guard] ?? kept.guard}</td><td class="mono">{kept.item}</td></tr>
                          )}
                        </For>
                      </tbody>
                    </table>
                  </div>
                </Show>
                <Show when={planned().unobserved.length > 0}>
                  <p class="dim">Not watched yet, so nothing is planned for them: {planned().unobserved.map(words).join(", ")}.</p>
                </Show>
              </>
            )}
          </Show>
        </Show>
      </section>
      <Show when={done().length > 0}>
        <section class="panel">
          <h2>What retention has removed</h2>
          <div class="ops-table-wrap">
            <table class="ops-table">
              <thead><tr><th>When</th><th>Result</th><th>Kind</th><th>Why</th><th>Size</th><th>Item</th></tr></thead>
              <tbody>
                <For each={done()}>
                  {(row) => (
                    <tr>
                      <td>{new Date(row.at).toLocaleString()}</td>
                      <td>{row.state === "committed" ? "Removed" : `Could not remove${row.error_kind ? ` (${row.error_kind})` : ""}`}</td>
                      <td>{words(row.class)}</td>
                      <td>{WHY[row.reason] ?? row.reason}</td>
                      <td>{size(row.bytes)}</td>
                      <td class="mono">{row.item}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </section>
      </Show>
      <section class="panel">
        <h2>How long each kind is kept</h2>
        <Show when={status()} fallback={<p class="dim">{status.error ? `Could not read the label: ${status.error}` : "Reading…"}</p>}>
          {(read) => (
            <div class="ops-table-wrap">
              <table class="ops-table">
                <thead><tr><th>Kind</th><th>Kept for</th><th>Size limit</th><th>Then</th><th>Watched</th></tr></thead>
                <tbody>
                  <For each={read().label.rules}>
                    {(rule) => (
                      <tr>
                        <td>{words(rule.class)}</td>
                        <td>{KEEP_DAYS(rule.delete_after_secs)}</td>
                        <td>{rule.max_bytes ? size(rule.max_bytes) : ""}</td>
                        <td>{DOES[rule.on_expiry] ?? rule.on_expiry}</td>
                        <td>{read().observed.includes(rule.class) ? "Yes" : "Not yet"}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          )}
        </Show>
        <p class="dim">Conversations and their files have no keep time: they stay until a person removes them.</p>
      </section>
    </>
  );
}

function Storage() {
  const [usage, { refetch }] = createResource<DataUsage>(() => api.dataUsage());
  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>What is stored</h2>
          <p class="dim">Measured from the files on this machine when the page loaded. Each file is counted once.</p>
        </div>
        <button class="ghost small" disabled={usage.loading} onClick={() => void refetch()}>Measure again</button>
      </div>
      <Show when={!usage.error} fallback={<p class="dim">Could not measure storage: {`${usage.error}`}</p>}>
        <Show when={usage()} fallback={<p class="dim">Measuring…</p>}>
          {(read) => (
            <>
              <p><strong>{size(read().bytes)}</strong> in {read().files} files.</p>
              <div class="ops-table-wrap">
                <table class="ops-table">
                  <thead><tr><th>Where</th><th>Kind</th><th>Part of Vakyartha</th><th>Files</th><th>Size</th></tr></thead>
                  <tbody>
                    <For each={[...read().rows].sort((a, b) => b.bytes - a.bytes)}>
                      {(row) => (
                        <tr><td>{row.root}</td><td>{row.class}</td><td>{row.owner}</td><td>{row.files}</td><td>{size(row.bytes)}</td></tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </>
          )}
        </Show>
      </Show>
    </section>
  );
}

export default function Data(props: { section: "retention" | "storage" }) {
  return (
    <div class="data-page">
      <Show when={props.section === "storage"} fallback={<Retention />}><Storage /></Show>
    </div>
  );
}
