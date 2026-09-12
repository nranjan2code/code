import { createMemo, Show } from "solid-js";
import { connection, type Connection } from "../store";

/** What each connection state means to a reader, in words rather than hue.
 *
 * Colour is never the only carrier (DESIGN.md): each state also has its own
 * label text and its own dot fill, so the difference survives both a
 * greyscale screen and the ~8% of men for whom green and red are one dot. */
const CONNECTION_COPY: Record<Connection, { label: string; title: string }> = {
  live: { label: "Live", title: "Streaming events from the agent" },
  reconnecting: {
    label: "Reconnecting",
    title: "The event stream dropped. Retrying — work already running is unaffected.",
  },
  resyncing: {
    label: "Resyncing",
    title: "Reconnected past the replay window; rebuilding the transcript from the ledger.",
  },
  offline: {
    label: "Offline",
    title: "No connection to the server. Anything you send is held until it returns.",
  },
};

export default function StatusBar() {
  const conn = createMemo(() => CONNECTION_COPY[connection()]);

  return (
    <Show when={connection() !== "live"}>
      <footer class="statusbar statusbar-alert">
        <div class="st-left">
          {/* First, because on a remote surface "can I even reach it" outranks
              every other fact in this bar. */}
          <span
            class="st-item st-conn"
            data-conn={connection()}
            title={conn().title}
            role="status"
            aria-live="polite"
            aria-atomic="true"
          >
            <span class="dot" classList={{ run: connection() === "live" }} aria-hidden="true" />
            <span class="visually-hidden">Connection: </span>
            {conn().label}
          </span>
        </div>
      </footer>
    </Show>
  );
}
