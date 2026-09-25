import assert from "node:assert/strict";
import { displayFileName } from "../src/attachFiles.ts";

assert.equal(displayFileName("inbox/4e370266d52f-Q3 report final.docx"), "Q3 report final.docx");
assert.equal(displayFileName("inbox/notes.txt"), "notes.txt");
assert.equal(displayFileName("reports/4e370266d52f-kept.docx"), "4e370266d52f-kept.docx");
assert.equal(displayFileName("deck.pptx"), "deck.pptx");
console.log("attach-files: ok");
