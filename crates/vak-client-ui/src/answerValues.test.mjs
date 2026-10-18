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

test("failed and replaced sources restore recorded consumers across multiple viewers", () => {
  const answer = acquireAnswerValuesScope("lifecycle:two-viewers");
  const panel = acquireAnswerValuesScope("lifecycle:two-viewers");
  answer.reconcile(["source", "rate"]);
  panel.reconcile(["source", "rate"]);
  const consumer = { value: 17, bound: [{ path: ["value"], expression: "x * rate", from: ["source", "rate"] }] };
  answer.scope.publish("source", { x: 4 });
  panel.scope.publish("rate", { rate: 3 });
  assert.equal(answer.scope.withBoundValues(consumer).value, 12);
  panel.scope.clearPublication("rate");
  assert.equal(answer.scope.withBoundValues(consumer).value, 17);
  panel.scope.publish("rate", { rate: 5 });
  assert.equal(answer.scope.withBoundValues(consumer).value, 20);

  answer.reconcile(["replacement", "rate"]);
  answer.scope.publish("replacement", { x: 99 });
  assert.equal(answer.scope.withBoundValues(consumer).value, 20, "the other viewer still owns the original source");
  panel.reconcile(["replacement", "rate"]);
  assert.equal(answer.scope.withBoundValues(consumer).value, 17, "a replacement never silently retargets an existing binding");
  panel.scope.publish("source", { x: 100 });
  assert.equal(answer.scope.withBoundValues(consumer).value, 17, "retired source callbacks cannot revive its value");
  answer.release();
  panel.release();
});

test("series consumers lose stale points on source failure and reconnect cleanly", () => {
  const first = acquireAnswerValuesScope("lifecycle:series-reconnect");
  first.reconcile(["source"]);
  const recorded = { points: [{ x: 0, y: 9 }], bound: [{ path: ["points"], series: "trend", from: ["source"] }] };
  first.scope.publish("source", { trend: [[1, 2], [3, 4]] });
  assert.deepEqual(first.scope.withBoundValues(recorded).points, [{ x: 1, y: 2 }, { x: 3, y: 4 }]);
  first.scope.clearPublication("source");
  assert.deepEqual(first.scope.withBoundValues(recorded).points, [{ x: 0, y: 9 }]);
  first.scope.publish("source", { trend: [[5, 6]] });
  first.release();
  first.release();
  const reconnect = acquireAnswerValuesScope("lifecycle:series-reconnect");
  reconnect.reconcile(["source"]);
  assert.deepEqual(reconnect.scope.withBoundValues(recorded).points, [{ x: 0, y: 9 }]);
  reconnect.scope.publish("source", { trend: [[7, 8]] });
  assert.deepEqual(reconnect.scope.withBoundValues(recorded).points, [{ x: 7, y: 8 }]);
  reconnect.release();
  assert.deepEqual(recorded.points, [{ x: 0, y: 9 }], "publications never mutate the saved answer");
});
