import type { OutputItem, WorkProgressItem } from "./types";

/** The latest accepted whole list in an already selected turn. */
export function latestProgress(items: readonly OutputItem[]): WorkProgressItem[] | undefined {
  const item = items.findLast(item => item.content.type === "work_progress");
  return item?.content.type === "work_progress" ? item.content.progress.items : undefined;
}
