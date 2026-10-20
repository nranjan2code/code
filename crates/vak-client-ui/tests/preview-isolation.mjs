import assert from "node:assert/strict";
import { previewSandbox, sandboxedSrcdoc } from "../src/safeUrl.ts";

// The sandbox is a client-owned choice: only the two named kinds exist, and
// the opaque-origin one never carries allow-same-origin.
assert.equal(previewSandbox("static"), "allow-forms");
assert.ok(!previewSandbox("static").includes("allow-same-origin"));
assert.ok(previewSandbox("origin").includes("allow-same-origin"));

const policyAt = (doc) => doc.indexOf("Content-Security-Policy");

// Every network path is closed; nothing a document contains can widen it.
const plain = sandboxedSrcdoc("<p>hi</p>");
assert.match(plain, /script-src 'none'/);
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
assert.match(popped, /^<!doctype html><meta charset="utf-8"><style>[^<]*<\/style><iframe sandbox="allow-forms" srcdoc="/);
assert.ok(!popped.replace(/srcdoc="[^"]*"/, "").includes("<script"), "the wrapper carries no script of its own");
assert.ok(!popped.slice(popped.indexOf('srcdoc="') + 8, -"\"></iframe>".length).includes('"'), "the page cannot close the attribute");
assert.ok(popped.includes("&amp;") && popped.includes("&quot;"));
console.log("popout ok");

// The theme is written as variables after the policy and before the markup,
// and holds nothing a page could use to reach out (doc 96, Q1).
import { withCurrentTheme } from "../src/safeUrl.ts";
const tokens = { "--bg": "#111", "--text": "#eee", "--muted": "#999", "--accent": "#f80", "--border": "#333", "--sans": '"Inter", sans-serif', "--mono": "monospace", "--radius": "12px" };
let scheme = "dark";
globalThis.document = { documentElement: {} };
globalThis.getComputedStyle = () => ({ colorScheme: scheme, getPropertyValue: (name) => tokens[name] ?? "" });
const themed = sandboxedSrcdoc("<p>hi</p>");
const themeAt = themed.indexOf("<style data-vak-theme>");
assert.ok(policyAt(themed) < themeAt && themeAt < themed.indexOf("<p>"));
assert.match(themed, /--vak-bg:#111;.*--vak-radius:12px;color-scheme:dark/);
// The inherited default has zero specificity, so authored preview styles win.
assert.match(themed, /:where\(html\)\{[^}]*font-family:var\(--vak-font\)[^}]*\}/);
assert.ok(!/<script|https?:|url\(/.test(themed.slice(themeAt, themed.indexOf("<p>"))));
tokens["--bg"] = "#fff</style><script>x()</script>";
assert.ok(!sandboxedSrcdoc("<p>hi</p>").includes("<script>"), "a token value cannot close the block");
tokens["--bg"] = "#fafafa";
scheme = "light";
const switched = withCurrentTheme(themed);
assert.match(switched, /--vak-bg:#fafafa;.*color-scheme:light/);
assert.equal(switched.replace(/<style data-vak-theme>[^<]*<\/style>/, ""), themed.replace(/<style data-vak-theme>[^<]*<\/style>/, ""));
const forged = sandboxedSrcdoc('<style data-vak-theme>:root{--vak-bg:red}</style>');
assert.equal(withCurrentTheme(forged).match(/--vak-bg:red/g).length, 1, "only the leading block is rewritten");
delete globalThis.document;
delete globalThis.getComputedStyle;
console.log("preview theme ok");

const { previewOriginWindowDocument } = await import('../src/safeUrl.ts');
assert.match(previewOriginWindowDocument('http://127.0.0.1:1234/token/index.html'), /<iframe sandbox="allow-forms" src="http:/);
