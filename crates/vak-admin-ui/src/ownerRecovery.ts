import { createSignal } from "solid-js";

// Keep one-time codes above route and sign-in components. A session refresh
// must never unmount the only visible copy before the owner saves it.
export const [pendingRecoveryCodes, setPendingRecoveryCodes] = createSignal<string[]>([]);
