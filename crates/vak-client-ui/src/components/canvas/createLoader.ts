import { createEffect, createMemo, createSignal, on, onCleanup, untrack } from "solid-js";
import { ApiError } from "../../api";

export interface Loader<T> {
  data: () => T | undefined;
  loading: () => boolean;
  /** What went wrong, in everyday words. */
  error: () => string | null;
  /** The error as it came, for Show technical details. */
  detail: () => string | null;
  reload: () => void;
}

/**
 * Loads what a viewer shows whenever its source changes. A load that has been
 * overtaken by a newer one, or by the viewer closing, is discarded and its
 * result disposed, so a slow read can never replace a newer subject's content
 * and nothing it allocated (a blob URL, a preview, a dev server's lease)
 * outlives it. `load` is told whether it is still current so it can stop
 * early.
 *
 * A reload (`reloadOn` changing, or `reload()`) keeps what is shown until the
 * fresh content is ready, so the page does not blank out and come back; what
 * it replaces is then disposed.
 */
export function createLoader<S, T>(
  source: () => S,
  load: (source: S, current: () => boolean) => Promise<T>,
  dispose?: (value: T) => void,
  options?: { reloadOn?: () => unknown },
): Loader<T> {
  const [data, setData] = createSignal<T>();
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [detail, setDetail] = createSignal<string | null>(null);
  const [attempt, setAttempt] = createSignal(0);
  let generation = 0;
  let held: { value: T } | undefined;
  let loadedSource: S | undefined;
  let hasLoadedSource = false;

  const release = () => {
    if (held) dispose?.(held.value);
    held = undefined;
    setData(undefined);
  };

  // A source is what to load, not when it was read: an equal one, however many
  // times it is produced, loads nothing again (a reader picking a line must
  // not reload the page they are reading).
  const wantedSource = createMemo(source, undefined, { equals: sameSource });
  createEffect(() => {
    const wanted = wantedSource();
    attempt();
    const mine = ++generation;
    const current = () => mine === generation;
    const keepPrevious = held !== undefined && hasLoadedSource && sameSource(loadedSource, wanted);
    if (!keepPrevious) release();
    setLoading(true);
    setError(null);
    setDetail(null);
    untrack(() => load(wanted, current))
      .then((value) => {
        if (!current()) {
          dispose?.(value);
          return;
        }
        const replaced = held;
        held = { value };
        loadedSource = wanted;
        hasLoadedSource = true;
        setData(() => value);
        if (replaced && replaced.value !== value) dispose?.(replaced.value);
      })
      .catch((cause) => {
        if (!current()) return;
        setError(problemWords(cause));
        setDetail(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        if (current()) setLoading(false);
      });
  });

  if (options?.reloadOn) {
    createEffect(on(options.reloadOn, () => setAttempt((count) => count + 1), { defer: true }));
  }

  onCleanup(() => {
    generation += 1;
    release();
    loadedSource = undefined;
    hasLoadedSource = false;
  });

  return { data, loading, error, detail, reload: () => setAttempt((count) => count + 1) };
}

function sameSource(a: unknown, b: unknown): boolean {
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((item, index) => item === b[index]);
  return a === b;
}

/** What went wrong, in the words a reader needs: what happened and what to do. */
export function problemWords(cause: unknown): string {
  if (cause instanceof ApiError) {
    if (cause.status === 401) return "You were signed out. Sign in again to see this.";
    if (cause.status === 403) return "Vakyartha isn't allowed to open this.";
    if (cause.status === 404) return "This isn't there any more. It may have been moved or deleted.";
    if (cause.status >= 500) return "Vakyartha couldn't open this just now. Try again in a moment.";
    return cause.message;
  }
  // `fetch` rejects with a TypeError when the server cannot be reached at all.
  if (cause instanceof TypeError) return "Vakyartha can't be reached. Check that it's running, then try again.";
  return cause instanceof Error ? cause.message : String(cause);
}

/** Reads a file's text, or says why it cannot be. */
export async function readText(reader: { readFile(path: string): Promise<{ content?: string }> } | null, file: string): Promise<string> {
  if (!reader || !file) throw new Error("There is no file to show here.");
  const response = await reader.readFile(file);
  if (response.content === undefined) throw new Error("This file isn't text, so it can't be shown here.");
  return response.content;
}
