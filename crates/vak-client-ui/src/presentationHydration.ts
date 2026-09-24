import type { OutputTimeline, OutputItem, PresentationDelta } from "./types";

/**
 * Merge a presentation snapshot at the boundary where the durable projection
 * and the live event projection meet.
 *
 * Durable snapshots are authoritative replacement points. Live SSE frames are
 * additive, in-flight projections: a subscriber can span several runs while
 * its server-local timeline still contains live items from the prior run. A
 * live frame therefore updates matching ids, retains settled items, removes
 * stale live-only items, and appends genuinely new live items.
 */
export function mergePresentationSnapshot(
  current: OutputTimeline | null,
  incoming: OutputTimeline,
): OutputTimeline {
  if (!current || !incoming.cursor?.startsWith("live:")) return incoming;

  const incomingById = new Map(incoming.items.map((item) => [item.id, item]));
  const currentIds = new Set(current.items.map((item) => item.id));
  const items = current.items.flatMap((item) => {
    const replacement = incomingById.get(item.id);
    const isLive = item.turn_id.startsWith("live") || item.provenance?.source === "live_event";
    // A live frame can update its own in-flight item, but it is not an
    // authoritative re-render of an already-settled turn. That authority
    // belongs to the next durable snapshot at the run boundary.
    if (replacement) return [isLive ? replacement : item];
    return isLive ? [] : [item];
  });
  for (const item of incoming.items) {
    if (!currentIds.has(item.id)) items.push(item);
  }
  return { ...incoming, items };
}

function upsertItem(items: OutputItem[], item: OutputItem): OutputItem[] {
  const idx = items.findIndex((candidate) => candidate.id === item.id);
  if (idx === -1) return [...items, item];
  const next = [...items];
  next[idx] = item;
  return next;
}

/**
 * Apply one live delta to the running presentation timeline, mirroring the
 * server's own `apply_stream_event` (vak-server/src/projection.rs). Most SSE
 * frames now carry only a delta (docs/audits Finding 2: a full-timeline
 * snapshot on every frame previously measured up to 9.3 MB per answer) — the
 * next authoritative snapshot arrives on run settlement or resync.
 *
 * A `TextDelta`/`ItemCompleted` on a `document` item updates the exact
 * source text but cannot re-run the server's Markdown-to-AST compiler
 * client-side; it clears `blocks` so the renderer's own `source_markdown`
 * fallback path (used whenever `blocks` is empty) carries the live text
 * until the next snapshot supplies the compiled blocks.
 */
export function applyPresentationDelta(
  current: OutputTimeline | null,
  delta: PresentationDelta,
): OutputTimeline | null {
  if (!current) return current;
  switch (delta.type) {
    case "item_started":
    case "item_replaced":
      return { ...current, items: upsertItem(current.items, delta.item) };
    case "text_delta":
      return {
        ...current,
        items: current.items.map((item) => {
          if (item.id !== delta.item_id) return item;
          if (item.content.type !== "document") {
            return { ...item, fallback_text: delta.text };
          }
          return {
            ...item,
            fallback_text: delta.text,
            content: {
              type: "document",
              document: { ...item.content.document, source_markdown: delta.text, blocks: [] },
            },
          };
        }),
      };
    case "item_completed":
      return {
        ...current,
        items: current.items.map((item) =>
          item.id === delta.item_id ? { ...item, status: delta.status } : item,
        ),
      };
    default:
      return current;
  }
}
