import { For, Show, createSignal, onMount } from "solid-js";

export interface MetricCard {
  label: string;
  value: string;
  delta?: string;
  deltaType?: "up" | "down" | "neutral";
}

export interface ChartPoint {
  x: string | number;
  y: number;
}

export interface ChartSeries {
  name: string;
  color?: string;
  points: ChartPoint[];
}

export interface ChartData {
  title?: string;
  kpis?: MetricCard[];
  series: ChartSeries[];
  x_labels?: string[];
  accessible_summary?: string;
}

export default function UniversalChart(props: { data: ChartData }) {
  let containerRef!: HTMLDivElement;
  let lineRef!: SVGLineElement;
  let ptRef!: SVGCircleElement;

  const [activeX, setActiveX] = createSignal<string>("");
  const [activeValues, setActiveValues] = createSignal<{ name: string; val: number; color: string }[]>([]);
  const [copied, setCopied] = createSignal(false);

  const seriesList = () => props.data.series || [];
  const primarySeries = () => seriesList()[0] || { points: [] };

  const maxY = () => {
    let max = 1;
    for (const s of seriesList()) {
      for (const p of s.points) {
        if (p.y > max) max = p.y;
      }
    }
    return max;
  };

  const minY = () => 0;

  const getPath = (pts: ChartPoint[], width = 700, height = 200) => {
    if (pts.length < 2) return "";
    const range = maxY() - minY() || 1;
    return pts
      .map((p, idx) => {
        const x = (idx / (pts.length - 1)) * width;
        const y = height - 20 - ((p.y - minY()) / range) * (height - 40);
        return `${idx === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(" ");
  };

  const getAreaPath = (pts: ChartPoint[], width = 700, height = 200) => {
    if (pts.length < 2) return "";
    const linePath = getPath(pts, width, height);
    return `${linePath} L${width},${height - 10} L0,${height - 10} Z`;
  };

  const handleMouseMove = (e: MouseEvent) => {
    if (!containerRef) return;
    const rect = containerRef.getBoundingClientRect();
    const relX = Math.max(0, Math.min(rect.width, e.clientX - rect.left));
    const pct = relX / rect.width;
    const pts = primarySeries().points;
    if (!pts.length) return;

    const idx = Math.min(pts.length - 1, Math.max(0, Math.round(pct * (pts.length - 1))));
    const curX = pts[idx]?.x ?? idx;
    setActiveX(String(curX));

    const vals = seriesList().map((s, sIdx) => ({
      name: s.name,
      val: s.points[idx]?.y ?? 0,
      color: s.color ?? (sIdx === 0 ? "var(--accent-bright)" : "var(--cyan)"),
    }));
    setActiveValues(vals);

    const svgX = (idx / Math.max(pts.length - 1, 1)) * 700;
    const range = maxY() - minY() || 1;
    const svgY = 200 - 20 - (((pts[idx]?.y ?? 0) - minY()) / range) * 160;

    if (lineRef) {
      lineRef.setAttribute("x1", String(svgX));
      lineRef.setAttribute("x2", String(svgX));
    }
    if (ptRef) {
      ptRef.setAttribute("cx", String(svgX));
      ptRef.setAttribute("cy", String(svgY));
    }
  };

  onMount(() => {
    if (primarySeries().points.length) {
      const lastIdx = primarySeries().points.length - 1;
      setActiveX(String(primarySeries().points[lastIdx]?.x ?? ""));
      setActiveValues(
        seriesList().map((s, idx) => ({
          name: s.name,
          val: s.points[lastIdx]?.y ?? 0,
          color: s.color ?? (idx === 0 ? "var(--accent-bright)" : "var(--cyan)"),
        }))
      );
    }
  });

  const handleExportCsv = () => {
    const s = seriesList();
    if (!s.length) return;
    let csv = "x," + s.map((item) => item.name).join(",") + "\n";
    const len = s[0].points.length;
    for (let i = 0; i < len; i++) {
      const row = [s[0].points[i]?.x ?? i, ...s.map((item) => item.points[i]?.y ?? "")];
      csv += row.join(",") + "\n";
    }
    void navigator.clipboard.writeText(csv);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  return (
    <div class="canvas-card chart-studio-wrap">
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-emerald">Telemetry & Benchmarks</span>
          <span class="card-subtitle">{props.data.title ?? "Performance Metrics"}</span>
        </div>
        <div class="card-actions">
          <button class="pill-action-btn" onClick={handleExportCsv}>
            {copied() ? "✓ Copied CSV" : "Export CSV"}
          </button>
        </div>
      </div>

      <Show when={props.data.kpis && props.data.kpis.length > 0}>
        <div class="metric-kpi-row">
          <For each={props.data.kpis}>
            {(kpi) => (
              <div class="metric-pod">
                <span class="pod-label">{kpi.label}</span>
                <span class="pod-value">{kpi.value}</span>
                <Show when={kpi.delta}>
                  <span
                    class="pod-badge"
                    style={{
                      color:
                        kpi.deltaType === "down"
                          ? "var(--rose-bright)"
                          : "var(--emerald-bright)",
                    }}
                  >
                    {kpi.delta}
                  </span>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>

      <div class="chart-interactive-stage">
        <Show when={activeValues().length > 0}>
          <div class="chart-active-bubble">
            <span>
              Point: <strong>{activeX()}</strong>
            </span>
            <For each={activeValues()}>
              {(v) => (
                <span>
                  {v.name}: <strong style={{ color: v.color }}>{v.val.toLocaleString()}</strong>
                </span>
              )}
            </For>
          </div>
        </Show>

        <div
          class="svg-chart-container"
          ref={containerRef}
          onMouseMove={handleMouseMove}
        >
          <svg viewBox="0 0 700 200" preserveAspectRatio="none">
            <defs>
              <linearGradient id="chartGradient1" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stop-color="#6366f1" stop-opacity="0.3" />
                <stop offset="100%" stop-color="#6366f1" stop-opacity="0.0" />
              </linearGradient>
            </defs>
            <line x1="0" y1="40" x2="700" y2="40" stroke="rgba(255,255,255,0.04)" />
            <line x1="0" y1="90" x2="700" y2="90" stroke="rgba(255,255,255,0.04)" />
            <line x1="0" y1="140" x2="700" y2="140" stroke="rgba(255,255,255,0.04)" />

            <Show when={primarySeries().points.length > 1}>
              <polygon
                points={getAreaPath(primarySeries().points)}
                fill="url(#chartGradient1)"
              />
            </Show>

            <For each={seriesList()}>
              {(s, idx) => (
                <path
                  d={getPath(s.points)}
                  fill="none"
                  stroke={s.color ?? (idx() === 0 ? "var(--accent-bright)" : "var(--cyan)")}
                  stroke-width={idx() === 0 ? "2.5" : "2"}
                  stroke-dasharray={idx() > 0 ? "4 3" : undefined}
                />
              )}
            </For>

            <line ref={lineRef} x1="350" y1="0" x2="350" y2="200" class="chart-crosshair-line" />
            <circle ref={ptRef} cx="350" cy="100" r="5" fill="#0c0f17" stroke="var(--accent-bright)" stroke-width="3" />
          </svg>
        </div>

        <Show when={props.data.x_labels && props.data.x_labels.length > 0}>
          <div style={{ display: "flex", "justify-content": "space-between", "font-size": "11px", color: "var(--text-dim)", "margin-top": "4px", "font-family": "var(--font-mono)" }}>
            <For each={props.data.x_labels}>{(lbl) => <span>{lbl}</span>}</For>
          </div>
        </Show>
      </div>
    </div>
  );
}
