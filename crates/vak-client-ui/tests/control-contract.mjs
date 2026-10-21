import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const client=readFileSync(new URL('../src/styles.css',import.meta.url),'utf8');
const admin=readFileSync(new URL('../../vak-admin-ui/src/styles.css',import.meta.url),'utf8');
for(const name of ['height','compact','prominent','radius']) {
  const pattern=new RegExp(`--control-${name}:\\s*([^;]+);`);
  assert.equal(client.match(pattern)?.[1],admin.match(pattern)?.[1],`Client and Admin must agree on control ${name}`);
}
console.log('shared control geometry contract ok');
