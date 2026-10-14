import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { niceTicks } from "./chartScale";
import { Card, downloadCsv, exportName, num, records, text, type Payload } from "./shared";

interface Point { x: string | number; y: number | null }
interface Series { name: string; points: Point[] }

const PAD = { top: 16, right: 16, bottom: 8, left: 8 };
const PLOT_HEIGHT = 200;
const LINE = 16;

/** A category label in at most two lines of roughly `chars` characters. */
function wrap(label: string, chars: number): string[] {
  const words = label.split(/\s+/);
  const lines: string[] = [""];
  for (const word of words) {
    const current = lines[lines.length - 1];
    if (!current || (current + " " + word).length <= chars) lines[lines.length - 1] = current ? `${current} ${word}` : word;
    else lines.push(word);
  }
  if (lines.length <= 2) return lines;
  return [lines[0], `${lines.slice(1).join(" ").slice(0, Math.max(1, chars - 1))}…`];
}

export default function ChartCard(props: { payload: Payload }) {
  const p = () => props.payload;
  const type = () => (text(p().chart_type) === "bar" ? "bar" : text(p().chart_type) === "area" ? "area" : "line");
  const series = createMemo<Series[]>(() =>
    records(p().series).map((s) => ({
      name: text(s.name) ?? "",
      points: records(s.points).map((point) => ({
        x: typeof point.x === "number" ? point.x : String(point.x ?? ""),
        y: num(point.y) ?? null,
      })),
    })),
  );
  const keys = createMemo(() => [...new Set(series().flatMap((s) => s.points.map((point) => point.x)))]);
  const valueAt = (s: Series, key: string | number) => s.points.find((point) => point.x === key)?.y ?? null;
  const format = (value: number) =>
    new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(value);

  let host: HTMLDivElement | undefined;
  const [width, setWidth] = createSignal(640);
  onMount(() => {
    if (!host) return;
    const observer = new ResizeObserver(([entry]) => setWidth(Math.max(280, Math.round(entry.contentRect.width))));
    observer.observe(host);
    onCleanup(() => observer.disconnect());
  });

  const layout = createMemo(() => {
    const values = series().flatMap((s) => s.points.map((point) => point.y).filter((y): y is number => y !== null));
    const low = Math.min(0, ...values);
    const high = Math.max(0, ...values);
    const ticks = niceTicks(low, high);
    const yMin = ticks[0];
    const yMax = ticks[ticks.length - 1];
    const axisWidth = Math.max(...ticks.map((tick) => format(tick).length)) * 7 + 12;
    const plotLeft = PAD.left + axisWidth;
    const plotRight = width() - PAD.right;
    const band = (plotRight - plotLeft) / Math.max(1, keys().length);
    const chars = Math.max(6, Math.floor(band / 7));
    const labels = keys().map((key) => wrap(String(key), chars));
    const labelLines = Math.max(1, ...labels.map((lines) => lines.length));
    const height = PAD.top + PLOT_HEIGHT + 10 + labelLines * LINE + PAD.bottom;
    const y = (value: number) => PAD.top + PLOT_HEIGHT - ((value - yMin) / (yMax - yMin)) * PLOT_HEIGHT;
    const x = (index: number) => plotLeft + band * (index + 0.5);
    return { ticks, plotLeft, plotRight, band, labels, height, x, y };
  });

  const color = (index: number) => `var(--chart-${(index % 4) + 1})`;
  const exportRows = () => [
    [text(p().x_label) ?? "", ...series().map((s) => s.name)],
    ...keys().map((key) => [key, ...series().map((s) => valueAt(s, key) ?? "")]),
  ];

  return (
    <Card form="chart" label={p().label} title={p().title}>
      <Show when={text(p().accessible_summary)}>{(summary) => <p class="card-lede">{summary()}</p>}</Show>
      <Show when={series().length > 1}>
        <ul class="chart-legend">
          <For each={series()}>{(s, index) => <li><span class="chart-swatch" style={{ background: color(index()) }} />{s.name}</li>}</For>
        </ul>
      </Show>
      <div class="chart-frame" ref={host}>
        <svg width={width()} height={layout().height} role="img" aria-label={text(p().accessible_summary) ?? text(p().title) ?? "Chart"}>
          <For each={layout().ticks}>{(tick) => (
            <g class="chart-tick">
              <line x1={layout().plotLeft} x2={layout().plotRight} y1={layout().y(tick)} y2={layout().y(tick)} />
              <text x={layout().plotLeft - 8} y={layout().y(tick) + 4} text-anchor="end">{format(tick)}</text>
            </g>
          )}</For>
          <Show when={type() === "bar"} fallback={
            <For each={series()}>{(s, index) => {
              const points = () => keys().map((key, i) => ({ i, y: valueAt(s, key) })).filter((point): point is { i: number; y: number } => point.y !== null);
              const path = () => points().map((point, n) => `${n ? "L" : "M"}${layout().x(point.i)},${layout().y(point.y)}`).join(" ");
              return (
                <g>
                  <Show when={type() === "area" && points().length > 1}>
                    <path class="chart-area" fill={color(index())}
                      d={`${path()} L${layout().x(points()[points().length - 1].i)},${layout().y(Math.max(0, layout().ticks[0]))} L${layout().x(points()[0].i)},${layout().y(Math.max(0, layout().ticks[0]))} Z`} />
                  </Show>
                  <path d={path()} fill="none" stroke={color(index())} stroke-width="2" />
                  <For each={points()}>{(point) => (
                    <circle cx={layout().x(point.i)} cy={layout().y(point.y)} r="3.5" fill={color(index())}>
                      <title>{`${s.name ? `${s.name}: ` : ""}${keys()[point.i]}, ${format(point.y)}`}</title>
                    </circle>
                  )}</For>
                </g>
              );
            }}</For>
          }>
            <For each={keys()}>{(key, i) => {
              const group = () => Math.min(layout().band * 0.7, 56 * series().length);
              const barWidth = () => group() / series().length;
              return (
                <For each={series()}>{(s, index) => {
                  const value = () => valueAt(s, key);
                  const zero = () => layout().y(Math.max(0, layout().ticks[0]));
                  return (
                    <Show when={value() !== null}>
                      <g class="chart-bar">
                        <rect
                          x={layout().x(i()) - group() / 2 + index() * barWidth() + 1}
                          y={Math.min(zero(), layout().y(value()!))}
                          width={Math.max(2, barWidth() - 2)}
                          height={Math.abs(layout().y(value()!) - zero())}
                          rx="3"
                          fill={color(index())}
                        />
                        <text x={layout().x(i()) - group() / 2 + (index() + 0.5) * barWidth()} y={layout().y(value()!) - 6} text-anchor="middle">{format(value()!)}</text>
                        <title>{`${s.name ? `${s.name}: ` : ""}${key}, ${format(value()!)}`}</title>
                      </g>
                    </Show>
                  );
                }}</For>
              );
            }}</For>
          </Show>
          <For each={layout().labels}>{(lines, i) => (
            <text class="chart-category" x={layout().x(i())} y={PAD.top + PLOT_HEIGHT + 10 + LINE - 4} text-anchor="middle">
              <For each={lines}>{(line, n) => <tspan x={layout().x(i())} dy={n() ? LINE : 0}>{line}</tspan>}</For>
              <title>{String(keys()[i()])}</title>
            </text>
          )}</For>
        </svg>
      </div>
      <Show when={text(p().x_label) || text(p().y_label)}>
        <p class="card-meta">{[text(p().x_label), text(p().y_label)].filter(Boolean).join(" · ")}</p>
      </Show>
      <footer class="card-foot">
        <details class="chart-data">
          <summary>Show the numbers</summary>
          <table class="card-table">
            <thead><tr><th scope="col">{text(p().x_label) ?? ""}</th><For each={series()}>{(s) => <th scope="col" class="is-numeric">{s.name}</th>}</For></tr></thead>
            <tbody>
              <For each={keys()}>{(key) => (
                <tr><th scope="row">{String(key)}</th><For each={series()}>{(s) => <td class="is-numeric">{valueAt(s, key) === null ? "" : format(valueAt(s, key)!)}</td>}</For></tr>
              )}</For>
            </tbody>
          </table>
        </details>
        <button type="button" class="card-button" onClick={() => downloadCsv(exportName(text(p().title), "chart"), exportRows())}>Download CSV</button>
      </footer>
    </Card>
  );
}
