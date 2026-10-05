import type { Trigger } from "./types";

export function intervalSecs(trigger: Pick<Trigger, "kind">): number | null;

export function mailCalendarWatchFreshness(
  trigger: Pick<Trigger, "kind" | "scope" | "last_completed_at">,
  nowMs?: number,
): "not_watching" | "unknown" | "scheduled" | "current" | "overdue";
