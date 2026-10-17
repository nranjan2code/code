import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { daysOf, evaluateExpression, parseExpression } from "../src/controlsExpression.ts";

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

console.log("controls-expression ok");
