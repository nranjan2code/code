import { render } from "solid-js/web";
import { createSignal, Show } from "solid-js";
import ControlsCard from "../src/components/cards/Controls";
import { FormView } from "../src/components/cards";
import AnswerValuesScope, { pageValuesKey } from "../src/components/AnswerValuesScope";
import { CardIdContext } from "../src/components/cards/shared";
import type { OutputItem } from "../src/types";
import "../src/styles.css";

const root = document.getElementById("root")!;
const payload = {
  title: "Shared control",
  inputs: [{ name: "amount", label: "Amount", kind: "number", value: 10 }],
  outputs: [{ name: "double", label: "Double", expression: "amount * 2" }],
};
const source = { id: "source", provenance: { presentation_id: "source" } } as OutputItem;
const boundStats = {
  readings: [{ label: "Double", value: 5 }],
  bound: [{ path: ["readings", 0, "value"], expression: "double", from: ["source"] }],
};
const [second, setSecond] = createSignal(false);
const [version, setVersion] = createSignal("version-1");
const [show, setShow] = createSignal(true);
const view = () => <AnswerValuesScope scopeKey={pageValuesKey(version())} items={[source]}>
  <CardIdContext.Provider value="source"><ControlsCard payload={payload} /></CardIdContext.Provider>
  <Show when={second()}><CardIdContext.Provider value="source"><ControlsCard payload={payload} /></CardIdContext.Provider></Show>
  <section id="linked"><FormView block="stats" payload={boundStats} fallback="Recorded value: 5" /></section>
</AnswerValuesScope>;
const dispose = render(() => <Show when={show()}>{view()}</Show>, root);

const assert = (condition: unknown, message: string) => { if (!condition) throw new Error(message); };
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
(window as any).runChecks = async () => {
  const inputs = () => Array.from(root.querySelectorAll<HTMLInputElement>('input[type="number"]'));
  assert(inputs().length === 1, "The first Controls mount rendered");
  inputs()[0].value = "25";
  inputs()[0].dispatchEvent(new Event("input", { bubbles: true }));
  await tick();
  setSecond(true);
  await tick();
  assert(inputs().length === 2 && inputs()[1].value === "25", "A second Controls mount adopts current settings");

  const statsValue = () => Number(root.querySelector("#linked .stat-value")?.textContent?.trim());
  assert(statsValue() === 50, "The mounted FormView follows shared publications");
  inputs()[0].value = "";
  inputs()[0].dispatchEvent(new Event("input", { bubbles: true }));
  await tick();
  assert(statsValue() === 5, "Invalid inputs clear publication and restore the recorded fallback");
  inputs()[0].value = "4";
  inputs()[0].dispatchEvent(new Event("input", { bubbles: true }));
  await tick();
  assert(statsValue() === 8, "Valid input republishes and recomputes the bound value");

  setVersion("version-2");
  await tick();
  assert(inputs().every((input) => input.value === "10"), "Changing immutable page version starts with independent settings");
  setShow(false);
  await tick();
  setShow(true);
  await tick();
  assert(inputs().every((input) => input.value === "10"), "Reopening after the final unmount starts clean");
  return ["shared settings", "live bound values", "recorded fallback", "valid recovery", "version isolation", "final cleanup"]
    .map((name) => `PASS: ${name}`);
};
window.addEventListener("beforeunload", dispose);
