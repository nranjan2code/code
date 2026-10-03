import assert from "node:assert/strict";
import { approvalNetworkNote, approvalSubjects } from "../src/approvalSubject.ts";

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
assert.ok(approvalNetworkNote("bash", "WorkspaceWrite", "seatbelt")?.includes("no internet access"));
assert.ok(approvalNetworkNote("bash", "ReadOnly", "landlock"));
assert.equal(approvalNetworkNote("bash", "FullAccess", "off"), null, "full access keeps the network");
assert.equal(approvalNetworkNote("bash", "WorkspaceWrite", "off"), null, "no sandbox, no claim");
assert.equal(approvalNetworkNote("bash", undefined, undefined), null, "unknown state, no claim");
assert.equal(approvalNetworkNote("read", "WorkspaceWrite", "seatbelt"), null, "only bash runs in the protected area");
console.log("approval-subject: ok");
