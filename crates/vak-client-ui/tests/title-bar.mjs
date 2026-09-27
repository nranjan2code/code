import assert from "node:assert/strict";
import { onTitleBar, titleBarGestures } from "../src/titleBar.ts";

// A press's event path, target first, as `composedPath()` gives it.
const el = (tagName, attributes = {}) => ({
  tagName,
  getAttribute: (name) => (name in attributes ? attributes[name] : null),
});
const header = el("HEADER", { "data-titlebar": "" });
const title = el("DIV", { class: "workspace-title" });
const doc = { nodeType: 9 };

// Empty space and plain text anywhere inside a region are the title bar.
assert.equal(onTitleBar([header, doc]), true);
assert.equal(onTitleBar([el("SPAN"), title, header, doc]), true);
// What a person operates keeps its own clicks, however deep it sits.
assert.equal(onTitleBar([el("BUTTON"), title, header]), false);
assert.equal(onTitleBar([el("SPAN"), el("H1"), el("BUTTON"), header]), false);
assert.equal(onTitleBar([el("SUMMARY"), el("DETAILS"), header]), false);
assert.equal(onTitleBar([el("A", { href: "#" }), header]), false);
assert.equal(onTitleBar([el("INPUT"), header]), false);
assert.equal(onTitleBar([el("DIV", { role: "menu" }), el("DETAILS"), header]), false);
assert.equal(onTitleBar([el("DIV", { tabindex: "0" }), header]), false);
assert.equal(onTitleBar([el("DIV", { contenteditable: "true" }), header]), false);
// A focusable-by-script element is not something a person operates.
assert.equal(onTitleBar([el("DIV", { tabindex: "-1" }), header]), true);
// An opted-out subtree (a menu without a role) is not the title bar.
assert.equal(onTitleBar([el("DIV"), el("DIV", { "data-titlebar": "false" }), header]), false);
// Nothing outside a region is the title bar.
assert.equal(onTitleBar([el("DIV"), el("MAIN"), doc]), false);
assert.equal(onTitleBar([]), false);

// Gestures: one press drags; a double-click acts on its second release,
// only where it went down.
function fakeDocument() {
  const listeners = {};
  return {
    listeners,
    addEventListener: (type, fn) => { listeners[type] = fn; },
    removeEventListener: (type, fn) => { if (listeners[type] === fn) delete listeners[type]; },
  };
}
const press = (detail, at = { x: 10, y: 10 }, path = [title, header]) => {
  let prevented = false;
  return {
    button: 0, detail, clientX: at.x, clientY: at.y,
    composedPath: () => path,
    preventDefault: () => { prevented = true; },
    get prevented() { return prevented; },
  };
};
const calls = [];
const fake = fakeDocument();
const uninstall = titleBarGestures({
  dragWindow: () => calls.push("drag"),
  titleBarDoubleClick: () => calls.push("double"),
}, fake);

const first = press(1);
fake.listeners.mousedown(first);
fake.listeners.mouseup(press(1));
assert.equal(first.prevented, true, "a title bar selects no text and takes no focus");
fake.listeners.mousedown(press(2));
fake.listeners.mouseup(press(2));
assert.deepEqual(calls, ["drag", "double"]);

// A second press that moved before release does not zoom.
calls.length = 0;
fake.listeners.mousedown(press(2, { x: 10, y: 10 }));
fake.listeners.mouseup(press(2, { x: 40, y: 10 }));
assert.deepEqual(calls, []);

// A double-click on a button inside the title bar is the button's.
const onButton = [el("BUTTON"), title, header];
const buttonPress = press(1, undefined, onButton);
fake.listeners.mousedown(buttonPress);
fake.listeners.mousedown(press(2, undefined, onButton));
fake.listeners.mouseup(press(2, undefined, onButton));
assert.deepEqual(calls, []);
assert.equal(buttonPress.prevented, false);

// Other buttons are left alone.
fake.listeners.mousedown({ ...press(1), button: 2 });
assert.deepEqual(calls, []);

uninstall();
assert.deepEqual(Object.keys(fake.listeners), []);
console.log("title-bar: ok");
