import { createEffect, createResource, For, on, onCleanup, Show } from "solid-js";
import * as api from "../api";
import type { ArtifactSummary } from "../api";
import { activeId, isRunning, libraryOpen, openInLibrary } from "../store";

/** What one file waits on the person for, in a few words (doc 90 §5). */
function waitsFor(artifact: ArtifactSummary): string {
  const needs = artifact.needs!;
  if (needs.missing) return "not in your folder now";
  if (needs.behind > 0) return "changed outside Vak since its draft";
  return needs.waiting === 1 ? "1 draft" : `${needs.waiting} drafts`;
}

/** The files of this workspace that wait on the person: drafts to review,
 *  or a file no longer in the folder (doc 90 §5). Read from the Library's
 *  record, so it is what the artifact pages say. */
export default function NeedsYou() {
  const [files, { refetch }] = createResource(async () => {
    const listed = await api.library().catch(() => ({ artifacts: [] as ArtifactSummary[] }));
    return listed.artifacts.filter((artifact) => artifact.needs && (artifact.needs.waiting > 0 || artifact.needs.missing));
  });
  // A turn that ends, the Library closing and coming back to the window can
  // each change what waits.
  createEffect(on(() => isRunning(activeId()), (running, was) => { if (was && !running) void refetch(); }));
  createEffect(on(libraryOpen, (open, was) => { if (was && !open) void refetch(); }));
  const onFocus = () => void refetch();
  window.addEventListener("focus", onFocus);
  onCleanup(() => window.removeEventListener("focus", onFocus));
  return (
    <Show when={(files() ?? []).length > 0}>
      <details class="needs-you">
        <summary class="needs-you-lead"><span class="dot" aria-hidden="true" /><strong>Needs you</strong><span>{files()!.length === 1 ? "1 file" : `${files()!.length} files`}</span></summary>
        <div class="needs-you-items">
          <For each={files()}>
            {(artifact) => (
              <button type="button" class="needs-you-item" onClick={() => openInLibrary(artifact.id)}>
                <span class="needs-you-name">{artifact.name}</span>
                <span class="needs-you-what">{waitsFor(artifact)}</span>
              </button>
            )}
          </For>
        </div>
      </details>
    </Show>
  );
}
