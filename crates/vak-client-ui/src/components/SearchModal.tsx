import { createSignal, For, Show } from "solid-js";
import { searchOpen, setSearchOpen } from "../store";
import { activate } from "../App";
import * as api from "../api";
import type { SearchHit } from "../api";
import Icon from "./Icon";
import Sheet from "./Sheet";

/** What a hit is, in words. */
const HIT_WORDS: Record<SearchHit["role"], string> = {
  conversation: "Conversation",
  memory: "Memory",
  profile: "About you",
  entity: "Person or thing",
  item: "From your sources",
};

const LIMITS = [8, 25, 50];

/**
 * Recall search over session ledgers (docs/design/29-personal-os.md P1/P4).
 * Global mode crosses every project under the sessions home; hits then carry
 * the `space_id` of the ledger they were found in.
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
      <Sheet size="wide" class="search-modal" title="Search" subtitle="Your conversations and what your agents remember." onClose={() => setSearchOpen(false)}>
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
              title="Search every project under the Vakyartha home, not just this workspace"
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
              {(hit) => {
                const body = (
                  <>
                    <span class="badge">{HIT_WORDS[hit.role] ?? hit.role}</span>
                    <span class="search-hit-body">
                      <span class="search-hit-snippet">{hit.snippet}</span>
                      <Show when={hit.ts}>
                        {(ts) => <span class="search-hit-meta">{new Date(ts()).toLocaleString()}</span>}
                      </Show>
                    </span>
                    <Icon name="chevron" size={13} />
                  </>
                );
                return (
                  <Show
                    when={hit.role === "item" && hit.link}
                    fallback={
                      <button type="button" class="search-hit" disabled={hit.role !== "conversation"} onClick={() => { setSearchOpen(false); if (hit.role === "conversation") void activate(hit.session_id); }}>
                        {body}
                      </button>
                    }
                  >
                    {(link) => (
                      <a class="search-hit" href={link()} target="_blank" rel="noreferrer noopener">
                        {body}
                      </a>
                    )}
                  </Show>
                );
              }}
            </For>
          </div>

          <div class="bo-foot" style="margin-top:10px">
            <span class="hint">Global also looks through your other workspaces</span>
          </div>
      </Sheet>
    </Show>
  );
}
