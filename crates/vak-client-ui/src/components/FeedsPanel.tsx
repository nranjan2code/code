import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import * as api from "../api";
import type { FeedItem, FeedSearchResponse } from "../types";

export default function FeedsPanel() {
  const [query, setQuery] = createSignal("");
  const [results, setResults] = createSignal<FeedSearchResponse | null>(null);
  const [items, setItems] = createSignal<FeedItem[]>([]);
  const [stats, setStats] = createSignal<{ total: number; today: number } | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [view, setView] = createSignal<"search" | "browse">("search");

  const loadStats = async () => {
    try {
      const s = await api.feedStats();
      setStats({ total: s.total_items, today: s.items_today });
    } catch {
      // non-fatal
    }
  };

  const loadItems = async () => {
    try {
      const res = await api.feedItems({ limit: 30 });
      setItems(res.items || []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  createEffect(() => {
    void loadStats();
    void loadItems();
    const t = setInterval(() => void loadStats(), 60_000);
    onCleanup(() => clearInterval(t));
  });

  const search = async () => {
    const q = query().trim();
    if (!q || busy()) return;
    setBusy(true);
    setError(null);
    try {
      const res = await api.feedSearch({ q, limit: 10 });
      setResults(res);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    setBusy(false);
  };

  const triggerIngest = async () => {
    setBusy(true);
    try {
      await api.feedIngest();
      // Refetch real stats after ingestion
      await loadStats();
      await loadItems();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    setBusy(false);
  };

  return (
    <div class="feeds-panel" role="tabpanel" aria-label="Feed pipeline">
      <Show when={error()}>
        <div class="subagent-error">{error()}</div>
      </Show>

      <div class="dock-head">
        <span>Feeds</span>
        <div style={{ display: "flex", gap: "4px" }}>
          <button class="btn sm" onClick={triggerIngest} disabled={busy()}>
            {busy() ? "..." : "Ingest"}
          </button>
        </div>
      </div>

      <Show when={stats()}>
        <div style={{ padding: "8px 12px", "font-size": "11px", color: "var(--faint)", "border-bottom": "1px solid var(--border)" }}>
          {stats()!.total} items · {stats()!.today} today
        </div>
      </Show>

      <div style={{ padding: "8px 12px", display: "flex", gap: "4px", "border-bottom": "1px solid var(--border)" }}>
        <button type="button" class={`btn sm ${view() === "search" ? "primary" : ""}`} onClick={() => setView("search")}>Search</button>
        <button type="button" class={`btn sm ${view() === "browse" ? "primary" : ""}`} onClick={() => setView("browse")}>Browse</button>
      </div>

      <Show when={view() === "search"}>
        <form
          style={{ padding: "8px 12px", display: "flex", gap: "4px" }}
          onSubmit={(e) => { e.preventDefault(); void search(); }}
        >
          <input
            class="search-input"
            placeholder="Search feeds..."
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
          />
          <button class="btn primary sm" type="submit" disabled={!query().trim() || busy()}>
            {busy() ? "..." : "Go"}
          </button>
        </form>

        <Show when={results()}>
          <Show when={results()!.answer}>
            <div style={{ padding: "8px 12px", "font-size": "12px", color: "var(--text-soft)", "border-bottom": "1px solid var(--border)" }}>
              {results()!.answer}
            </div>
          </Show>
          <For each={results()!.results}>
            {(item) => (
              <a class="sb-item" href={item.url} target="_blank" rel="noopener">
                <span class="sb-item-copy">
                  <span class="sb-title">{item.title}</span>
                  <span class="sb-item-meta">
                    {item.source_name} · {item.published_date}
                    <Show when={item.score > 0}> · {Math.round(item.score * 100)}%</Show>
                  </span>
                </span>
              </a>
            )}
          </For>
        </Show>
      </Show>

      <Show when={view() === "browse"}>
        <div class="feeds-list">
          <Show when={items().length === 0} fallback={<For each={items()}>
            {(item) => (
              <a class="sb-item" href={item.url} target="_blank" rel="noopener">
                <span class="sb-item-copy">
                  <span class="sb-title">{item.title}</span>
                  <span class="sb-item-meta">
                    {item.source_name} · {item.published_at?.slice(0, 10)}
                  </span>
                </span>
              </a>
            )}
          </For>}>
            <div class="dock-empty">No items yet. Add sources and run ingestion.</div>
          </Show>
        </div>
      </Show>
    </div>
  );
}
