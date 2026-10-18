import { render } from "solid-js/web";
import { AnswerValuesContext, acquireAnswerValuesScope } from "../src/answerValues";
import { FormView, CardIdContext } from "../src/components/cards";
import "../src/styles.css";

document.documentElement.dataset.theme = new URLSearchParams(location.search).get("theme") === "dark" ? "dark" : "light";
const exact = {
  title: "Exact values",
  columns: [{ label: "ID", numeric: false }, { label: "Amount", numeric: true }],
  rows: [["00123", "9007199254740993"], ["1,5", "9007199254740992"], ["00007", "0.0100"]],
};
const controls = {
  title: "Split the bill",
  inputs: [{ name: "bill", label: "Bill", kind: "number", value: 1200 }, { name: "people", label: "People", kind: "number", value: 3 }],
  outputs: [{ name: "each", label: "Each pays", expression: "bill / people", equals: 400 }],
};
const linked = {
  title: "Linked result",
  columns: [{ label: "Item", numeric: false }, { label: "Amount", numeric: true }],
  rows: [["Each pays", 400]],
  bound: [{ path: ["rows", 0, 1], expression: "bill / people", from: ["fixture-controls"] }],
};
const values = acquireAnswerValuesScope("fixture:table-fidelity");
values.reconcile(["fixture-controls"]);
window.addEventListener("beforeunload", () => values.release());
render(() => <AnswerValuesContext.Provider value={values.scope}><main style="max-width:960px;margin:0 auto;padding:16px;display:grid;gap:24px">
  <h1>Table fidelity</h1>
  <section id="exact"><FormView block="table" payload={exact} fallback="Exact values unavailable" /></section>
  <section id="controls"><CardIdContext.Provider value="fixture-controls"><FormView block="controls" payload={controls} fallback="Controls unavailable" /></CardIdContext.Provider></section>
  <section id="linked"><FormView block="table" payload={linked} fallback="Linked values unavailable" /></section>
</main></AnswerValuesContext.Provider>, document.getElementById("root")!);

const tick = () => new Promise((resolve) => setTimeout(resolve, 30));
(window as any).runTableChecks = async () => {
  const passed: string[] = [];
  const check = (value: unknown, message: string) => { if (!value) throw new Error(message); passed.push(message); };
  const grid = () => Array.from(document.querySelectorAll("#exact tbody tr"), row => Array.from(row.querySelectorAll("td"), cell => cell.textContent));
  await tick();
  check(JSON.stringify(grid()) === JSON.stringify(exact.rows), "Display preserves IDs, comma text, large integers and decimal spelling");
  const headers = document.querySelectorAll<HTMLButtonElement>("#exact th button");
  headers[1].click(); await tick();
  check(JSON.stringify(grid().map(row => row[0])) === JSON.stringify(["00007", "1,5", "00123"]), "Ascending numeric sort distinguishes adjacent large integers");
  headers[1].click(); await tick();
  check(JSON.stringify(grid().map(row => row[0])) === JSON.stringify(["00123", "1,5", "00007"]), "Descending numeric sort preserves exact values");
  headers[0].click(); await tick();
  check(JSON.stringify(grid().map(row => row[0])) === JSON.stringify(["00007", "00123", "1,5"]), "Text sorting keeps lexical IDs");
  let captured: Blob | undefined;
  const create = URL.createObjectURL;
  const click = HTMLAnchorElement.prototype.click;
  try {
    URL.createObjectURL = blob => { captured = blob as Blob; return create(blob); };
    HTMLAnchorElement.prototype.click = function () { if (!this.download) click.call(this); };
    document.querySelector<HTMLButtonElement>("#exact .card-actions button")!.click();
  } finally {
    URL.createObjectURL = create;
    HTMLAnchorElement.prototype.click = click;
  }
  const csv = await captured?.text();
  check(csv?.includes('"00123","9007199254740993"') && csv?.includes('"1,5","9007199254740992"') && csv?.includes('"00007","0.0100"'), "CSV preserves exact strings without a real download");
  const people = document.querySelector<HTMLInputElement>('#controls input[name="people"]') ?? document.querySelectorAll<HTMLInputElement>("#controls input")[1];
  people.value = "4"; people.dispatchEvent(new Event("input", { bubbles: true })); await tick();
  check(document.querySelector("#linked tbody td:last-child")?.textContent === "300", "Controls update the linked table through production bindings");
  check(document.documentElement.scrollWidth <= innerWidth, "The page fits the viewport without horizontal overflow");
  return passed;
};
