const CONTINUOUS_WATCH_INTERVAL_SECS = 60;
const OVERDUE_AFTER_MS = 3 * CONTINUOUS_WATCH_INTERVAL_SECS * 1000;

/** Provider-check freshness for Settings. Cron-based mail and calendar
 * routines use their own schedule, not the one-minute threshold. */
export function mailCalendarWatchFreshness(task, nowMs = Date.now()) {
  const scope = task.mail_calendar_scope;
  if (!scope?.watch_new_mail && !scope?.calendar_event_trigger) return "not_watching";

  const checkedAt = task.mail_calendar_last_check_at;
  if (!checkedAt) return "unknown";

  // Only the continuous mode has a fixed one-minute polling contract. A
  // schedule such as weekday mornings must not be labelled late overnight.
  if (task.interval_secs > CONTINUOUS_WATCH_INTERVAL_SECS) return "scheduled";

  const checkedAtMs = Date.parse(checkedAt);
  if (!Number.isFinite(checkedAtMs)) return "unknown";
  return nowMs - checkedAtMs > OVERDUE_AFTER_MS ? "overdue" : "current";
}
