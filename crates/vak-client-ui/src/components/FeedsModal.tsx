import { trapFocus } from "../focusTrap";
import { createSignal, For, Show } from "solid-js";
import { feedsOpen, setFeedsOpen } from "../store";
import * as api from "../api";
import type { FeedSearchResult } from "../types";

export default function FeedsModal() {
  const [query, setQuery] = createSignal("");
  const [results, setResults] = createSignal<FeedSearchResult[]>([]);
  const [answer, setAnswer] = createSignal<string | null>(null);
  const [followUps, setFollowUps] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const search = async () => {
    const q = query().trim();
    if (!q || busy()) return;
    setBusy(true);
    setError(null);
    try {
      const res = await api.feedSearch({ q, limit: 20 });
      setResults(res.results);
      setAnswer(res.answer ?? null);
      setFollowUps(res.follow_up_questions ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    setBusy(false);
  };

  const searchFollowUp = (q: string) => {
    setQuery(q);
    void search();
  };

  return (
    <Show when={feedsOpen()}>
      <div class="modal-back" onClick={() => setFeedsOpen(false)}>
        <div class="modal search-modal" role="dialog" aria-modal="true" aria-labelledby="feeds-modal-title"
             onClick={(e) => e.stopPropagation()} use:trapFocus>
          <h3 id="feeds-modal-title">Search Feeds</h3>
          <form class="task-add-row" onSubmit={(e) => { e.preventDefault(); void search(); }}>
            <input class="search-input" placeholder="Search feed items..."
                   ref={(el) => requestAnimationFrame(() => el.focus())}
                   value={query()} onInput={(e) => setQuery(e.currentTarget.value)} />
            <button class="btn primary" type="submit" disabled={!query().trim() || busy()}>
              {busy() ? "..." : "Search"}
            </button>
          </form>

          <Show when={error()}>
            <div class="gate-err">{error()}</div>
          </Show>

          <Show when={answer()}>
            <div style={{ padding: "8px 12px", "font-size": "12px", color: "var(--text-soft)", background: "var(--surface-raised)", "border-radius": "var(--radius-sm)", margin: "8px 0" }}>
              {answer()}
            </div>
          </Show>

          <Show when={followUps().length > 0}>
            <div style={{ display: "flex", gap: "4px", "flex-wrap": "wrap", margin: "8px 0" }}>
              <For each={followUps()}>
                {(q) => (
                  <span
                    style={{ "font-size": "11px", padding: "3px 8px", "border-radius": "999px", background: "var(--surface-raised)", border: "1px solid var(--border)", cursor: "pointer" }}
                    onClick={() => searchFollowUp(q)}
                  >
                    {q}
                  </span>
                )}
              </For>
            </div>
          </Show>

          <div class="search-results">
            <For each={results()} fallback={<Show when={query().trim() && !busy()}>
              <div class="dock-empty">No matches.</div>
            </Show>}>
              {(item) => (
                <a class="search-hit" href={item.url} target="_blank" rel="noopener">
                  <span class="search-hit-body">
                    <span class="search-hit-snippet">{item.title}</span>
                    <span class="search-hit-meta">
                      {item.source_name} · {item.published_date}
                      <Show when={item.score > 0}> · {Math.round(item.score * 100)}%</Show>
                    </span>
                  </span>
                </a>
              )}
            </For>
          </div>

          <div class="bo-foot" style={{ "margin-top": "10px" }}>
            <button class="btn primary" onClick={() => setFeedsOpen(false)}>Close</button>
          </div>
        </div>
      </div>
    </Show>
  );
}
