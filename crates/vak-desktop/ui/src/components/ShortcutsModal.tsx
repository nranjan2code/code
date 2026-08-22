import { setShowShortcuts } from "../store";

const SHORTCUTS: [string, string][] = [
  ["⌘N", "new session"],
  ["⌘D", "toggle diff pane"],
  ["⌃`", "toggle terminal"],
  ["⌘;", "toggle side chat (ask aside)"],
  ["⌘/", "this help"],
  ["Esc", "stop the running turn"],
  ["Enter", "send / steer while running"],
  ["Shift+Enter", "newline"],
];

export default function ShortcutsModal() {
  return (
    <div class="modal-back" onClick={() => setShowShortcuts(false)}>
      <div class="modal" onClick={(e) => e.stopPropagation()}>
        <h3>Keyboard shortcuts</h3>
        <table>
          <tbody>
            {SHORTCUTS.map(([k, d]) => (
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
