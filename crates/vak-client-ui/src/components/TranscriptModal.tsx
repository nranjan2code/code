import { createEffect, createSignal, For, Show } from "solid-js";
import { sessions, setTranscriptViewId, transcriptToItems, transcriptViewId, type Item } from "../store";
import * as api from "../api";
import { ItemView } from "./ChatPane";
import Sheet from "./Sheet";
import Skeleton from "./Skeleton";

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
        setItems(transcriptToItems(id, t.messages, t.entries));
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
    <Sheet
      size="wide"
      class="transcript-modal"
      title={title()}
      subtitle={loading() || error() ? "Read only" : `Read only · ${count()} events`}
      onClose={() => setTranscriptViewId(null)}
    >
        <div class="transcript-actions">
          <button type="button" class="btn sm" onClick={async () => {
              const id = transcriptViewId();
              if (!id) return;
              try {
                const html = await api.transcriptHtml(id);
                const blob = new Blob([html], { type: "text/html;charset=utf-8" });
                const url = URL.createObjectURL(blob);
                const a = document.createElement("a");
                a.href = url;
                a.download = `session-${id}.html`;
                a.click();
                URL.revokeObjectURL(url);
              } catch (err) {
                console.error("Failed to export HTML:", err);
              }
            }}>Save as web page</button>
          <button type="button" class="btn sm" onClick={async () => {
              const id = transcriptViewId();
              if (!id) return;
              try {
                const md = await api.transcriptMarkdown(id);
                const blob = new Blob([md], { type: "text/markdown;charset=utf-8" });
                const url = URL.createObjectURL(blob);
                const a = document.createElement("a");
                a.href = url;
                a.download = `session-${id}.md`;
                a.click();
                URL.revokeObjectURL(url);
              } catch (err) {
                console.error("Failed to export Markdown:", err);
              }
            }}>Save as Markdown</button>
        </div>
        <Show when={error()}>
          <div class="gate-err">Could not load this transcript: {error()}</div>
        </Show>
        <div class="transcript-scroll" ref={scroller}>
          <Show when={!loading()} fallback={
            <Skeleton kind="transcript" class="transcript-skeleton" label="Loading the conversation" />
          }>
            <For each={items()}>
              {(it) => <ItemView item={it} />}
            </For>
            <Show when={items()?.length === 0 && !error()}>
              <div class="dock-empty">This task has no recorded events.</div>
            </Show>
          </Show>
        </div>
    </Sheet>
  );
}
