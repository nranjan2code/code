import test from "node:test";
import assert from "node:assert/strict";
import { compareDecimalValues, sortTableRows } from "./tableNumbers.ts";

test("decimal comparison preserves precision beyond JavaScript safe integers", () => {
  assert.equal(compareDecimalValues("9007199254740992", "9007199254740993"), -1);
  assert.equal(compareDecimalValues("123456789012345678901234567890", "123456789012345678901234567889"), 1);
});

test("decimal comparison handles signs, equivalent zero padding, and exponents", () => {
  assert.equal(compareDecimalValues("-2", "-1.99"), -1);
  assert.equal(compareDecimalValues("1.20", "12e-1"), 0);
  assert.equal(compareDecimalValues(".001", "1e-3"), 0);
  assert.equal(compareDecimalValues("1e100000000000000000000", "9e99999999999999999999"), 1);
  assert.equal(compareDecimalValues("-0.0", "0e999999999999999999999"), 0);
  assert.equal(compareDecimalValues("0", "0.01"), -1);
  assert.equal(compareDecimalValues("-0", "-0.01"), 1);
});

test("signed zeros sort between negative and positive subunit decimals", () => {
  const rows = [["1"], [".01"], ["0"], ["-.01"], ["-1"], ["-0"]];
  assert.deepEqual(sortTableRows(rows, 0, true, true), [rows[4], rows[3], rows[2], rows[5], rows[1], rows[0]]);
  assert.deepEqual(sortTableRows(rows, 0, true, false), [rows[0], rows[1], rows[2], rows[5], rows[3], rows[4]]);
  assert.equal(compareDecimalValues("-0e-999999999999999999999", "0e999999999999999999999"), 0);
});

test("numeric rows preserve source strings and place invalid values last stably", () => {
  const rows = [["9007199254740993"], ["bad"], ["9007199254740992"], [null], ["9007199254740992"]];
  assert.deepEqual(sortTableRows(rows, 0, true, true), [rows[2], rows[4], rows[0], rows[1], rows[3]]);
  assert.deepEqual(sortTableRows(rows, 0, true, false), [rows[0], rows[2], rows[4], rows[1], rows[3]]);
  assert.equal(sortTableRows(rows, 0, true, true)[0][0], "9007199254740992");
});

test("legacy finite numbers sort with exact string representations", () => {
  const rows = [[10], [2], [Number.MAX_SAFE_INTEGER], ["9007199254740992"]];
  assert.deepEqual(sortTableRows(rows, 0, true, true), [rows[1], rows[0], rows[2], rows[3]]);
});

test("text columns use lexicographic ordering, not numeric guesses", () => {
  const rows = [["item10"], ["item2"], ["Item2"], ["item2"]];
  assert.deepEqual(sortTableRows(rows, 0, false, true), [rows[2], rows[0], rows[1], rows[3]]);
});
