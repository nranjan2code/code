import assert from "node:assert/strict";
import { cellAddress, columnName, parseCell } from "../src/officeCells.ts";

assert.deepEqual(parseCell("150"), { shown: "150", formula: null, stale: false, notCalculated: false });
assert.deepEqual(parseCell("=SUM(B2:B4) [cached: 1950, stale until recalculated]"), { shown: "1950", formula: "=SUM(B2:B4)", stale: true, notCalculated: false });
assert.deepEqual(parseCell("=B3*2 [cached: 140]"), { shown: "140", formula: "=B3*2", stale: false, notCalculated: false });
assert.deepEqual(parseCell("=B3*2 [not calculated yet]"), { shown: "", formula: "=B3*2", stale: false, notCalculated: true });
assert.deepEqual(parseCell("(shared formula) [cached: 7]"), { shown: "7", formula: "(shared formula)", stale: false, notCalculated: false });
assert.equal(parseCell("[cached: looks like one] but is text").formula, null, "text that merely resembles a marker stays text");
assert.deepEqual(cellAddress("B12"), { column: 2, row: 12 });
assert.deepEqual(cellAddress("AA3"), { column: 27, row: 3 });
assert.equal(cellAddress("B"), null);
assert.equal(columnName(1), "A");
assert.equal(columnName(26), "Z");
assert.equal(columnName(27), "AA");
assert.equal(columnName(703), "AAA");
console.log("office-cells: ok");
