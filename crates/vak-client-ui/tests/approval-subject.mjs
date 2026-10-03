import assert from "node:assert/strict";
import { approvalNetworkNote, approvalSubjects, approvalTitle, mcpServerName } from "../src/approvalSubject.ts";

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
const W = "/Users/me/vak-home";
assert.deepEqual(approvalSubjects(JSON.stringify({ command: "ls" }), W), [
  { key: "command", value: "ls" },
  { key: "in", value: W },
], "no folder named: it runs in the workspace, and the card says where");
assert.deepEqual(approvalSubjects(JSON.stringify({ command: "ls", cwd: "." }), W)[1], { key: "in", value: W });
assert.deepEqual(approvalSubjects(JSON.stringify({ command: "ls", cwd: "./sub/dir" }), `${W}/`)[1], { key: "in", value: `${W}/sub/dir` });
assert.deepEqual(approvalSubjects(JSON.stringify({ command: "ls", cwd: ".." }), W)[1], { key: "in", value: `${W}/..` }, "a climbing folder is shown as written, never resolved away");
assert.equal(approvalSubjects(JSON.stringify({ command: "ls" })).length, 1, "the workspace unknown: no claim about the default");
const call = JSON.stringify({ action: "call", server: "files", tool: "delete_all", arguments: { path: "/tmp/x", force: true } });
assert.equal(approvalTitle("mcp", call), "delete_all from files", "the server's tool is named, not the mcp door");
assert.deepEqual(approvalSubjects(call), [
  { key: "server", value: "files" },
  { key: "tool", value: "delete_all" },
  { key: "with", value: '{"path":"/tmp/x","force":true}' },
]);
assert.deepEqual(approvalSubjects(JSON.stringify({ action: "call", server: "s", tool: "t" })).map((r) => r.key), ["server", "tool"]);
assert.equal(approvalTitle("mcp", JSON.stringify({ action: "list" })), "mcp", "a list is not a call");
assert.equal(approvalTitle("bash", JSON.stringify({ command: "ls" })), "bash");
assert.equal(approvalTitle("mcp", "not json"), "mcp");
assert.equal(mcpServerName(call), "files");
assert.equal(mcpServerName(JSON.stringify({ action: "list", server: "files" })), null);
assert.equal(mcpServerName(JSON.stringify({ command: "ls" })), null);
assert.equal(mcpServerName("not json"), null);
assert.ok(approvalNetworkNote("mcp", "WorkspaceWrite", "seatbelt", false)?.includes("server runs in a protected area"));
assert.equal(approvalNetworkNote("mcp", "WorkspaceWrite", "seatbelt", true), null, "a server allowed out says nothing");
assert.equal(approvalNetworkNote("mcp", "WorkspaceWrite", "seatbelt", undefined), null, "an unknown server says nothing");
assert.equal(approvalNetworkNote("mcp", "FullAccess", "off", false), null);
assert.equal(approvalNetworkNote("mcp", "WorkspaceWrite", "off", false), null);
console.log("approval-subject: ok");
