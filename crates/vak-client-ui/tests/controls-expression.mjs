import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { daysOf, evaluateExpression, parseExpression, seriesOf } from "../src/controlsExpression.ts";

// The host answers the same table (vak-delivery expression.rs).
const cases = JSON.parse(readFileSync(new URL("../../vak-delivery/tests/expression_cases.json", import.meta.url), "utf8"));
assert.ok(cases.length > 0);
for (const item of cases) {
  let got;
  try {
    got = evaluateExpression(parseExpression(item.expr), item.values ?? {});
  } catch {
    got = undefined;
  }
  if (item.value === undefined) {
    assert.equal(got, undefined, `${item.expr} should have no value, got ${got}`);
  } else {
    assert.ok(got !== undefined && Math.abs(got - item.value) < 1e-9, `${item.expr}: ${got}, wanted ${item.value}`);
  }
}

assert.equal(daysOf("1970-01-01"), 0);
assert.equal(daysOf("2026-10-17") - daysOf("2026-10-10"), 7);
assert.equal(daysOf("2026-13-01"), undefined);

// A series varies one input, as the host's controls::series does.
const grown = seriesOf([{ name: "grown", expression: "start * (1 + rate / 100) ^ year", vary: "year", from: "0", to: "3", step: "1" }],
  { start: 100, rate: 10, year: 0, grown: 100 }).grown;
assert.equal(grown.length, 4);
assert.ok(Math.abs(grown[3][1] - 133.1) < 1e-9);

console.log("controls-expression ok");
