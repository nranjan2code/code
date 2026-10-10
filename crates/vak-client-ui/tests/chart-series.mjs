import assert from "node:assert/strict";
import { chartGeometry, normalizeSeries } from "../src/components/presentation/data.ts";

// A chart as the host's pack compiler sends it (seed.timeseries, found on a
// deployed host 2026-10-10): the series sit in the chart node's props, with
// category labels for x. Read from child nodes, it showed "No data points".
const props = {
  chart_type: "line",
  series: [
    { name: "NIFTY 50", points: [{ x: "5 Oct", y: 22555.75 }, { x: "6 Oct", y: 22776.1 }, { x: "9 Oct", y: 22520.45 }] },
    { name: "NIFTY BANK", points: [{ x: "5 Oct", y: 54714.1 }, { x: "6 Oct", y: "n/a" }] },
  ],
};
const series = normalizeSeries(props.series);
assert.equal(series.length, 2);
assert.deepEqual(series[0].points[1], { x: "6 Oct", y: 22776.1 });
assert.equal(series[1].points[1].y, null, "a non-number y is a gap, not a point");
const geometry = chartGeometry({ chart_type: "line", series, accessible_summary: "" });
assert.deepEqual(geometry.keys, ["5 Oct", "6 Oct", "9 Oct"]);
assert.equal(geometry.valueAt(series[1], "5 Oct"), 54714.1);
assert.deepEqual(normalizeSeries(undefined), []);
console.log("chart series ok");
