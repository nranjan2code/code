import assert from "node:assert/strict";
import { validateAdaptiveTree } from "../src/adaptiveValidation.ts";

const node = (primitive, props = {}, children = []) => ({ primitive, props, children });
const valid = {
  stack: node("stack", {}, [node("text", { text: "Child" })]),
  row: node("row", {}, [node("text", { text: "Child" })]),
  group: node("group", {}, [node("text", { text: "Child" })]),
  section: node("section", { title: "Title" }),
  text: node("text", { text: "Visible" }),
  metric: node("metric", { label: "Latency", value: 12 }),
  summary: node("summary", { headline: "Answer" }),
  steps: node("steps", { steps: [{ label: "Step" }] }),
  list: node("list", { items: [{ label: "Item" }] }),
  details: node("details", { fields: [{ label: "Fact", detail: "Value" }] }),
  table: node("table", { columns: [{ label: "Name" }], rows: [["A"]] }),
  comparison: node("comparison", { options: [{ name: "A" }, { name: "B" }] }),
  stats: node("stats", { readings: [{ label: "Count", value: 0 }] }),
  chart: node("chart", { chart_type: "bar", series: [{ name: "Count", points: [{ x: "A", y: 1 }] }] }),
  callout: node("callout", { tone: "note", text: "Note" }),
  sources: node("sources", { sources: [{ title: "Source", url: "https://example.test", finding: "Finding" }] }),
  calendar: node("calendar", { events: [{ title: "Event", date: "2026-10-18" }] }),
  map: node("map", { connect: "none", places: [{ label: "Place", latitude: 12.5, longitude: 45.25 }] }),
  controls: node("controls", { inputs: [{ name: "amount", kind: "number", label: "Amount" }], outputs: [{ name: "total", label: "Total" }] }),
  link: node("link", { url: "https://example.test" }),
  media: node("media", { format: "image", url: "image.png" }),
  file_preview: node("file_preview", { artifact_path: "index.html" }),
  markup_preview: node("markup_preview", { html: "<p>Preview</p>", libraries: [] }),
  diff: node("diff", { files: [{ filename: "main.rs", hunks: "@@ -1 +1 @@", additions: 1, deletions: 0 }] }),
  test_report: node("test_report", { command: "cargo test", tests: [{ name: "unit", status: "passed" }] }),
  terminal: node("terminal", { command: "pwd", output: "/tmp" }),
};

for (const [primitive, tree] of Object.entries(valid)) {
  assert.equal(validateAdaptiveTree(tree), true, `${primitive} should validate`);
}

assert.equal(validateAdaptiveTree(node("chart", { chart_type: "bar", series: [] })), false);
assert.equal(validateAdaptiveTree(node("chart", { chart_type: "pie", series: [{ name: "x", points: [{ x: "A", y: 1 }] }] })), false);
assert.equal(validateAdaptiveTree(node("chart", { chart_type: "bar", series: [{ name: "x", points: [{ x: "A", y: null }] }] })), false);
assert.equal(validateAdaptiveTree(node("mystery_form", { text: "has text but no renderer" })), false);
assert.equal(validateAdaptiveTree(node("details", { fields: [] })), false);
assert.equal(validateAdaptiveTree({ primitive: "text", props: null, children: [] }), false);
assert.equal(validateAdaptiveTree(node("stack", {}, [node("unknown", {})])), false);

// Controls keep incomplete user-entered settings visible so the person can
// finish typing; value parsing and expression errors belong to the control.
assert.equal(validateAdaptiveTree(node("controls", {
  inputs: [{ name: "amount", kind: "number", label: "Amount", value: "-" }],
  outputs: [{ name: "total", label: "Total", expression: "amount * 2" }],
})), true);

// A dynamically followed chart series may not have points until its controls
// publisher is ready, but an empty ordinary chart cannot claim coverage.
assert.equal(validateAdaptiveTree(node("chart", {
  chart_type: "line",
  series: [{ name: "total", follows: "total", points: [] }],
})), true);

console.log("adaptive validation ok");
