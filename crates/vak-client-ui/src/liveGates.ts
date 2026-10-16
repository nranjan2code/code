/** What a gate item needs for this rule: an approval or a worker question,
 *  answered or not. */
type GateLike = { kind: string; id?: string; resolved?: string | null };

/**
 * Carry the unanswered approvals and questions of `current` into `next`.
 *
 * A transcript is the ledger, and a gate that waits on the person is not in
 * it: it lives on the running turn (`/approvals`, `/questions`). Replacing the
 * item list from a transcript dropped a gate restored a moment before, so a
 * page reloaded while a turn waited showed nothing to answer until the next
 * heartbeat (2026-10-17, 10 s on the dev route).
 */
export function keepLiveGates<T extends GateLike>(current: readonly T[], next: T[]): T[] {
  const present = new Set(next.filter(isGate).map((item) => item.id));
  const waiting = current.filter((item) => isGate(item) && item.resolved === null && !present.has(item.id));
  return waiting.length ? [...next, ...waiting] : next;
}

/** Whether a list holds nothing a transcript paints: empty, or only gates
 *  restored from the running turn before its history arrived. */
export function holdsOnlyLiveGates(items: readonly GateLike[]): boolean {
  return items.every(isGate);
}

function isGate(item: GateLike): boolean {
  return item.kind === "approval" || item.kind === "question";
}
