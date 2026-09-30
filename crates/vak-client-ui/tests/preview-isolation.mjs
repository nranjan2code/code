import assert from "node:assert/strict";
import { previewSandbox, sandboxedSrcdoc } from "../src/safeUrl.ts";

// The sandbox is a client-owned choice: only the two named kinds exist, and
// the opaque-origin one never carries allow-same-origin.
assert.equal(previewSandbox("static"), "allow-scripts allow-forms");
assert.ok(!previewSandbox("static").includes("allow-same-origin"));
assert.ok(previewSandbox("live_server").includes("allow-same-origin"));

const policyAt = (doc) => doc.indexOf("Content-Security-Policy");

// Every network path is closed; nothing a document contains can widen it.
const plain = sandboxedSrcdoc("<p>hi</p>");
assert.match(plain, /connect-src 'none'/);
assert.ok(!/https?:/.test(plain.slice(0, plain.indexOf("<p>"))));
assert.ok(policyAt(plain) < plain.indexOf("<p>"));

// The policy precedes a script that sits before `<head>`.
const early = sandboxedSrcdoc("<script>steal()</script><head><title>x</title></head>");
assert.ok(policyAt(early) < early.indexOf("steal()"));

// ...and one that only mentions `<head>` inside a string or comment.
const decoy = sandboxedSrcdoc("<!-- <head> --><script>var a = '<head>'; run()</script>");
assert.ok(policyAt(decoy) < decoy.indexOf("run()"));

// A leading doctype stays first, so the document keeps standards mode.
const typed = sandboxedSrcdoc("  <!DOCTYPE html><html><head></head><body></body></html>");
assert.ok(typed.startsWith("<!DOCTYPE html><meta"));
assert.equal(typed.match(/<!doctype/gi).length, 1);

console.log("preview isolation ok");

// A preview given a window of its own is a sandboxed frame inside a bare wrapper:
// a blob page takes the app's origin, so the page itself can never be the preview.
import { previewWindowDocument } from "../src/safeUrl.ts";
const popped = previewWindowDocument(sandboxedSrcdoc('<p title="a&b">x</p><script>run()</script>'));
assert.match(popped, /^<!doctype html><meta charset="utf-8"><style>[^<]*<\/style><iframe sandbox="allow-scripts allow-forms" srcdoc="/);
assert.ok(!popped.replace(/srcdoc="[^"]*"/, "").includes("<script"), "the wrapper carries no script of its own");
assert.ok(!popped.slice(popped.indexOf('srcdoc="') + 8, -"\"></iframe>".length).includes('"'), "the page cannot close the attribute");
assert.ok(popped.includes("&amp;") && popped.includes("&quot;"));
console.log("popout ok");
