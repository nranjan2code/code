// The title bar the page draws itself, where the window controls overlay it
// (the macOS desktop shell, docs/design/75-visual-refresh.md §6). The system
// never sees a press there, so the page gives it the system title bar's two
// gestures: pressing and dragging moves the window, and a double-click does
// what the system's settings say (zoom, minimise or nothing).
//
// A title-bar region is an element marked `data-titlebar` and everything
// inside it, except what a person operates (a button, a link, a field, a
// menu) and anything under `data-titlebar="false"`, which keep their clicks.

/** The part of an element this needs, so tests can pass plain objects. */
interface Target {
  tagName?: string;
  getAttribute?(name: string): string | null;
}

const OPERABLE_TAGS = new Set(["A", "BUTTON", "INPUT", "SELECT", "TEXTAREA", "LABEL", "SUMMARY", "OPTION", "AUDIO", "VIDEO"]);
const OPERABLE_ROLES = new Set([
  "button", "link", "menu", "menubar", "menuitem", "tab", "checkbox", "radio",
  "switch", "option", "listbox", "combobox", "slider", "textbox", "dialog",
]);

function operable(target: Target, attribute: (name: string) => string | null): boolean {
  if (target.tagName && OPERABLE_TAGS.has(target.tagName.toUpperCase())) return true;
  const editable = attribute("contenteditable");
  if (editable !== null && editable !== "false") return true;
  const tabindex = attribute("tabindex");
  if (tabindex !== null && tabindex !== "-1") return true;
  const role = attribute("role");
  return role !== null && OPERABLE_ROLES.has(role);
}

/** Whether a press whose event path is `path` (target first) lands on a
 *  title-bar region rather than on something inside it a person operates. */
export function onTitleBar(path: readonly unknown[]): boolean {
  for (const node of path) {
    const target = node as Target;
    if (typeof target?.getAttribute !== "function") continue;
    const attribute = (name: string) => target.getAttribute!(name);
    const region = attribute("data-titlebar");
    if (region === "false" || operable(target, attribute)) return false;
    if (region !== null) return true;
  }
  return false;
}

export interface TitleBarActions {
  dragWindow?(): void;
  titleBarDoubleClick?(): void;
}

/** Listen for title-bar presses on `doc`. Returns the uninstaller. */
export function titleBarGestures(actions: TitleBarActions, doc: Document = document): () => void {
  // Where the second press of a double-click went down. It acts on release,
  // and only if the pointer stayed put, so a double-click that turns into a
  // drag does not also zoom.
  let second: { x: number; y: number } | null = null;
  const down = (event: MouseEvent) => {
    second = null;
    if (event.button !== 0 || !onTitleBar(event.composedPath())) return;
    // A title bar neither selects text nor takes focus from the message box.
    event.preventDefault();
    if (event.detail === 1) actions.dragWindow?.();
    else if (event.detail === 2) second = { x: event.clientX, y: event.clientY };
  };
  const up = (event: MouseEvent) => {
    const pressed = second;
    second = null;
    if (!pressed || event.button !== 0 || event.detail !== 2) return;
    if (event.clientX !== pressed.x || event.clientY !== pressed.y) return;
    if (onTitleBar(event.composedPath())) actions.titleBarDoubleClick?.();
  };
  doc.addEventListener("mousedown", down);
  doc.addEventListener("mouseup", up);
  return () => {
    doc.removeEventListener("mousedown", down);
    doc.removeEventListener("mouseup", up);
  };
}
