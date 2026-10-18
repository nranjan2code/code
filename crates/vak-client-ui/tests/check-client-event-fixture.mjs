import { readFile } from "node:fs/promises";

const fixture = new URL("./fixtures/client-events.json", import.meta.url);
const contract = new URL("./client-event-contract.ts", import.meta.url);
const events = JSON.parse(await readFile(fixture, "utf8"));
const expected = `import type { ClientEvent } from "../src/types";\n\n// Generated from fixtures/client-events.json; check it with\n// check-client-event-fixture.mjs after changing the Rust wire fixture.\nexport const wireExamples = ${JSON.stringify(events, null, 2)} as const satisfies readonly ClientEvent[];\n\n`;
const actual = await readFile(contract, "utf8");
if (process.argv.includes("--write")) {
  await (await import("node:fs/promises")).writeFile(contract, expected);
  process.exit(0);
}
if (actual !== expected) {
  throw new Error("client-event-contract.ts is stale; regenerate its wireExamples from fixtures/client-events.json");
}
