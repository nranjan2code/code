import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const rendererSource = await readFile(new URL("../components/PresentationRenderer.tsx", import.meta.url), "utf8");
const fixtureSource = await readFile(new URL("./fixtures.ts", import.meta.url), "utf8");
const cardToolsSource = await readFile(new URL("../../../vak-core/src/presentation_tools.rs", import.meta.url), "utf8");

function objectKeys(source, declaration) {
  const start = source.indexOf(declaration);
  assert.notEqual(start, -1, `Could not find ${declaration}`);
  const open = source.indexOf("{", start);
  let depth = 1;
  let end = open + 1;
  for (; end < source.length && depth > 0; end += 1) {
    if (source[end] === "{") depth += 1;
    if (source[end] === "}") depth -= 1;
  }
  assert.equal(depth, 0, `Unclosed object for ${declaration}`);
  return [...source.slice(open + 1, end - 1).matchAll(/(?:^|,)\s*(?:"([^"]+)"|([\w$]+))\s*:/gm)]
    .map((match) => match[1] ?? match[2]);
}

test("every registered semantic renderer has fixture coverage", () => {
  const registered = objectKeys(rendererSource, "const STRUCTURED_RENDERERS:");
  const fixtureCategories = objectKeys(fixtureSource, "export const TYPE_CATEGORY:");
  const missing = registered.filter((type) => !fixtureCategories.includes(type));
  assert.deepEqual(missing, [], `Add fixture routing for registered semantic types: ${missing.join(", ")}`);
});

test("every fixture route points at a defined fixture category", () => {
  const categoryStart = fixtureSource.indexOf("const CATEGORY_FIXTURES:");
  assert.notEqual(categoryStart, -1, "Could not find CATEGORY_FIXTURES");
  const categoryEnd = fixtureSource.indexOf("};", categoryStart);
  const definedCategories = new Set(
    [...fixtureSource.slice(categoryStart, categoryEnd).matchAll(/(?:^|,)\s*([\w$]+)\s*:/gm)].map((match) => match[1]),
  );
  const routesStart = fixtureSource.indexOf("export const TYPE_CATEGORY:");
  const routesEnd = fixtureSource.indexOf("};", routesStart);
  const routes = fixtureSource.slice(routesStart, routesEnd);
  const referencedCategories = [...routes.matchAll(/\["([\w$]+)"(?:,\s*"([\w$]+)")?\]/g)]
    .flatMap((match) => [match[1], match[2]].filter(Boolean));
  const missing = [...new Set(referencedCategories)].filter((category) => !definedCategories.has(category));
  assert.deepEqual(missing, [], `Define fixtures for categories: ${missing.join(", ")}`);
});

test("every Rust card-tool semantic type has exactly one UI renderer and fixture route", () => {
  const fromTools = [...cardToolsSource.matchAll(/semantic_types:\s*&\[((?:[^\]])*)\]/g)]
    .flatMap((match) => [...match[1].matchAll(/"([^"]+)"/g)].map((item) => item[1]));
  const fromRenderer = objectKeys(rendererSource, "const STRUCTURED_RENDERERS:");
  const fromFixtures = objectKeys(fixtureSource, "export const TYPE_CATEGORY:");
  const counts = (values) => values.reduce((map, value) => map.set(value, (map.get(value) ?? 0) + 1), new Map());
  const toolCounts = counts(fromTools);
  assert.deepEqual([...toolCounts].filter(([, count]) => count !== 1), [], "Each card type must belong to one tool shape");
  assert.deepEqual([...toolCounts.keys()].sort(), [...fromRenderer].sort(), "Tool and UI renderer types must match");
  assert.deepEqual([...toolCounts.keys()].sort(), [...fromFixtures].sort(), "Tool and fixture types must match");
});
