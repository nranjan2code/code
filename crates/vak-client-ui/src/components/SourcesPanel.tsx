import { createResource, createSignal, For, Show } from "solid-js";
import * as api from "../api";

/** Plain words for an item held back from the Agent (doc 75 §7). */
const HELD: Record<string, string> = {
  quarantined: "Held back",
  blocked: "Blocked",
};

/**
 * What the sources an Agent follows brought in (plan M6.5), newest first.
 * Searching them is the search sheet's job; managing them is in the admin
 * console's Sources.
 */
export default function SourcesPanel() {
  const [source, setSource] = createSignal("");
  const [checking, setChecking] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [sources] = createResource(() => api.intakeSources().then((res) => res.sources));
  const [items, { refetch }] = createResource(source, (id) =>
    api.intakeItems({ source: id || undefined, limit: 50 }).then((res) => res.items),
  );

  const sourceName = (itemId: string) =>
    sources()?.find((entry) => itemId.startsWith(`itm:${entry.id}:`))?.name ?? "";

  const checkNow = async () => {
    setChecking(true);
    setError(null);
    try {
      const chosen = (sources() ?? []).filter((entry) => !source() || entry.id === source());
      await Promise.all(chosen.map((entry) => api.intakePoll(entry.id)));
      setTimeout(() => void refetch(), 3000);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    setChecking(false);
  };

  return (
    <div class="sources-panel" role="tabpanel" aria-label="Sources">
      <Show when={error() || items.error}>
        <div class="worker-error">{error() ?? String(items.error)}</div>
      </Show>
      <div class="dock-head">
        <span>Sources</span>
        <button class="btn sm" onClick={() => void checkNow()} disabled={checking() || !(sources() ?? []).length}>
          {checking() ? "Checking…" : "Check now"}
        </button>
      </div>
      <Show when={(sources() ?? []).length > 1}>
        <div class="sources-filter">
          <select value={source()} onChange={(e) => setSource(e.currentTarget.value)} aria-label="Source">
            <option value="">All sources</option>
            <For each={sources() ?? []}>{(entry) => <option value={entry.id}>{entry.name}</option>}</For>
          </select>
        </div>
      </Show>
      <div class="sources-list">
        <Show
          when={(items() ?? []).length > 0}
          fallback={
            <div class="dock-empty">
              {(sources() ?? []).length ? "Nothing new yet." : "No sources yet. Add one in the admin console under Sources."}
            </div>
          }
        >
          <For each={items() ?? []}>
            {(item) => {
              const held = HELD[item.status ?? ""];
              const meta = () =>
                [sourceName(item.id), item.created_at?.slice(0, 10), held].filter(Boolean).join(" · ");
              return (
                <Show
                  when={item.locator && !held}
                  fallback={
                    <div class="sb-item" classList={{ "sb-item-held": !!held }}>
                      <span class="sb-item-copy">
                        <span class="sb-title">{item.title}</span>
                        <span class="sb-item-meta">{meta()}</span>
                      </span>
                    </div>
                  }
                >
                  <a class="sb-item" href={item.locator} target="_blank" rel="noreferrer noopener">
                    <span class="sb-item-copy">
                      <span class="sb-title">{item.title}</span>
                      <span class="sb-item-meta">{meta()}</span>
                    </span>
                  </a>
                </Show>
              );
            }}
          </For>
        </Show>
      </div>
    </div>
  );
}
