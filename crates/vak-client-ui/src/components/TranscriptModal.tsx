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
  const [page, setPage] = createSignal<api.ConversationSnapshot["page"]>();
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  let scroller!: HTMLDivElement;

  const title = () =>
    sessions().find((s) => s.session_id === transcriptViewId())?.title || "Untitled task";

  const load = async (id: string, before?: string) => {
    setError(null);
    setLoading(true);
    try {
      const snapshot = await api.conversationSnapshot(id, { before });
      if (transcriptViewId() !== id) return;
      const t = snapshot.transcript;
      setItems(transcriptToItems(id, t.messages, t.entries));
      setPage(snapshot.page);
      setCount(snapshot.page?.total_turns ?? t.count);
      requestAnimationFrame(() => scroller?.scrollTo({ top: before ? 0 : scroller.scrollHeight }));
    } catch (error) {
      if (transcriptViewId() === id) setError(error instanceof Error ? error.message : String(error));
    } finally {
      if (transcriptViewId() === id) setLoading(false);
    }
  };
  createEffect(() => {
    const id = transcriptViewId();
    if (!id) return;
    setItems(null);
    setPage(undefined);
    void load(id);
  });

  return (
    <Sheet
      size="wide"
      class="transcript-modal"
      title={title()}
      subtitle={loading() || error() ? "Read only" : `Read only · ${count()} turns`}
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
            <Show when={page()?.older}><button class="btn sm" disabled={loading()} onClick={() => void load(transcriptViewId()!, page()!.older!)}>Earlier messages</button></Show>
            <For each={items()}>
              {(it) => <ItemView item={it} />}
            </For>
            <Show when={page()?.newer}><button class="btn sm" disabled={loading()} onClick={() => void load(transcriptViewId()!, page()!.newer!)}>Newer messages</button></Show>
            <Show when={items()?.length === 0 && !error()}>
              <div class="dock-empty">This task has no recorded events.</div>
            </Show>
          </Show>
        </div>
    </Sheet>
  );
}
