import { render } from "solid-js/web";
import OfficeView from "../src/components/OfficeView";
import "../src/styles.css";

const marker = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==";
const units = [
  { anchor: "slide:256", kind: "slide", level: 0, text: "Slide 1: Audit slide", labels: [] },
  { anchor: "slide:256/shape:2", kind: "shape", level: 0, text: "Audit slide", labels: [] },
  { anchor: "slide:256/shape:3", kind: "image", level: 0, text: "Audit image", labels: ["image position: x 1 in from left, y 1 in from top, width 4 in, height 4 in"] },
  ...[["Category", "Series 1"], ["North", "12"], ["South", "18"]].map((row, index) => ({
    anchor: `slide:256/chart:chart1.xml/r${index + 1}`, kind: "table_row", level: 0, text: row.join(" | "), row_cells: row.map((text) => [["", text]]), labels: ["chart data; cached values", "chart title: Audit values", "chart type: column", "chart series type 0: column", "chart position: x 6 in from left, y 1 in from top, width 6 in, height 5 in"],
  })),
  { anchor: "slide:256/notes", kind: "notes", level: 0, text: "Speaker note", labels: ["speaker notes"] },
  { anchor: "slide:257", kind: "slide", level: 0, text: "Slide 2: Table slide", labels: [] },
  { anchor: "slide:257/shape:2", kind: "shape", level: 0, text: "Table slide", labels: [] },
  ...[["Region", "Value"], ["North", "12"]].map((row, index) => ({
    anchor: `slide:257/shape:3/tbl@1/r${index + 1}`, kind: "table_row", level: 0, text: row.join(" | "), row_cells: row.map((text) => [["", text]]), labels: ["table position: x 1 in from left, y 1 in from top, width 5 in, height 3 in"],
  })),
] as any[];
const projection: any = {
  path: "audit.pptx", sha256: "audit", vocabulary: "power_point", kind: "PowerPoint presentation", extension: "pptx", macro_enabled: false, strict: false,
  title: "Office preview audit", stats: [["slides", 2]], flags: [], sensitivity_labels: [], outline: [
    { anchor: "slide:256", title: "Slide 1: Audit slide", level: 1, first_unit: 0, units: 7 },
    { anchor: "slide:257", title: "Slide 2: Table slide", level: 1, first_unit: 7, units: 4 },
  ], total_units: units.length, from: 0, next: null, units,
  image_object_ids: { "slide:256/shape:3": "slide1.xml#3" },
  media: [{ object_id: "slide1.xml#3", alt_text: "Audit image", mime_type: "image/png", data_url: marker }], not_read: [],
};
const originalFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const url = String(input);
  if (url.startsWith("/fs/office?")) {
    const parsed = new URL(url, location.origin);
    const value = parsed.searchParams.get("view") === "facts" ? { ...projection, units: [], media: [], image_object_ids: {} } : projection;
    return new Response(JSON.stringify(value), { headers: { "Content-Type": "application/json" } });
  }
  return originalFetch(input, init);
};
render(() => <div style="height:100vh;width:100vw"><OfficeView source={{ path: "audit.pptx" }} fileName="audit.pptx" /></div>, document.getElementById("root")!);
const tick = () => new Promise((resolve) => setTimeout(resolve, 50));
(window as any).runChecks = async () => {
  await tick();
  const canvasSlide = document.querySelector<HTMLElement>(".office-deck-canvas .office-slide");
  if (!canvasSlide) throw new Error("Canvas omitted the slide frame");
  const canvasRatio = canvasSlide.getBoundingClientRect().width / canvasSlide.getBoundingClientRect().height;
  if (Math.abs(canvasRatio - 16 / 9) > 0.03) throw new Error(`Canvas slide ratio is ${canvasRatio}`);
  const present = Array.from(document.querySelectorAll("button")).find((button) => button.textContent === "Present");
  if (!present) throw new Error("Present action did not appear");
  present.click();
  await tick();
  await tick();
  if (!document.querySelector(".office-presentation .office-image img")) throw new Error("Presenter omitted the slide image");
  if (!document.querySelector(".office-presentation .office-chart-svg")) throw new Error("Presenter omitted the slide chart");
  if (!document.querySelector(".office-presentation .office-chart-svg rect.office-chart-series-0")) throw new Error("Presenter rendered an empty chart");
  const slide = document.querySelector<HTMLElement>(".office-presentation .office-slide");
  if (!slide) throw new Error("Presenter omitted the slide frame");
  const ratio = slide.getBoundingClientRect().width / slide.getBoundingClientRect().height;
  if (Math.abs(ratio - 16 / 9) > 0.03) throw new Error(`Presenter slide ratio is ${ratio}`);
  const next = Array.from(document.querySelectorAll(".office-presentation footer button")).find((button) => button.textContent === "Next");
  if (!next) throw new Error("Next slide action did not appear");
  next.click();
  await tick();
  if (!document.querySelector(".office-presentation .office-unit-table")) throw new Error("Presenter omitted the slide table");
  return "image, chart, table and 16:9 slide frame: passed";
};
