import { createMemo, createSignal, For, Show } from "solid-js";
import { chartGeometry, downloadCsv, type ChartData, type ChartPoint } from "./data";
export type { ChartData, ChartPoint, ChartSeries } from "./data";

export default function UniversalChart(props: { data: ChartData }) {
  const geometry = createMemo(() => chartGeometry(props.data));
  const [selected, setSelected] = createSignal(0);
  const key = () => geometry().keys[Math.min(selected(), geometry().keys.length - 1)];
  const colors = ["var(--accent-bright)", "var(--cyan)", "var(--green)", "var(--muted)"];
  const line = (points: ChartPoint[]) => points.map((point, i) => `${i ? "L" : "M"}${geometry().x(point.x)},${geometry().y(point.y!)}`).join(" ");
  const number = (value: number | null) => value === null ? "Missing" : new Intl.NumberFormat(undefined, { maximumFractionDigits: 4 }).format(value);
  const exportData = () => downloadCsv("chart.csv", [
    [props.data.x_label ?? "X", ...props.data.series.map((series) => series.name)],
    ...geometry().keys.map((x) => [x, ...props.data.series.map((series) => geometry().valueAt(series, x))]),
  ]);
  return <figure class="semantic-chart">
    <figcaption><strong>{props.data.title ?? "Chart"}</strong><p>{props.data.accessible_summary}</p></figcaption>
    <Show when={geometry().keys.length} fallback={<p>No data points supplied.</p>}>
      <svg viewBox="0 0 680 264" role="img" aria-label={props.data.accessible_summary}>
        <title>{props.data.title ?? "Chart"}</title>
        <desc>{props.data.accessible_summary} Use the point selector or data table to inspect exact values.</desc>
        <For each={[0, 0.5, 1]}>{(fraction) => {
          const value = () => geometry().minY + fraction * (geometry().maxY - geometry().minY);
          return <g><line x1="54" x2="626" y1={geometry().y(value())} y2={geometry().y(value())} stroke="var(--border)" /><text x="46" y={geometry().y(value()) + 4} text-anchor="end">{number(value())}</text></g>;
        }}</For>
        <For each={props.data.series}>{(series, index) => <g>
          <Show when={props.data.chart_type !== "bar"}>
            <For each={geometry().segments(series)}>{(points) => <>
              <Show when={props.data.chart_type === "area" && points.length > 1}><path d={`${line(points)} L${geometry().x(points[points.length - 1].x)},${geometry().y(0)} L${geometry().x(points[0].x)},${geometry().y(0)} Z`} fill={colors[index() % colors.length]} opacity="0.12" /></Show>
              <path d={line(points)} fill="none" stroke={colors[index() % colors.length]} stroke-width="2" stroke-dasharray={index() ? `${index() + 2} 3` : undefined} />
              <For each={points}>{(point) => <circle cx={geometry().x(point.x)} cy={geometry().y(point.y!)} r="3" fill={colors[index() % colors.length]}><title>{series.name}: {point.x}, {number(point.y)}</title></circle>}</For>
            </>}</For>
          </Show>
          <Show when={props.data.chart_type === "bar"}>
            <For each={series.points.filter((point) => point.y !== null)}>{(point) => {
              const width = () => Math.min(28, 420 / Math.max(1, geometry().keys.length * props.data.series.length));
              return <rect x={geometry().x(point.x) + (index() - props.data.series.length / 2) * width()} y={Math.min(geometry().y(0), geometry().y(point.y!))} width={width() - 1} height={Math.abs(geometry().y(point.y!) - geometry().y(0))} fill={colors[index() % colors.length]}><title>{series.name}: {point.x}, {number(point.y)}</title></rect>;
            }}</For>
          </Show>
        </g>}</For>
        <text x="54" y="249">{String(geometry().keys[0])}</text><text x="626" y="249" text-anchor="end">{String(geometry().keys.at(-1))}</text>
      </svg>
      <p class="semantic-chart-axes">{props.data.x_label ?? "X"}{props.data.y_label ? ` · ${props.data.y_label}` : ""}</p>
      <label class="semantic-chart-selector">Inspect point: {String(key())}<input type="range" min="0" max={Math.max(0, geometry().keys.length - 1)} value={selected()} onInput={(event) => setSelected(Number(event.currentTarget.value))} aria-valuetext={String(key())} /></label>
      <dl class="semantic-chart-values"><For each={props.data.series}>{(series) => <div><dt>{series.name}</dt><dd>{number(geometry().valueAt(series, key()))}</dd></div>}</For></dl>
    </Show>
    <details><summary>Data table</summary><div class="semantic-table-wrap"><table class="semantic-table"><thead><tr><th scope="col">{props.data.x_label ?? "X"}</th><For each={props.data.series}>{(series) => <th scope="col">{series.name}</th>}</For></tr></thead><tbody><For each={geometry().keys}>{(x) => <tr><th scope="row">{String(x)}</th><For each={props.data.series}>{(series) => <td>{number(geometry().valueAt(series, x))}</td>}</For></tr>}</For></tbody></table></div></details>
    <button onClick={exportData}>Download CSV</button>
  </figure>;
}
