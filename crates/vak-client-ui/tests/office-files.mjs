import assert from "node:assert/strict";
import { isAnchor, parseOfficeCitation } from "../src/officeFiles.ts";

for (const anchor of ["p:0A1B2C3D", "p@12", "Budget!B4", "'Q4 plan'!A5:B5", "Budget!", "slide:256", "slide:256/shape:3", "slide:256/placeholder:title", "slide:256/notes", "page:0/shape:5", "p@2/comment:0", "p:0A1B2C3D/comment:12", "tbl@1", "tbl@1/r2"]) {
  assert.ok(isAnchor(anchor), anchor);
}
for (const anchor of ["", "p:XYZ", "p:123456789", "slide:x", "slide:1/other", "page:1/notes", "B4", "Budget!B4:C5:D6", "Budget!4B", "'Q4'plan'!A1", "a\u0001!A1", "p@2/comment:", "slide:1/comment:2", "tbl@", "tbl@1/2", "tbl@1/rx"]) {
  assert.ok(!isAnchor(anchor), anchor);
}
assert.deepEqual(parseOfficeCitation("inbox/4e37-Q3 report.docx#p:0A1B2C3D"), { path: "inbox/4e37-Q3 report.docx", anchor: "p:0A1B2C3D" });
assert.deepEqual(parseOfficeCitation(" budget.xlsx#'Q4 plan'!B4 "), { path: "budget.xlsx", anchor: "'Q4 plan'!B4" });
assert.equal(parseOfficeCitation("notes.md#intro"), null);
assert.equal(parseOfficeCitation("deck.pptx"), null);
assert.equal(parseOfficeCitation("deck.pptx#nothing"), null);
console.log("office-files: ok");
