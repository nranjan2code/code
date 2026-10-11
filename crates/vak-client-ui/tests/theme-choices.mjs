// The theme list is one contract: every choice Settings offers is accepted
// when stored, resolves to a palette the stylesheet defines, and has a
// preview. Run: node tests/theme-choices.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { isThemeChoice, resolveTheme, themeChoices, themeIsLight } from "../src/theme.ts";

const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const ids = themeChoices.map((choice) => choice.id);

assert.deepEqual(ids, ["system", "light", "dark", "contrast", "frosted-light", "frosted-dark"]);
for (const id of ids) assert.ok(isThemeChoice(id), id);
for (const retired of ["midnight", "sage", "paper", "", undefined, 3]) assert.ok(!isThemeChoice(retired));

assert.equal(resolveTheme("system", true), "dark");
assert.equal(resolveTheme("system", false), "light");
assert.equal(resolveTheme("frosted-dark", false), "frosted-dark");
assert.ok(themeIsLight("light") && themeIsLight("frosted-light"));
assert.ok(!themeIsLight("dark") && !themeIsLight("frosted-dark") && !themeIsLight("contrast"));

// Each palette is a block that sets the glass tokens, so a theme without
// glass says so itself instead of inheriting another theme's.
const block = (id) => {
  const open = id === "light" ? css.indexOf(":root {") : css.indexOf(`html[data-theme="${id}"] {`);
  assert.ok(open >= 0, `no theme block for ${id}`);
  return css.slice(open, css.indexOf("\n}", open));
};
for (const id of ids.filter((id) => id !== "system")) {
  const body = block(id);
  for (const token of ["--surface-alpha", "--surface-blur", "--surface-filter", "--backdrop"]) {
    assert.ok(body.includes(`${token}:`), `${id} does not set ${token}`);
  }
  const glass = id.startsWith("frosted-");
  assert.equal(/--surface-filter: none;/.test(body), !glass, `${id} glass`);
  assert.equal(/--surface-alpha: 100%;/.test(body), !glass, `${id} alpha`);
  // The plain preview is the light palette; every other theme draws its own.
  assert.ok(css.includes(id === "light" ? ".theme-preview {" : `.theme-preview.${id} `), `no preview for ${id}`);
}
assert.ok(css.includes(".theme-preview.system "));

// Glass has an opaque form where it cannot be drawn or is not wanted.
for (const guard of ["@media (prefers-reduced-transparency: reduce)", "@supports not ((backdrop-filter: blur(1px))"]) {
  const at = css.indexOf(guard);
  assert.ok(at >= 0, guard);
  const rule = css.slice(at, css.indexOf("\n}", at));
  assert.ok(rule.includes('html[data-theme^="frosted-"]') && rule.includes("--surface-alpha: 100%") && rule.includes("--surface-filter: none"), guard);
}

console.log("theme choices: ok");
