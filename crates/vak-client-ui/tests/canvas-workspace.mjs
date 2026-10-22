import assert from "node:assert/strict";
import { canvasViewport } from "../src/canvasDrawing.ts";
import { validCheckpoints } from "../src/canvasWorkspace.ts";
import { openEntry, fromStored, toStored, updateEntry, diffCanvas, applyOp } from "../src/canvasStack.ts";
import { rebaseCanvas } from "../src/canvasRebase.ts";
import { subjectKey } from "../src/canvasSubject.ts";
import { freshEntry } from "../src/canvasViewers.ts";
const fresh = s => freshEntry(s, false);
const file = path => ({ kind: "file", title: path, path, sessionId: "s" });
const checkpoint = text => ({ "extension/editor": { schema: 1, summary: text, data: { text } } });
const key = subjectKey(file("work.txt"));
let stacks = openEntry({}, "s", file("work.txt"), fresh);
const baseline = toStored(stacks.s);
stacks = updateEntry(stacks, "s", key, { checkpoints: checkpoint("mine") });
const local = stacks.s;
const pending = [{ op: "update", key, patch: { checkpoints: checkpoint("mine") } }];
const remote = toStored(updateEntry({s:fromStored(baseline, undefined, fresh)}, "s", key, {checkpoints:checkpoint("theirs")}).s);
assert.deepEqual(rebaseCanvas(local, remote, baseline, pending, fresh), {conflict:true});
assert.equal(rebaseCanvas(local, remote, baseline, pending, fresh, {[key]:"mine"}).canvas.entries[0].checkpoints["extension/editor"].data.text, "mine");
assert.equal(rebaseCanvas(local, remote, baseline, pending, fresh, {[key]:"saved"}).canvas.entries[0].checkpoints["extension/editor"].data.text, "theirs");
assert.equal(rebaseCanvas(local, {entries:[], active:null}, baseline, pending, fresh).canvas.entries[0].checkpoints["extension/editor"].data.text, "mine");
assert.deepEqual(fromStored(JSON.parse(JSON.stringify(toStored(local))), undefined, fresh).entries[0].checkpoints, checkpoint("mine"));
for(let n=1;n<8;n++) { stacks=openEntry(stacks,"s",file(`${n}.txt`),fresh); stacks=updateEntry(stacks,"s",stacks.s.active,{checkpoints:checkpoint(`${n}`)}); }
assert.equal(openEntry(stacks,"s",file("overflow.txt"),fresh),stacks,"domain edits protect eviction without a discussion note");
assert.equal(validCheckpoints(checkpoint("ok")),true);
assert.equal(validCheckpoints({bad:{schema:1,summary:"bad",data:Infinity}}),false);
assert.equal(validCheckpoints({bad:{schema:1,summary:"bad",data:()=>{}}}),false);
assert.throws(()=>fromStored({entries:[...baseline.entries,{key:"unknown",subject:{kind:"unknown"}}],active:key},undefined,fresh),/saved Canvas is invalid/);
for(const ratio of [1,1.25,1.5,2,3]) { const v=canvasViewport(390,844,ratio); assert.equal(v.pixelWidth,Math.round(390*ratio)); assert.equal(v.pixelHeight,Math.round(844*ratio)); }
console.log("Canvas checkpoints, conflicts, current-contract refusal and display scaling passed");

const restored = rebaseCanvas(local, {entries:[], active:null}, baseline, pending, fresh).canvas;
let replayed = {};
for (const op of diffCanvas(null, restored)) replayed = applyOp(replayed, "s", op, fresh);
assert.deepEqual(toStored(replayed.s), toStored(restored), "reopened-tab resolution replays pending edits as well as subject identity");

// Independent discussion and domain edits on the same tab merge without
// carrying a stale value from the other field across surfaces.
{
  const base = fromStored(baseline, undefined, fresh);
  const noteRemote = updateEntry({s:base}, "s", key, {draft:"Remote discussion"}).s;
  const result = rebaseCanvas(local, toStored(noteRemote), baseline, pending, fresh);
  assert.equal(result.canvas.entries[0].draft, "Remote discussion");
  assert.deepEqual(result.canvas.entries[0].checkpoints, checkpoint("mine"));
  const noteLocal = updateEntry({s:base}, "s", key, {draft:"Local discussion"}).s;
  const domainRemote = updateEntry({s:base}, "s", key, {checkpoints:checkpoint("remote domain")}).s;
  const reverse = rebaseCanvas(noteLocal, toStored(domainRemote), baseline, [
    {op:"update",key,patch:{draft:"Local discussion"}},
  ], fresh);
  assert.equal(reverse.canvas.entries[0].draft, "Local discussion");
  assert.deepEqual(reverse.canvas.entries[0].checkpoints, checkpoint("remote domain"));
}

// Renderer state is copied at the boundary, and an old renderer's async work
// cannot mutate a tab after its editing lease ends.
{
  const { createWorkspaceEdits, isWorkspaceValue } = await import("../src/canvasWorkspace.ts");
  let saved;
  let writes = 0;
  const edits = createWorkspaceEdits(() => saved, next => { saved = next; writes++; });
  const input = {schema:1,summary:"Pending",data:{vertices:[1,2,3]}};
  edits.checkpoint("scene/edit", input);
  input.data.vertices[0] = 99;
  assert.equal(saved["scene/edit"].data.vertices[0], 1);
  const read = edits.read("scene/edit");
  read.data.vertices[1] = 99;
  assert.equal(saved["scene/edit"].data.vertices[1], 2);
  edits.dispose();
  assert.equal(edits.signal.aborted, true);
  edits.clear("scene/edit");
  edits.checkpoint("scene/edit", input);
  assert.equal(writes, 1);
  assert.equal(edits.read("scene/edit"), undefined);
  const cyclic = {}; cyclic.self = cyclic;
  assert.equal(isWorkspaceValue(cyclic), false);
  let deep = null; for(let i=0;i<70;i++) deep = {child:deep};
  assert.equal(isWorkspaceValue(deep), false);
  assert.equal(isWorkspaceValue(new Float32Array([1,2,3])), false);
  assert.equal(isWorkspaceValue({ get value() { throw new Error("must not run"); } }), false);
  assert.equal(isWorkspaceValue([,1]), false);
  assert.equal(isWorkspaceValue({a:{value:1},b:{value:1}}), true);
  assert.equal(validCheckpoints({entry:{schema:1,summary:"Missing data"}}), false);
}
