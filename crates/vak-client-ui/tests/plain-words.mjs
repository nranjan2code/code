// Everyday strings name things by what people recognise (DESIGN.md "Words",
// docs/design/75-visual-refresh.md §7). This fails on an engineering term in
// a user-facing string outside the technical views, so the plain-words pass
// cannot quietly regress.
import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const root = new URL("../src/", import.meta.url).pathname;
const BANNED = /\b(sandbox(?:ed)?|ledgers?|receipts?|frozen|candidates?|execution|provenance|manifest|verifier|persona|fleet|daemons?|MCP)\b/i;
// Technical views, where precise terms are the point.
const TECHNICAL = new Set(["components/ReceiptsModal.tsx"]);

const files = [];
const walk = (dir) => {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) { if (name !== "harness") walk(path); }
    else if (name.endsWith(".tsx")) files.push(path);
  }
};
walk(root);

const found = [];
for (const path of files) {
  const file = relative(root, path);
  if (TECHNICAL.has(file)) continue;
  const src = readFileSync(path, "utf8");
  const strings = [
    ...[...src.matchAll(/>([^<>{}\n]{3,})</g)].map((m) => m[1]),
    ...[...src.matchAll(/\b(?:title|aria-label|placeholder|label|alt)=\{?["`]([^"`]{3,})["`]/g)].map((m) => m[1]),
    // `hint:` holds search keywords, which are never shown, so it is not here.
    ...[...src.matchAll(/\b(?:label|title|text|help|description|summary|what|repair|detail|message|empty):\s*["`]([^"`]{3,})["`]/g)].map((m) => m[1]),
  ];
  for (const text of strings) if (/[a-z]{3}/i.test(text) && BANNED.test(text)) found.push(`${file}: ${text.trim().slice(0, 90)}`);
}
assert.deepEqual(found, [], `engineering terms in everyday strings:\n${found.join("\n")}`);
console.log(`plain-words: ok (${files.length} files)`);
