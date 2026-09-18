// Per-viewer "most recently used" ordering for the agent switcher. This is a
// local convenience, not shared state: it never syncs across devices and is
// fine to lose (falls back to whatever order the server returns).
const KEY = "vak.agentRecents";

function readMap(): Record<string, number> {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as Record<string, number>) : {};
  } catch {
    return {};
  }
}

export function recordAgentOpened(id: string): void {
  try {
    const map = readMap();
    map[id] = Date.now();
    localStorage.setItem(KEY, JSON.stringify(map));
  } catch {
    /* private window / storage blocked — ordering just falls back to default */
  }
}

export function agentRecency(id: string): number {
  return readMap()[id] ?? 0;
}

/** Most-recently-opened first; agents never opened keep their relative order, last. */
export function sortByRecent<T extends { id: string }>(items: T[]): T[] {
  return items
    .map((item, index) => ({ item, index, recency: agentRecency(item.id) }))
    .sort((a, b) => b.recency - a.recency || a.index - b.index)
    .map((entry) => entry.item);
}
