import { trapFocus } from "../focusTrap";
import { setShowShortcuts } from "../store";

const isMac = typeof navigator !== "undefined" && /mac/i.test(navigator.platform || navigator.userAgent);
const mod = isMac ? "⌘" : "Ctrl+";

const shortcuts: [string, string][] = [
  [`${mod}N`, "new session"],
  [`${mod}D`, "toggle diff pane"],
  [`${mod}\\`, "toggle split view (two tasks side-by-side)"],
  [`${mod}B`, "toggle sidebar"],
  [`${mod}H`, "time travel (workspace snapshots)"],
  [`${mod}K`, "global search across sessions"],
  ["G then I", "open inbox"],
  [`${mod},`, "open settings"],
  ["⌃`", "toggle terminal"],
  [`${mod};`, "toggle side chat (ask aside)"],
  ["@", "mention a file in the composer"],
  ["/", "pick a skill or command in the composer"],
  ["↑ / ↓", "navigate prompt history (empty composer)"],
  [`${mod}/`, "this help"],
  ["Esc", "stop the running turn"],
  ["Enter", "send / steer while running"],
  ["Shift+Enter", "newline"],
];

export default function ShortcutsModal() {
  return (
    <div class="modal-back" onClick={() => setShowShortcuts(false)}>
      <div class="modal" role="dialog" aria-modal="true" aria-labelledby="shortcuts-title" onClick={(e) => e.stopPropagation()} use:trapFocus>
        <h3 id="shortcuts-title">Keyboard shortcuts</h3>
        <table>
          <tbody>
            {shortcuts.map(([k, d]) => (
              <tr>
                <td>
                  <kbd>{k}</kbd>
                </td>
                <td>{d}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <button class="btn primary" onClick={() => setShowShortcuts(false)}>
          Close
        </button>
      </div>
    </div>
  );
}

