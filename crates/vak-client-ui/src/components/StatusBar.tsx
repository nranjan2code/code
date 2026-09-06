import { createMemo, Show } from "solid-js";
import { connection, density, health, setDensity, type Connection, type Density } from "../store";

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
  const h = createMemo(() => health());
  const conn = createMemo(() => CONNECTION_COPY[connection()]);

  return (
    <footer class="statusbar">
      <div class="st-left">
        {/* First, because on a remote surface "can I even reach it" outranks
            every other fact in this bar. */}
        <span
          class="st-item st-conn"
          data-conn={connection()}
          title={conn().title}
        >
          <span class="dot" classList={{ run: connection() === "live" }} aria-hidden="true" />
          <span class="visually-hidden">Connection: </span>
          {conn().label}
        </span>
        <span class="st-item st-model" classList={{ offline: !h() }} title={`provider: ${h()?.provider ?? "connecting"}`}>
          {h()?.model ?? "Connecting…"}
        </span>
        <span class="st-item st-sandbox" title="sandbox backend">{h()?.sandbox}</span>
        <Show when={(h()?.warnings?.length ?? 0) > 0}>
          <span class="st-item warn" title={JSON.stringify(h()?.warnings)}>
            ⚠ {h()?.warnings?.length} warning(s)
          </span>
        </Show>
      </div>
      <div class="st-right">
        <select
          class="st-density"
          value={density()}
          onChange={(e) => setDensity(e.currentTarget.value as Density)}
          title="Transcript detail level (Outcome: results only, Balanced: key steps, Audit: full verbose receipts)"
          aria-label="Transcript detail"
        >
          <option value="outcome">Outcome detail</option>
          <option value="balanced">Balanced detail</option>
          <option value="audit">Audit detail</option>
        </select>
        <Show when={h()?.provider}>
          <span class="st-item" title="active provider">{h()?.provider}</span>
        </Show>
      </div>
    </footer>
  );
}
