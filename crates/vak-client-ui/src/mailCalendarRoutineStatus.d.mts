import type { TaskDef } from "./types";

export function mailCalendarWatchFreshness(
  task: Pick<TaskDef, "interval_secs" | "mail_calendar_scope" | "mail_calendar_last_check_at">,
  nowMs?: number,
): "not_watching" | "unknown" | "scheduled" | "current" | "overdue";
