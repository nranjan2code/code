/// System › Diagnostics › Traces & logs (plan M5b), and the waterfall of
/// one run's spans that Run detail draws. Every line here was written
/// content-free (ids, kinds, counts, durations and outcomes; never text a
/// person wrote, a path or an address), so the screen shows lines as they
/// are. A line's run opens its Run detail, whose id is the trace id.

import { createResource, createSignal, For, Show } from "solid-js";
import { api } from "./api";
import { navigate, route } from "./store";
import type { TelemetryLine, TelemetrySpan } from "./types";

const LEVELS = [
  { value: "error", label: "Errors" },
  { value: "warn", label: "Warnings and worse" },
  { value: "info", label: "Info and worse" },
  { value: "debug", label: "Everything but trace" },
  { value: "trace", label: "Everything" },
] as const;

const LEVEL_TONE: Record<string, string> = {
  ERROR: "chip-tone-danger",
  WARN: "chip-tone-warning",
  INFO: "chip-tone-info",
};

/** Keys every line has; the rest are its fields. */
const STRUCTURAL = new Set(["ts", "level", "target", "service", "message", "event", "span", "spans", "trace_id", "started_at", "duration_ms"]);

function fields(line: TelemetryLine): [string, string][] {
  return Object.entries(line)
    .filter(([key]) => !STRUCTURAL.has(key))
    .map(([key, value]) => [key, typeof value === "string" ? value : JSON.stringify(value)]);
}

function clock(ts?: string): string {
  return ts ? new Date(ts).toLocaleTimeString(undefined, { hour12: false }) : "";
}

/** What a span did, in words a person would use. */
export function spanWords(span: TelemetryLine): string {
  const tool = typeof span.tool === "string" ? span.tool : "a tool";
  switch (span.span) {
    case "run": return "The run";
    case "turn": return "A turn";
    case "step": return `Step ${typeof span.step === "number" ? span.step + 1 : ""}`.trim();
    case "dispatch": return `Asked ${typeof span.model === "string" ? span.model : "the model"}`;
    case "tool_call": return `Used ${tool}`;
    case "execution": return `Ran ${tool} in the tool worker`;
    case "delivery": return "Sent a message";
    default: return span.span ?? "Span";
  }
}

