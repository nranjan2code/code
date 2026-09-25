import { createEffect, createMemo, createSignal, onCleanup, Show } from "solid-js";
import { connection, type Connection } from "../store";

type Trouble = Exclude<Connection, "live">;

/** What each connection state means to a reader, in words rather than hue.
 *
 * Colour is never the only carrier (DESIGN.md): each state also has its own
 * label and its own dot fill, so the difference survives a greyscale screen. */
const CONNECTION_COPY: Record<Trouble, { label: string; title: string }> = {
  connecting: { label: "Connecting…", title: "Connecting to Vakyartha." },
  reconnecting: {
    label: "Reconnecting…",
    title: "The connection dropped. Trying again; work already running continues.",
  },
  resyncing: {
    label: "Catching up…",
    title: "Reconnected after a long gap. Reloading the conversation.",
  },
  offline: {
    label: "Offline",
    title: "No connection. Anything you send waits until it returns.",
  },
};

/** How long a state must last before it is shown. A first connection stays
 * silent for a few seconds, so an ordinary load shows nothing at all. */
const GRACE_MS: Record<Trouble, number> = {
  connecting: 5000,
  reconnecting: 1500,
  resyncing: 1500,
  offline: 1500,
};

export default function StatusBar() {
  const [shown, setShown] = createSignal<Trouble | null>(null);
  createEffect(() => {
    const state = connection();
    if (state === "live") {
      setShown(null);
      return;
    }
    const timer = window.setTimeout(() => setShown(state), GRACE_MS[state]);
    onCleanup(() => window.clearTimeout(timer));
  });
  const copy = createMemo(() => {
    const state = shown();
    return state ? CONNECTION_COPY[state] : null;
  });

  return (
    <Show when={shown()}>
      {(state) => (
        <div class="statusbar" data-conn={state()} title={copy()?.title} role="status" aria-live="polite" aria-atomic="true">
          <span class="dot" aria-hidden="true" />
          <span class="visually-hidden">Connection: </span>
          {copy()?.label}
        </div>
      )}
    </Show>
  );
}
