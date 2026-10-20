import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { test } from "node:test";

const src = new URL("../src/", import.meta.url);
const sources = await Promise.all((await readdir(src))
  .filter(name => /\.(css|tsx|ts)$/.test(name))
  .map(async name => [name, await readFile(new URL(name, src), "utf8")]));
const css = sources.find(([name]) => name === "styles.css")[1];

test("every admin token reference belongs to the theme contract", () => {
  const defined = new Set([...css.matchAll(/(--[\w-]+)\s*:/g)].map(m => m[1]));
  const missing = [];
  for (const [name, text] of sources) {
    for (const match of text.matchAll(/var\(\s*(--[\w-]+)/g)) {
      if (!defined.has(match[1])) missing.push(`${name}: ${match[1]}`);
    }
  }
  assert.deepEqual(missing, [], "Undefined tokens silently discard color, shape and font declarations");
});

test("admin typography keeps the twelve pixel floor, including SVG charts", () => {
  const small = [];
  for (const [name, text] of sources) {
    for (const match of text.matchAll(/font-size\s*[:=]\s*["']?(\d+(?:\.\d+)?)(px|["'])/g)) {
      if (Number(match[1]) < 12) small.push(`${name}: ${match[0]}`);
    }
  }
  assert.deepEqual(small, []);
});

test("admin component class names resolve to styles or documented structural hooks", () => {
  const styled = new Set([...css.matchAll(/\.([a-z][\w-]*)/g)].map(m => m[1]));
  // Structural hooks inherit their appearance from a companion class or tag.
  const hooks = new Set([
    "primary", "settings-button", "select-agent-filter", "receipts-table",
    "home-questions", "home-now", "home-pulse", "home-money", "home-data",
    "home-history", "prompts-section", "automation-new",
  ]);
  const missing = new Set();
  for (const [file, source] of sources.filter(([file]) => file.endsWith(".tsx"))) {
    for (const match of source.matchAll(/class="([^"]+)"/g)) {
      for (const name of match[1].split(/\s+/)) {
        if (!styled.has(name) && !hooks.has(name)) missing.add(`${file}: ${name}`);
      }
    }
  }
  assert.deepEqual([...missing], [], "A renamed class must move with its stylesheet");
});

// Solid's directive declaration makes a side-effect-only import typecheck,
// but the compiled JSX needs an actual runtime binding when the dialog opens.
test("focus-trap directives have their runtime binding", () => {
  for (const [file, source] of sources) {
    if (source.includes("use:trapFocus")) {
      assert.match(source, /import\s*\{[^}]*\btrapFocus\b[^}]*\}\s*from\s*["']\.\/focusTrap["']/, file);
    }
  }
});

test("conditional badges evaluate their condition rather than a truthy function", () => {
  for (const [file, source] of sources.filter(([file]) => file.endsWith(".tsx"))) {
    assert.doesNotMatch(source, /<Show\s+when=\{\(\)\s*=>/, file);
  }
});
