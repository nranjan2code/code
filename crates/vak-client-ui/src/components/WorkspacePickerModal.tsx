import { Show } from "solid-js";
import { trapFocus } from "../focusTrap";
import { setWorkspacePickerOpen, workspacePickerOpen, workspaceSwitching } from "../store";
import { switchWorkspace } from "../App";
import DirectoryPicker from "./DirectoryPicker";
import Icon from "./Icon";

export default function WorkspacePickerModal() {
  return (
    <Show when={workspacePickerOpen()}>
      <div class="modal-back" onClick={() => setWorkspacePickerOpen(false)}>
        <div
          class="modal"
          role="dialog"
          aria-modal="true"
          aria-labelledby="workspace-picker-title"
          onClick={(e) => e.stopPropagation()}
          use:trapFocus
        >
          <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px;">
            <h3 id="workspace-picker-title" style="margin: 0;">Open a workspace</h3>
            <button
              class="icon-button subtle"
              aria-label="Close"
              onClick={() => setWorkspacePickerOpen(false)}
            >
              <Icon name="close" size={14} />
            </button>
          </div>
          <p style="color: var(--muted); font-size: 12.5px; margin: 0 0 12px;">
            Choose a project folder on the machine running Vak. Existing tasks and settings will be restored.
          </p>
          <DirectoryPicker
            disabled={workspaceSwitching()}
            onPick={async (dir) => {
              setWorkspacePickerOpen(false);
              await switchWorkspace(dir);
            }}
          />
        </div>
      </div>
    </Show>
  );
}
