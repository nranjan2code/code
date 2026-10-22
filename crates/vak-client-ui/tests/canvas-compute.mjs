import assert from "node:assert/strict";
import { evaluateWorkspaceModule, validateWorkspaceModule } from "../src/canvasCompute.ts";
const unsigned=value=>{const out=[];do{const next=value&127;value>>>=7;out.push(next|(value?128:0));}while(value);return out;};
const section=(id,bytes)=>[id,...unsigned(bytes.length),...bytes];
const name=value=>[...unsigned(value.length),...new TextEncoder().encode(value)];
// Real ABI module: one bounded memory; alloc(i32)->i32; evaluate(i32,i32)->i64.
// It returns JSON from its data segment, with no imports or host capabilities.
const module=(memory=[1,1,1,1],extra=[])=>new Uint8Array([
  0,97,115,109,1,0,0,0,
  ...section(1,[2,0x60,1,0x7f,1,0x7f,0x60,2,0x7f,0x7f,1,0x7e]),
  ...extra,
  ...section(3,[2,0,1]),...section(5,memory),
  ...section(7,[3,...name("memory"),2,0,...name("alloc"),0,0,...name("evaluate"),0,1]),
  ...section(10,[2,5,0,0x41,0x80,0x08,0x0b,4,0,0x42,11,0x0b]),
  ...section(11,[1,0,0x41,0,0x0b,11,...new TextEncoder().encode('{"value":7}')]),
]);
const bytes=module();
assert.doesNotThrow(()=>validateWorkspaceModule(bytes));
assert.deepEqual(await evaluateWorkspaceModule(bytes,{state:{},arguments:{value:3}}),{value:7});
assert.throws(()=>validateWorkspaceModule(module([1,0,1])),/memory/);
assert.throws(()=>validateWorkspaceModule(module([1,1,1,...unsigned(1025)])),/memory/);
assert.throws(()=>validateWorkspaceModule(module([1,3,1,1])),/memory/);
assert.throws(()=>validateWorkspaceModule(module(undefined,section(2,[1,...name("env"),...name("fetch"),0,0]))),/cannot import/);
assert.throws(()=>validateWorkspaceModule(bytes.slice(0,-1)),/Truncated/);
assert.throws(()=>validateWorkspaceModule(new Uint8Array([1,2,3])),/Invalid/);
console.log("Real WebAssembly ABI, no host imports, bounded memory/tables and malformed-module refusals passed");
