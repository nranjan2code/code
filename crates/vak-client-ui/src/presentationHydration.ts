import type { OutputTimeline } from "./types";

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
