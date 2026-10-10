export function csvCell(value: unknown): string {
  const text = value == null ? "" : String(value);
  const safe = typeof value === "string" && /^[=+@\-\t\r]/.test(text) ? `'${text}` : text;
  return `"${safe.replace(/"/g, '""')}"`;
}

export function downloadCsv(filename: string, rows: unknown[][]): void {
  const content = rows.map((row) => row.map(csvCell).join(",")).join("\r\n");
  const url = URL.createObjectURL(new Blob([content], { type: "text/csv;charset=utf-8" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export interface ChartPoint { x: string | number; y: number | null }
export interface ChartSeries { name: string; points: ChartPoint[] }
export interface ChartData {
  chart_type: "line" | "area" | "bar";
  title?: string;
  x_label?: string;
  y_label?: string;
  series: ChartSeries[];
  accessible_summary: string;
}

export function chartGeometry(data: ChartData) {
  const keys = [...new Set(data.series.flatMap((series) => series.points.map((point) => point.x)))];
  const numeric = keys.every((key) => typeof key === "number" && Number.isFinite(key));
  const dates = !numeric && keys.length > 0 && keys.every((key) => typeof key === "string" && /^\d{4}-\d{2}-\d{2}(T|$)/.test(key) && Number.isFinite(Date.parse(key)));
  const raw = (key: string | number) => numeric ? Number(key) : dates ? Date.parse(String(key)) : keys.indexOf(key);
  if (numeric || dates) keys.sort((a, b) => raw(a) - raw(b));
  const xs = keys.map(raw);
  const values = data.series.flatMap((series) => series.points.flatMap((p) => p.y === null ? [] : [p.y]));
  const minX = Math.min(...xs, Infinity);
  const maxX = Math.max(...xs, -Infinity);
  let minY = values.length ? Math.min(...values) : 0;
  let maxY = values.length ? Math.max(...values) : 1;
  if (data.chart_type !== "line") { minY = Math.min(0, minY); maxY = Math.max(0, maxY); }
  if (minY === maxY) { minY -= 1; maxY += 1; }
  const x = (key: string | number) => maxX === minX ? 340 : 54 + ((raw(key) - minX) / (maxX - minX)) * 572;
  const y = (value: number) => 224 - ((value - minY) / (maxY - minY)) * 196;
  const valueAt = (series: ChartSeries, key: string | number) => series.points.find((point) => point.x === key)?.y ?? null;
  const segments = (series: ChartSeries): ChartPoint[][] => {
    const result: ChartPoint[][] = [];
    let run: ChartPoint[] = [];
    for (const key of keys) {
      const value = valueAt(series, key);
      if (value === null) { if (run.length) result.push(run); run = []; }
      else run.push({ x: key, y: value });
    }
    if (run.length) result.push(run);
    return result;
  };
  return { keys, minY, maxY, x, y, valueAt, segments };
}

/** A payload's `series`, whatever shape it came in, as chart series. */
export function normalizeSeries(raw: unknown): ChartSeries[] {
  const rawSeries: any[] = Array.isArray(raw) ? raw : [];
  return rawSeries
    .filter((s: any) => s && typeof s === "object")
    .map((s: any) => {
      const rawPoints: any[] = Array.isArray(s.points) ? s.points : [];
      const points: ChartPoint[] = rawPoints
        .map((pt: any): ChartPoint | null => {
          if (Array.isArray(pt) && pt.length >= 2) {
            return { x: typeof pt[0] === "number" ? pt[0] : String(pt[0] ?? ""), y: typeof pt[1] === "number" ? pt[1] : null };
          }
          if (pt && typeof pt === "object") {
            const x = pt.x ?? pt.label ?? pt.key;
            if (x === undefined || x === null) return null;
            return { x: typeof x === "number" ? x : String(x), y: typeof pt.y === "number" ? pt.y : typeof pt.value === "number" ? pt.value : null };
          }
          return null;
        })
        .filter((pt): pt is ChartPoint => pt !== null);
      return { name: String(s.name ?? s.label ?? "Series"), points };
    });
}
