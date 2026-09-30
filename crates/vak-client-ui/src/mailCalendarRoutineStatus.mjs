const CONTINUOUS_WATCH_INTERVAL_SECS = 60;
const OVERDUE_AFTER_MS = 3 * CONTINUOUS_WATCH_INTERVAL_SECS * 1000;

/** Provider-check freshness for the Settings routine list. Cron watches use
 * their own schedule and are not compared with the one-minute threshold. */
export function mailCalendarWatchFreshness(task, nowMs = Date.now()) {
  if (!task.mail_calendar_scope?.watch_new_mail) return "not_watching";

  const checkedAt = task.mail_calendar_last_check_at;
  if (!checkedAt) return "unknown";

  // Only the continuous mode has a fixed one-minute polling contract. A
  // schedule such as weekday mornings must not be labelled late overnight.
  if (task.interval_secs > CONTINUOUS_WATCH_INTERVAL_SECS) return "scheduled";

  const checkedAtMs = Date.parse(checkedAt);
  if (!Number.isFinite(checkedAtMs)) return "unknown";
  return nowMs - checkedAtMs > OVERDUE_AFTER_MS ? "overdue" : "current";
}
