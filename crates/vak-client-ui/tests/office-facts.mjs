import assert from "node:assert/strict";
import { officeFactsLine, officeFlagsLabel } from "../src/officeFacts.ts";

assert.equal(
  officeFactsLine({ vocabulary: "power_point", kind: "PowerPoint presentation", stats: [["slides", 3], ["hidden slides", 0], ["slides with notes", 1]], flags: [] }),
  "PowerPoint presentation · 3 slides",
);
assert.equal(
  officeFactsLine({ vocabulary: "word", kind: "Word document", stats: [["paragraphs", 40], ["words", 1240], ["tracked changes", 2], ["comments", 1]], flags: [] }),
  "Word document · 1,240 words · 2 tracked changes · 1 comment",
);
assert.equal(
  officeFactsLine({ vocabulary: "excel", kind: "Excel workbook", stats: [["sheets", 1], ["formulas", 0]], flags: [] }),
  "Excel workbook · 1 sheet",
);
assert.equal(
  officeFactsLine({ vocabulary: "power_point", kind: "PowerPoint presentation", stats: [["hidden slides", 1], ["slides", 1]], flags: [] }),
  "PowerPoint presentation · 1 slide · 1 hidden slide",
);
assert.equal(officeFlagsLabel({ vocabulary: "word", kind: "", stats: [], flags: [] }), null);
assert.equal(officeFlagsLabel({ vocabulary: "word", kind: "", stats: [], flags: ["macros"] }), "1 flag");
console.log("office-facts: ok");
