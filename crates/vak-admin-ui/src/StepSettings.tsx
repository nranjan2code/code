/// Settings › Models: what each kind of step asks of the model
/// (docs/design/88-control-plane.md §4, §12.4). The shared defaults apply
/// on every route; the current route's own settings apply on top of them.
/// A model that cannot do what is asked does what it can, and each run's
/// receipts say what was sent.

import { createResource, createSignal, For, Show } from "solid-js";
import { api } from "./api";
import { pushToast } from "./store";
import type { ControlLayer, StepOverride, TimeLimits } from "./types";

const PURPOSES: [string, string][] = [
  ["act", "Working"],
  ["plan", "Planning"],
  ["repair", "Fixing a mistake"],
  ["judge", "Checking finished work"],
  ["compact", "Shortening history"],
  ["summarise", "Handing over"],
];

const THINKING: [string, string][] = [
  ["default", "The model's own"],
  ["off", "Off"],
  ["low", "Light"],
  ["medium", "Medium"],
  ["high", "Deep"],
  ["xhigh", "Deeper"],
  ["max", "Deepest"],
];

function cleaned(over: StepOverride): StepOverride {
  const next = { ...over };
  for (const key of Object.keys(next) as (keyof StepOverride)[]) {
    const value = next[key];
    if (value === undefined || value === "" || (typeof value === "number" && Number.isNaN(value))) delete next[key];
  }
  return next;
}

export function StepSettingsPanel() {
  const [layer, { refetch }] = createResource(() => api.control("user"));
  const [routeOnly, setRouteOnly] = createSignal(false);
  const row = (purpose: string): StepOverride => {
    const control = layer()?.control;
    return (routeOnly() ? control?.routes?.[layer()!.route]?.[purpose] : control?.steps?.[purpose]) ?? {};
  };
  const save = async (purpose: string, patch: StepOverride) => {
    const current: ControlLayer = layer()?.control ?? {};
    const route = layer()?.route ?? "";
    const table = { ...(routeOnly() ? current.routes?.[route] ?? {} : current.steps ?? {}) };
    const next = cleaned({ ...(table[purpose] ?? {}), ...patch });
    if (Object.keys(next).length) table[purpose] = next;
    else delete table[purpose];
    const control: ControlLayer = routeOnly()
      ? { ...current, routes: { ...(current.routes ?? {}), [route]: table } }
      : { ...current, steps: table };
    if (routeOnly() && !Object.keys(table).length) delete control.routes![route];
    try {
      await api.setControl("user", control);
      await refetch();
    } catch (error) {
      pushToast("alert", `${error}`);
    }
  };
  const number = (value: string) => (value === "" ? undefined : Number(value));
  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>How the model thinks</h2>
          <p class="dim">What each kind of step asks for. Empty fields inherit the default shown.</p>
        </div>
        <label class="dim"><input type="checkbox" checked={routeOnly()} onChange={(e) => setRouteOnly(e.currentTarget.checked)} /> Only for {layer()?.route ?? "this model"}</label>
      </div>
      <Show when={layer()} fallback={<p class="dim">Loading…</p>}>
        <table class="step-settings">
          <thead><tr><th>Step</th><th>Thinking</th><th>Temperature</th><th>Top-p</th><th>Top-k</th></tr></thead>
          <tbody>
            <For each={PURPOSES}>
              {([purpose, label]) => {
                const effective = () => layer()?.effective[purpose] ?? {};
                return (
                  <tr>
                    <td>{label}</td>
                    <td>
                      <select aria-label={`${label}: thinking`} value={row(purpose).reasoning ?? ""} onChange={(e) => void save(purpose, { reasoning: e.currentTarget.value || undefined })}>
                        <option value="">{`Default (${THINKING.find(([k]) => k === effective().reasoning)?.[1] ?? "The model's own"})`}</option>
                        <For each={THINKING}>{([value, text]) => <option value={value}>{text}</option>}</For>
                      </select>
                    </td>
                    <td><input aria-label={`${label}: temperature`} type="number" min="0" max="2" step="0.1" placeholder={effective().temperature?.toString() ?? "model"} value={row(purpose).temperature ?? ""} onChange={(e) => void save(purpose, { temperature: number(e.currentTarget.value) })} /></td>
                    <td><input aria-label={`${label}: top-p`} type="number" min="0" max="1" step="0.05" placeholder={effective().top_p?.toString() ?? "model"} value={row(purpose).top_p ?? ""} onChange={(e) => void save(purpose, { top_p: number(e.currentTarget.value) })} /></td>
                    <td><input aria-label={`${label}: top-k`} type="number" min="1" step="1" placeholder={effective().top_k?.toString() ?? "model"} value={row(purpose).top_k ?? ""} onChange={(e) => void save(purpose, { top_k: number(e.currentTarget.value) })} /></td>
                  </tr>
                );
              }}
            </For>
          </tbody>
        </table>
      </Show>
    </section>
  );
}

const TIME_LIMITS: [keyof TimeLimits, string, string][] = [
  ["command_secs", "A command", "When the call names no limit of its own"],
  ["command_max_secs", "Longest command limit", "The most a command's call may name; a longer one runs with this"],
  ["script_secs", "A scheduled script", "A script an automation runs"],
  ["hook_secs", "A hook", "When the hook sets no limit of its own"],
  ["approval_secs", "An approval", "Waiting for a person; no answer means no"],
  ["question_secs", "A worker's question", "Waiting for a person before the worker goes on without an answer"],
  ["reply_secs", "A channel reply", "How long a channel waits before saying the work is still running"],
  ["probe_secs", "A capacity probe request", "One request of the background check of a model's limits"],
];

/// Settings › Models: the operator's time limits
/// (docs/design/88-control-plane.md §5a). These are the only numbers that
/// end work still in progress; an empty field uses the shipped default.
export function TimeLimitsPanel() {
  const [layer, { refetch }] = createResource(() => api.control("user"));
  const save = async (key: keyof TimeLimits, text: string) => {
    const current: ControlLayer = layer()?.control ?? {};
    const limits = { ...(current.limits ?? {}) };
    const seconds = Math.floor(Number(text));
    if (text === "" || !Number.isFinite(seconds) || seconds < 1) delete limits[key];
    else limits[key] = seconds;
    try {
      await api.setControl("user", { ...current, limits });
      await refetch();
    } catch (error) {
      pushToast("alert", `${error}`);
    }
  };
  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>Time limits</h2>
          <p class="dim">How long each kind of wait may last, in seconds. Empty fields use the default shown.</p>
        </div>
      </div>
      <Show when={layer()} fallback={<p class="dim">Loading…</p>}>
        <table class="step-settings">
          <thead><tr><th>Wait</th><th>Applies</th><th>Seconds</th><th>In force</th></tr></thead>
          <tbody>
            <For each={TIME_LIMITS}>
              {([key, label, applies]) => (
                <tr>
                  <td>{label}</td>
                  <td class="dim">{applies}</td>
                  <td><input aria-label={`${label}: seconds`} type="number" min="1" step="1" placeholder={String(layer()!.shipped_limits[key])} value={layer()!.control.limits?.[key] ?? ""} onChange={(e) => void save(key, e.currentTarget.value)} /></td>
                  <td>{layer()!.limits[key]} s</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </Show>
    </section>
  );
}
