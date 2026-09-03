import { trapFocus } from "../focusTrap";
import { createEffect, createSignal, For, Show } from "solid-js";
import { sessions, setTranscriptViewId, transcriptToItems, transcriptViewId, type Item } from "../store";
import * as api from "../api";
import { ItemView } from "./ChatPane";
import Icon from "./Icon";

/**
 * Read-only historical transcript (docs/design/29): any session by id, served
 * straight from the on-disk ledger via GET /sessions/{id}/transcript. Never
 * attaches, never opens a stream, and never writes live view state — the
 * running/live path in the sidebar is untouched.
 */
export default function TranscriptModal() {
  const [items, setItems] = createSignal<Item[] | null>(null);
  const [count, setCount] = createSignal(0);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  let scroller!: HTMLDivElement;

  const title = () =>
    sessions().find((s) => s.session_id === transcriptViewId())?.title || "Untitled task";

  createEffect(() => {
    const id = transcriptViewId();
    if (!id) return;
    setItems(null);
    setError(null);
    setLoading(true);
    void (async () => {
      try {
        const t = await api.transcript(id);
        if (transcriptViewId() !== id) return; // a newer open won
        setItems(transcriptToItems(id, t.messages));
        setCount(t.count);
        // Transcripts read like chats: land pinned to the latest turn once
        // the rows exist to measure.
        requestAnimationFrame(() => scroller?.scrollTo({ top: scroller.scrollHeight }));
      } catch (e) {
        if (transcriptViewId() !== id) return;
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (transcriptViewId() === id) setLoading(false);
      }
    })();
  });

  return (
    <div class="modal-back" onClick={() => setTranscriptViewId(null)}>
      <div
        class="modal transcript-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="transcript-title"
        onClick={(e) => e.stopPropagation()}
        use:trapFocus
      >
        <div class="transcript-head">
          <Icon name="history" size={15} />
          <h3 id="transcript-title">{title()}</h3>
          <Show when={!loading() && !error()}>
            <span class="badge">{count()} events</span>
          </Show>
          <span class="badge">read-only</span>
          <button class="icon-button subtle" aria-label="Close transcript" onClick={() => setTranscriptViewId(null)}>
            <Icon name="close" size={14} />
          </button>
        </div>
        <Show when={error()}>
          <div class="gate-err">Could not load this transcript: {error()}</div>
        </Show>
        <div class="transcript-scroll" ref={scroller}>
          <Show when={!loading()} fallback={
            <div class="transcript-skeleton" aria-label="Loading transcript">
              <span class="skeleton-line wide" />
              <span class="skeleton-line medium" />
              <span class="skeleton-card" />
              <span class="skeleton-line wide" />
              <span class="skeleton-line short" />
            </div>
          }>
            <For each={items()}>
              {(it) => <ItemView item={it} />}
            </For>
            <Show when={items()?.length === 0 && !error()}>
              <div class="dock-empty">This task has no recorded events.</div>
            </Show>
          </Show>
        </div>
      </div>
    </div>
  );
}
