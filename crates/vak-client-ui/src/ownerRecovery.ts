import { createSignal } from "solid-js";

// Memory only: codes are never persisted in browser storage. This state lives
// above WorkspaceGate so a backend-ready event cannot unmount the only copy.
export const [pendingRecoveryCodes, setPendingRecoveryCodes] = createSignal<string[]>([]);
