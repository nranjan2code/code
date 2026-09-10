import { createResource, createSignal, For, Show } from "solid-js";
import { listDirectory } from "../api";
import Icon from "./Icon";

/**
 * Choose a folder on the *server's* filesystem.
 *
 * The web host has no native dialog by design: the machine running the
 * agent is the one whose folders matter, and a browser file picker would
 * be listing the operator's laptop instead (docs/design/48-web-client.md
 * §5). So the server lists directories — names only, never file contents,
 * rooted at `[server] workspace_roots`.
 *
 * Repositories are marked, because in practice that is what the operator
 * is looking for and scanning a home directory for the one folder with a
 * `.git` in it is the whole task.
 */
export default function DirectoryPicker(props: {
  onPick: (path: string) => void;
  disabled?: boolean;
}) {
  const [path, setPath] = createSignal<string | undefined>(undefined);
  const [listing, { refetch }] = createResource(path, listDirectory);

  return (
    <div class="dirpick">
      <div class="dirpick-bar">
        <button
          type="button"
          class="settings-button"
          disabled={!listing()?.parent || listing.loading}
          title="Up one level"
          aria-label="Up one level"
          onClick={() => setPath(listing()?.parent ?? undefined)}
        >
          ↑
        </button>
        <code class="dirpick-path" title={listing()?.path ?? ""}>
          {listing()?.path ?? "…"}
        </code>
        <button type="button" class="settings-button" disabled={listing.loading} onClick={() => void refetch()}>
          Refresh
        </button>
      </div>

      <Show when={listing.error}>
        <div class="gate-err">
          {listing.error instanceof Error ? listing.error.message : String(listing.error)}
        </div>
      </Show>

      <div class="dirpick-list" role="listbox" aria-label="Folders">
        <For
          each={listing()?.entries ?? []}
          fallback={<div class="dirpick-empty">{listing.loading ? "Loading…" : "No folders here."}</div>}
        >
          {(entry) => (
            <div class="dirpick-row">
              <button
                type="button"
                class="dirpick-enter"
                title={`Open ${entry.name}`}
                onClick={() => setPath(entry.path)}
              >
                <Icon name="folder" size={14} />
                <span>{entry.name}</span>
                <Show when={entry.git}>
                  <span class="chip sm">git</span>
                </Show>
              </button>
              <button
                type="button"
                class="btn sm"
                disabled={props.disabled}
                onClick={() => props.onPick(entry.path)}
              >
                Choose
              </button>
            </div>
          )}
        </For>
      </div>

      <Show when={listing()?.path}>
        {(here) => (
          <button
            type="button"
            class="btn primary dirpick-here"
            disabled={props.disabled}
            onClick={() => props.onPick(here())}
          >
            Use this folder
          </button>
        )}
      </Show>
    </div>
  );
}
