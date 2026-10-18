import test from "node:test";
import assert from "node:assert/strict";
import { acquireAnswerValuesScope } from "./answerValues.ts";

const boundPayload = { value: 1, bound: [{ path: ["value"], expression: "x + 1", from: ["source"] }] };

test("scope mounts share publications and settings while partial source removal respects other owners", () => {
  const first = acquireAnswerValuesScope("conversation:session:turn");
  const second = acquireAnswerValuesScope("conversation:session:turn");
  first.reconcile(["source"]);
  second.reconcile(["source", "other"]);
  first.scope.publish("source", { x: 4 });
  first.scope.setSettings("source", { x: "4." });

  assert.equal(second.scope.withBoundValues(boundPayload).value, 5);
  assert.deepEqual(second.scope.getSettings("source"), { x: "4." });

  first.reconcile([]);
  assert.equal(second.scope.withBoundValues(boundPayload).value, 5);
  assert.deepEqual(second.scope.getSettings("source"), { x: "4." });
  second.release();
  assert.equal(first.scope.withBoundValues(boundPayload).value, 1);
  first.release();
  first.scope.publish("source", { x: 99 });
  first.scope.setSettings("source", { x: "99" });
  const reopened = acquireAnswerValuesScope("conversation:session:turn");
  reopened.reconcile(["source"]);
  assert.equal(reopened.scope.withBoundValues(boundPayload).value, 1);
  assert.equal(reopened.scope.getSettings("source"), undefined);
  reopened.release();
});

test("keys isolate page versions and recorded values remain the fallback when absent", () => {
  const v1 = acquireAnswerValuesScope("page:artifact:version-1");
  const v2 = acquireAnswerValuesScope("page:artifact:version-2");
  v1.reconcile(["source"]);
  v2.reconcile(["source"]);
  v1.scope.publish("source", { x: 8 });
  assert.equal(v1.scope.withBoundValues(boundPayload).value, 9);
  assert.equal(v2.scope.withBoundValues(boundPayload).value, 1);
  v1.reconcile([]);
  assert.equal(v1.scope.withBoundValues(boundPayload).value, 1);
  v1.release();
  v2.release();
});

test("replacing a source snapshot removes names it no longer publishes", () => {
  const lease = acquireAnswerValuesScope("replace-snapshot");
  lease.reconcile(["source"]);
  lease.scope.publish("source", { x: 4, old: 12 });
  lease.scope.setSettings("source", { x: "4", removed: "stale" });
  lease.scope.publish("source", { x: 7 });
  lease.scope.setSettings("source", { x: "7" });
  assert.equal(lease.scope.withBoundValues(boundPayload).value, 8);
  assert.deepEqual(lease.scope.getSettings("source"), { x: "7" });
  lease.release();
});
