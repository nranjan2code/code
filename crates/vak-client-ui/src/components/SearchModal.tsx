import { trapFocus } from "../focusTrap";
import { createSignal, For, Show } from "solid-js";
import { searchOpen, setSearchOpen } from "../store";
import { activate } from "../App";
import * as api from "../api";
import type { SearchHit } from "../api";
import Icon from "./Icon";

const LIMITS = [8, 25, 50];

/**
 * Recall search over session ledgers (docs/design/29-personal-os.md P1/P4).
 * Global mode crosses every project under the sessions home; hits then carry
 * the `project_hash` of the ledger they were found in.
 */
export default function SearchModal() {
  const [query, setQuery] = createSignal("");
  const [global, setGlobal] = createSignal(false);
  const [limit, setLimit] = createSignal(LIMITS[0]);
  const [hits, setHits] = createSignal<SearchHit[] | null>(null);
  const [searched, setSearched] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const run = async () => {
    const q = query().trim();
    if (!q || busy()) return;
    setBusy(true);
    setError(null);
    try {
      const res = await api.searchSessions(q, limit(), global());
      setHits(res.hits);
      setSearched(q);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Show when={searchOpen()}>
      <div class="modal-back" onClick={() => setSearchOpen(false)}>
        <div class="modal search-modal" role="dialog" aria-modal="true" aria-labelledby="search-title" onClick={(e) => e.stopPropagation()} use:trapFocus>
          <h3 id="search-title">Recall search</h3>
          <form
            class="task-add-row"
            onSubmit={(e) => {
              e.preventDefault();
              void run();
            }}
          >
            <input
              class="search-input"
              placeholder="Search transcripts and memory…"
              aria-label="Search transcripts and memory"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
              ref={(el) => requestAnimationFrame(() => el.focus())}
            />
            <button
              type="button"
              class="chip"
              classList={{ on: global() }}
              aria-pressed={global()}
              title="Search every project under the Vak home, not just this workspace"
              onClick={() => setGlobal((v) => !v)}
            >
              Global
            </button>
            <label class="bo-n">
              <span class="hint">limit</span>
              <select value={String(limit())} onChange={(e) => setLimit(Number(e.currentTarget.value))} aria-label="Result limit">
                <For each={LIMITS}>{(n) => <option value={String(n)}>{n}</option>}</For>
              </select>
            </label>
            <button class="btn primary" type="submit" disabled={!query().trim() || busy()}>
              {busy() ? "…" : "Search"}
            </button>
          </form>

          <Show when={error()}>
            <div class="gate-err">{error()}</div>
          </Show>

          <Show when={!error() && searched()}>
            <p class="receipt-summary">
              {hits()?.length ?? 0} hit{hits()?.length === 1 ? "" : "s"} for “{searched()}”
              {global() ? " across all projects" : " in this workspace"}
            </p>
          </Show>

          <div class="search-results">
            <For each={hits()} fallback={<Show when={searched() && !error()}><div class="dock-empty">No matches.</div></Show>}>
              {(hit) => (
                <button class="search-hit" disabled={hit.role === "memory" || hit.role === "profile"} onClick={() => { setSearchOpen(false); if (hit.role !== "memory" && hit.role !== "profile") void activate(hit.session_id); }}>
                  <span class="badge">{hit.role}</span>
                  <span class="search-hit-body">
                    <span class="search-hit-snippet">{hit.snippet}</span>
                    <span class="search-hit-meta">
                      {hit.session_id.slice(0, 8)} · {new Date(hit.ts).toLocaleString()} · score {hit.score.toFixed(2)}
                    </span>
                  </span>
                  <Show when={hit.project_hash} keyed>
                    {(hash) => (
                      <span class="badge search-hit-project" title={`Project ${hash}`}>
                        {hash.slice(0, 8)}
                      </span>
                    )}
                  </Show>
                  <Icon name="chevron" size={13} />
                </button>
              )}
            </For>
          </div>

          <div class="bo-foot" style="margin-top:10px">
            <span class="hint">memory notes outrank transcript lines · global adds project chips</span>
            <button class="btn primary" onClick={() => setSearchOpen(false)}>Close</button>
          </div>
        </div>
      </div>
    </Show>
  );
}
