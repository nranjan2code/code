import assert from "node:assert/strict";
import { niceTicks } from "../src/components/cards/chartScale.ts";

// The live chart that showed 0, 44, 88 for three percentages: its axis is
// round numbers that reach past the top value.
assert.deepEqual(niceTicks(0, 88), [0, 25, 50, 75, 100]);
assert.deepEqual(niceTicks(0, 3), [0, 1, 2, 3]);
const mixed = niceTicks(-12, 30);
assert.ok(mixed[0] <= -12 && mixed.at(-1) >= 30 && mixed.includes(0), String(mixed));
const big = niceTicks(0, 22776.1);
assert.equal(big[0], 0);
assert.ok(big.at(-1) >= 22776.1 && big.length <= 6, String(big));
console.log("chart scale ok");
