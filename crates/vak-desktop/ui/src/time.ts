/** Compact relative time shared by the sidebar, tasks, and memory strips. */
export function relTime(iso?: string | null): string {
  if (!iso) return "";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const seconds = (Date.now() - date.getTime()) / 1000;
  if (seconds < 60) return "now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h`;
  if (seconds < 7 * 86400) return `${Math.floor(seconds / 86400)}d`;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** Words variant for sentence positions: "just now", "5m ago". */
export function relAgo(iso?: string | null): string {
  const t = relTime(iso);
  if (!t) return "";
  return t === "now" ? "just now" : `${t} ago`;
}
