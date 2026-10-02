import Sheet from "./Sheet";
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
  ["Esc twice", "stop the running turn"],
  ["Enter", "send / steer while running"],
  ["Shift+Enter", "newline"],
];

export default function ShortcutsModal() {
  return (
    <Sheet title="Keyboard shortcuts" onClose={() => setShowShortcuts(false)}>
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
    </Sheet>
  );
}
