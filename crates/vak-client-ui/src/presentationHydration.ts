import type { OutputTimeline, OutputItem, PresentationDelta } from "./types";

/** Stream snapshots describe the current turn, while historical pages keep
 * their own authority. Replace those turns and stale live items only. */
export function mergeTurnSnapshot(current: OutputTimeline | null, incoming: OutputTimeline): OutputTimeline {
  if (!current) return incoming;
  if (incoming.cursor?.startsWith("live:")) return mergePresentationSnapshot(current, incoming);
  const turns = new Set(incoming.items.map(item => item.turn_id));
  const retained = current.items.filter(item => !turns.has(item.turn_id)
    && !item.turn_id.startsWith("live") && item.provenance?.source !== "live_event");
  return { ...incoming, items: [...retained, ...incoming.items],
    answers: [...(current.answers ?? []).filter(answer => !turns.has(answer.turn_id)), ...(incoming.answers ?? [])] };
}

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
  if ((delta.type === "item_started" || delta.type === "item_replaced")
      && delta.item.provenance?.source === "live_event"
      && current.items.some(item => item.id === delta.item.id && item.provenance?.source !== "live_event")) return current;
  switch (delta.type) {
    case "item_started":
    case "item_replaced":
      return { ...current, items: upsertItem(current.items.filter((item) => !delta.item.placement?.replaces || item.provenance?.presentation_id !== delta.item.placement.replaces), delta.item) };
    case "text_delta":
      return {
        ...current,
        items: current.items.map((item) => {
          if (item.id !== delta.item_id) return item;
          const text = delta.text ?? `${item.fallback_text}${delta.delta}`;
          if (item.content.type !== "document") {
            return { ...item, fallback_text: text };
          }
          return {
            ...item,
            fallback_text: text,
            content: {
              type: "document",
              document: { ...item.content.document, source_markdown: text, blocks: [] },
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

/** Coordinates reads of one conversation. A fetched snapshot is not applied
 * state: running views may defer it, and slower reads must not replace newer
 * ones. Only the atomic view commit advances the applied fingerprint. */
export class ConversationHydration {
  private versions = new Map<string, number>();
  private applied = new Map<string, string>();

  begin(id: string): number {
    const version = (this.versions.get(id) ?? 0) + 1;
    this.versions.set(id, version);
    return version;
  }

  current(id: string, version: number): boolean {
    return this.versions.get(id) === version;
  }

  invalidate(id: string): void {
    this.begin(id);
    this.applied.delete(id);
  }

  commit(id: string, version: number, fingerprint: string, ready: boolean, apply: () => void): boolean {
    if (!this.current(id, version) || !ready || this.applied.get(id) === fingerprint) return false;
    apply();
    this.applied.set(id, fingerprint);
    return true;
  }
}
