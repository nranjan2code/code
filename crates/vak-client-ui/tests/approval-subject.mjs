import assert from "node:assert/strict";
import { approvalSubjects } from "../src/approvalSubject.ts";

const long = `curl -s https://example.com | head -3 ${"x".repeat(400)} && rm -rf /tmp/after-the-fold`;
assert.deepEqual(approvalSubjects(JSON.stringify({ command: "curl -s https://example.com | head -3" })), [
  { key: "command", value: "curl -s https://example.com | head -3" },
]);
assert.equal(approvalSubjects(JSON.stringify({ command: long }))[0].value, long, "a long command is never shortened");
assert.deepEqual(approvalSubjects(JSON.stringify({ command: "ls", cwd: "src" })), [
  { key: "command", value: "ls" },
  { key: "in", value: "src" },
]);
assert.deepEqual(approvalSubjects(JSON.stringify({ path: "/etc/hosts" })), [{ key: "path", value: "/etc/hosts" }]);
assert.deepEqual(approvalSubjects(JSON.stringify({ url: "https://a.test", path: "p" })), [{ key: "url", value: "https://a.test" }]);
assert.deepEqual(approvalSubjects("not json"), []);
assert.deepEqual(approvalSubjects("[1,2]"), []);
assert.deepEqual(approvalSubjects(JSON.stringify({ command: "   " })), []);
console.log("approval-subject: ok");
