import type { AdaptiveRenderNode } from "./types";

const object = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string" && value.trim().length > 0;
const list = (value: unknown): value is unknown[] => Array.isArray(value);
const namedItems = (value: unknown, fields: string[]): value is Record<string, unknown>[] =>
  list(value) && value.length > 0 && value.every((item) => object(item) && fields.every((key) => text(item[key])));
const oneOf = (value: unknown, values: string[]): value is string => typeof value === "string" && values.includes(value);

/**
 * Check persisted adaptive trees against the forms the client actually draws.
 * It is intentionally structural: it never guesses from prose, titles, or
 * row counts. A malformed tree falls back to its original answer text.
 */
export function validateAdaptiveTree(root: unknown): root is AdaptiveRenderNode {
  if (!validNode(root)) return false;
  return true;
}

function validNode(value: unknown): value is AdaptiveRenderNode {
  if (!object(value) || typeof value.primitive !== "string" || !object(value.props) || !list(value.children)) return false;
  const { primitive, props, children } = value;
  if (!children.every(validNode)) return false;
  const meaningfulChild = children.length > 0;
  switch (primitive) {
    case "stack":
    case "row":
    case "group":
      return meaningfulChild;
    case "section":
      return meaningfulChild || ["title", "label", "text"].some((key) => text(props[key]));
    case "text":
      return text(props.text);
    case "metric":
      return text(props.label) && (text(props.value) || (typeof props.value === "number" && Number.isFinite(props.value)));
    case "summary":
      return text(props.headline);
    case "steps":
      return namedItems(props.steps, ["label"]);
    case "list":
      return namedItems(props.items, ["label"]);
    case "details":
      return namedItems(props.fields, ["label", "detail"]);
    case "table":
      return validTable(props);
    case "comparison":
      return namedItems(props.options, ["name"]) && props.options.length >= 2;
    case "stats":
      return list(props.readings) && props.readings.length > 0
        && props.readings.every((reading) => object(reading) && text(reading.label)
          && reading.value !== undefined && reading.value !== null
          && (text(reading.value) || typeof reading.value === "number" || typeof reading.value === "boolean"));
    case "chart":
      return validChart(props);
    case "callout":
      return oneOf(props.tone, ["tip", "note", "warning", "important"]) && text(props.text);
    case "sources":
      return namedItems(props.sources, ["title", "url", "finding"]);
    case "calendar":
      return namedItems(props.events, ["title", "date"]);
    case "map":
      return oneOf(props.connect, ["none", "route", "area"])
        && list(props.places) && props.places.length > 0
        && props.places.every((place) => object(place) && text(place.label)
          && typeof place.latitude === "number" && Number.isFinite(place.latitude) && Math.abs(place.latitude) <= 90
          && typeof place.longitude === "number" && Number.isFinite(place.longitude) && Math.abs(place.longitude) <= 180);
    case "controls":
      // User-entered values can be blank or half typed. Validate the card's
      // structure, but leave value and expression evaluation to the controls
      // runtime so a partial setting remains visible and recoverable.
      return list(props.inputs) && list(props.outputs)
        && props.inputs.every((input) => object(input) && text(input.name) && text(input.label)
          && oneOf(input.kind, ["number", "range", "choice", "toggle", "date"]))
        && props.outputs.every((output) => object(output) && text(output.name) && typeof output.label === "string");
    case "link":
      return text(props.url);
    case "media":
      return oneOf(props.format, ["image", "video", "audio"]) && text(props.url);
    case "file_preview":
      return text(props.artifact_path);
    case "markup_preview":
      return text(props.html) && list(props.libraries) && props.libraries.every((library) => typeof library === "string");
    case "diff":
      return list(props.files) && props.files.length > 0
        && props.files.every((file) => object(file) && text(file.filename) && typeof file.hunks === "string"
          && typeof file.additions === "number" && Number.isInteger(file.additions) && file.additions >= 0
          && typeof file.deletions === "number" && Number.isInteger(file.deletions) && file.deletions >= 0);
    case "test_report":
      return text(props.command) && list(props.tests) && props.tests.length > 0
        && props.tests.every((test) => object(test) && text(test.name)
          && oneOf(test.status, ["passed", "failed", "skipped"]));
    case "terminal":
      return text(props.command) && typeof props.output === "string";
    default:
      return false;
  }
}

function validTable(props: Record<string, unknown>): boolean {
  const columns = props.columns;
  const rows = props.rows;
  return list(columns) && columns.length > 0
    && columns.every((column) => object(column) && text(column.label))
    && list(rows) && rows.length > 0
    && rows.every((row) => Array.isArray(row) && row.length === columns.length);
}

function validChart(props: Record<string, unknown>): boolean {
  if (!oneOf(props.chart_type, ["line", "bar", "area"]) || !list(props.series) || props.series.length === 0) return false;
  let hasMeasuredValue = false;
  for (const series of props.series) {
    if (!object(series) || !text(series.name) || !list(series.points)) return false;
    const follows = text(series.follows);
    if (series.points.length === 0 && !follows) return false;
    for (const point of series.points) {
      if (!object(point) || !(text(point.x) || (typeof point.x === "number" && Number.isFinite(point.x)))) return false;
      if (point.y === null) continue;
      if (typeof point.y !== "number" || !Number.isFinite(point.y)) return false;
      hasMeasuredValue = true;
    }
  }
  return hasMeasuredValue || props.series.some((series) => object(series) && text(series.follows));
}