function duration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)} s`;
}

/** The spans of one run laid out on its timeline, nested by depth. */
export function RunWaterfall(props: { run: string }) {
  const [spans] = createResource(() => props.run, (id) => api.runSpans(id));
  const layout = () => {
    const list: TelemetrySpan[] = spans()?.spans ?? [];
    if (list.length === 0) return { rows: [], total: 0 };
    const start = (span: TelemetrySpan) => new Date(span.started_at).getTime();
    const t0 = Math.min(...list.map(start));
    const t1 = Math.max(...list.map((span) => start(span) + span.duration_ms));
    const total = Math.max(t1 - t0, 1);
    return {
      total,
      rows: list.map((span) => ({
        span,
        depth: span.spans?.length ?? 0,
        left: ((start(span) - t0) / total) * 100,
        width: Math.max((span.duration_ms / total) * 100, 0.5),
      })),
    };
  };
  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <span class="eyebrow">Timings</span>
          <h2>Where the time went</h2>
        </div>
        <Show when={layout().total > 0}><span class="dim">{duration(layout().total)} in all</span></Show>
      </div>
      <Show when={!spans.error} fallback={<p class="dim">The timeline could not be loaded: {`${spans.error}`}</p>}>
        <Show when={layout().rows.length > 0} fallback={<p class="dim">{spans.loading ? "Loading the timeline…" : "No timeline was recorded for this run."}</p>}>
          <ol class="waterfall" aria-label="Spans of this run">
            <For each={layout().rows}>
              {(row) => (
                <li class="waterfall-row">
                  <span class="waterfall-label" style={{ "padding-left": `${Math.min(row.depth, 6) * 12}px` }}>
                    {spanWords(row.span)}
                  </span>
                  <span class="waterfall-track">
                    <span
                      class={`waterfall-bar waterfall-${row.span.span}`}
                      style={{ left: `${row.left}%`, width: `${row.width}%` }}
                      title={`${spanWords(row.span)}: ${duration(row.span.duration_ms)}`}
                    />
                  </span>
                  <span class="waterfall-time dim">{duration(row.span.duration_ms)}</span>
                </li>
              )}
            </For>
          </ol>
        </Show>
      </Show>
    </section>
  );
}

function initialTrace(): string {
  const query = route().split("?", 2)[1] ?? "";
  return new URLSearchParams(query).get("trace") ?? "";
}

export default function Diagnostics() {
  const [service, setService] = createSignal("");
  const [level, setLevel] = createSignal("info");
  const [trace, setTrace] = createSignal(initialTrace());
  // A run opened from its detail is mostly timings, so they are on.
  const [withSpans, setWithSpans] = createSignal(initialTrace() !== "");
  const [tick, setTick] = createSignal(0);
  const [services] = createResource(() => api.telemetryServices());
  const [lines] = createResource(
    () => ({ service: service(), level: level(), trace: trace().trim(), spans: withSpans(), tick: tick() }),
    (filter) => api.telemetryLogs({ service: filter.service || undefined, level: filter.level, trace: filter.trace || undefined, spans: filter.spans }),
  );
  return (
    <div class="page">
      <header class="page-header">
        <div>
          <h1>Traces & logs</h1>
          <p class="dim">What each part of Vakyartha reported, newest first. Lines carry ids, counts and timings, never what anyone wrote.</p>
        </div>
        <button class="ghost small" onClick={() => setTick(tick() + 1)}>Refresh</button>
      </header>
      <section class="panel">
        <div class="log-filters">
          <label>
            <span class="dim">From</span>
            <select value={service()} onChange={(event) => setService(event.currentTarget.value)}>
              <option value="">Every service</option>
              <For each={services()?.services ?? []}>{(name) => <option value={name}>{name}</option>}</For>
            </select>
          </label>
          <label>
            <span class="dim">Show</span>
            <select value={level()} onChange={(event) => setLevel(event.currentTarget.value)}>
              <For each={LEVELS}>{(option) => <option value={option.value}>{option.label}</option>}</For>
            </select>
          </label>
          <label class="log-trace">
            <span class="dim">Run</span>
            <input type="search" placeholder="A run id" value={trace()} onChange={(event) => setTrace(event.currentTarget.value)} />
          </label>
          <label class="log-spans">
            <input type="checkbox" checked={withSpans()} onChange={(event) => setWithSpans(event.currentTarget.checked)} />
            <span>Include timings</span>
          </label>
        </div>
        <Show when={!lines.error} fallback={<p class="dim">The logs could not be read: {`${lines.error}`}</p>}>
          <Show when={(lines()?.lines ?? []).length > 0} fallback={<p class="dim">{lines.loading ? "Reading logs…" : "Nothing matches."}</p>}>
            <ol class="log-lines">
              <For each={lines()?.lines ?? []}>
                {(line) => (
                  <li class="log-line">
                    <span class="mono dim">{clock(line.ts)}</span>
                    <span class={`chip ${LEVEL_TONE[line.level] ?? ""}`}>{line.event === "span.close" ? "timing" : line.level.toLowerCase()}</span>
                    <span class="dim">{line.service}</span>
                    <span class="log-message">
                      {line.event === "span.close" ? `${spanWords(line)} took ${duration(Number(line.duration_ms ?? 0))}` : line.message}
                      <span class="log-fields mono dim">
                        <For each={fields(line)}>{([key, value]) => <span>{key}={value}</span>}</For>
                      </span>
                    </span>
                    <Show when={line.trace_id}>
                      {(id) => <button class="ghost small" onClick={() => navigate(`#/runs/${encodeURIComponent(id())}`)}>Open run</button>}
                    </Show>
                  </li>
                )}
              </For>
            </ol>
          </Show>
        </Show>
      </section>
    </div>
  );
}
