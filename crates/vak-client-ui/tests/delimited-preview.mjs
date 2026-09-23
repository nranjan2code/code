import assert from "node:assert/strict";
import { parseDelimitedPreview } from "../src/delimitedPreview.ts";

const csv = parseDelimitedPreview('name,note\r\nAda,"hello, ""Vak"""\r\nLin,"two\nlines"\r\n', ",");
assert.deepEqual(csv.headers, ["name", "note"]);
assert.deepEqual(csv.rows, [["Ada", 'hello, "Vak"'], ["Lin", "two\nlines"]]);
assert.equal(csv.totalRows, 2);

const tsv = parseDelimitedPreview("name\tvalue\nA\t1\nB\t2\n", "\t", 1);
assert.deepEqual(tsv.rows, [["A", "1"]]);
assert.equal(tsv.totalRows, 2);
assert.equal(tsv.truncated, true);

assert.throws(() => parseDelimitedPreview("name,value\nA\n", ","), /header has 2/);
assert.throws(() => parseDelimitedPreview('name,value\nA,"unfinished', ","), /quoted value/);
