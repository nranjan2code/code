import { createEffect, createSignal, onCleanup, untrack } from "solid-js";

export interface Loader<T> {
  data: () => T | undefined;
  loading: () => boolean;
  error: () => string | null;
  reload: () => void;
}

/**
 * Loads what a viewer shows whenever its source changes. A load that has been
 * overtaken by a newer one, or by the viewer closing, is discarded and its
 * result disposed, so a slow read can never replace a newer subject's content
 * and nothing it allocated (a blob URL, a running server) outlives it.
 * `load` is told whether it is still current so it can stop early.
 */
export function createLoader<S, T>(
  source: () => S,
  load: (source: S, current: () => boolean) => Promise<T>,
  dispose?: (value: T) => void,
): Loader<T> {
  const [data, setData] = createSignal<T>();
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [attempt, setAttempt] = createSignal(0);
  let generation = 0;
  let held: { value: T } | undefined;

  const release = () => {
    if (held) dispose?.(held.value);
    held = undefined;
    setData(undefined);
  };

  createEffect(() => {
    const wanted = source();
    attempt();
    const mine = ++generation;
    const current = () => mine === generation;
    release();
    setLoading(true);
    setError(null);
    untrack(() => load(wanted, current))
      .then((value) => {
        if (!current()) {
          dispose?.(value);
          return;
        }
        held = { value };
        setData(() => value);
      })
      .catch((cause) => {
        if (current()) setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        if (current()) setLoading(false);
      });
  });

  onCleanup(() => {
    generation += 1;
    release();
  });

  return { data, loading, error, reload: () => setAttempt((count) => count + 1) };
}

/** Reads a file's text, or says why it cannot be. */
export async function readText(reader: { readFile(path: string): Promise<{ content?: string }> } | null, file: string): Promise<string> {
  if (!reader || !file) throw new Error("This preview has no file to read.");
  const response = await reader.readFile(file);
  if (response.content === undefined) throw new Error(`"${file}" has no readable text.`);
  return response.content;
}
