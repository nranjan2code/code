import { setShowShortcuts } from "../store";

const SHORTCUTS: [string, string][] = [
  ["⌘N", "new session"],
  ["⌘\\", "toggle split view (two tasks side-by-side)"],
  ["⌘B", "toggle sidebar"],
  ["⌘H", "time travel (workspace snapshots)"],
  ["⌘K", "global search across sessions"],
  ["G then I", "open inbox"],
  ["⌘,", "open settings"],
  ["⌘;", "toggle side chat (ask aside)"],
  ["@", "mention a file in the composer"],
  ["/", "pick a skill in the composer"],
  ["⌘/", "this help"],
  ["Esc", "stop the running turn"],
  ["Enter", "send / steer while running"],
  ["Shift+Enter", "newline"],
];

export default function ShortcutsModal() {
  return (
    <div class="modal-back" onClick={() => setShowShortcuts(false)}>
      <div class="modal" role="dialog" aria-modal="true" aria-labelledby="shortcuts-title" onClick={(e) => e.stopPropagation()}>
        <h3 id="shortcuts-title">Keyboard shortcuts</h3>
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
