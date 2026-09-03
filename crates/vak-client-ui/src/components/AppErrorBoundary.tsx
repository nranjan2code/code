import { ErrorBoundary as SolidErrorBoundary } from "solid-js";
import type { JSX } from "solid-js";

/**
 * The one thing standing between a render exception in any pane and a
 * totally blank window. Before this, nothing in the tree caught a render
 * error at all: one bad render — a null field the store didn't guard, a
 * malformed event, a third-party chart library throwing on unexpected
 * data — took the entire application down with no way back short of
 * relaunching, and (per docs/design/29-personal-os.md) no console to
 * explain why, because a bundled desktop app has no visible stderr.
 *
 * This does not fix whatever throws — it turns "the whole app is a blank
 * white/black rectangle" into "one recognizable error screen naming what
 * broke, with a way to recover without losing the session on disk" (the
 * ledger is durable regardless of what the client does).
 */
export default function AppErrorBoundary(props: { children: JSX.Element }) {
  return (
    <SolidErrorBoundary
      fallback={(err, reset) => (
        <div class="app-crash" role="alert">
          <div class="app-crash-card">
            <h1>Something went wrong in the UI</h1>
            <p>
              This is a rendering error in the desktop client, not a lost
              task — everything already on disk is untouched.
            </p>
            <pre class="app-crash-detail">{err instanceof Error ? err.message : String(err)}</pre>
            <div class="app-crash-actions">
              <button class="btn primary" onClick={() => reset()}>
                Try again
              </button>
              <button class="btn" onClick={() => window.location.reload()}>
                Reload
              </button>
            </div>
          </div>
        </div>
      )}
    >
      {props.children}
    </SolidErrorBoundary>
  );
}
