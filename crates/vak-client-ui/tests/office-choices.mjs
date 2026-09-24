import assert from "node:assert/strict";
import { keep, kept, leaveOut } from "../src/officeChoices.ts";

// 3 builds on 1, and 4 builds on 3.
const choices = [
  { id: "0", requires: [] },
  { id: "1", requires: [] },
  { id: "2", requires: [] },
  { id: "3", requires: ["1"] },
  { id: "4", requires: ["3"] },
];
const none = new Set();
const withoutOne = leaveOut(choices, none, "1");
assert.deepEqual([...withoutOne].sort(), ["1", "3", "4"], "leaving 1 out takes what builds on it");
assert.deepEqual(kept(choices, withoutOne), ["0", "2"]);
const backFour = keep(choices, withoutOne, "4");
assert.deepEqual([...backFour], [], "keeping 4 brings back what it builds on");
const withoutThree = leaveOut(choices, none, "3");
assert.deepEqual([...withoutThree].sort(), ["3", "4"]);
assert.deepEqual([...keep(choices, withoutThree, "3")], ["4"], "keeping 3 does not force 4");
assert.equal(none.size, 0, "inputs are not mutated");
console.log("office-choices: ok");
