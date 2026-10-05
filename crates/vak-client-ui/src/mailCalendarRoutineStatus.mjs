const CONTINUOUS_WATCH_INTERVAL_SECS = 60;
const OVERDUE_AFTER_MS = 3 * CONTINUOUS_WATCH_INTERVAL_SECS * 1000;

/** The seconds between an automation's slots when it runs on an interval. */
export function intervalSecs(trigger) {
  const schedule = trigger.kind?.kind === "schedule" ? trigger.kind.schedule : null;
  return schedule?.kind === "interval" ? schedule.every_secs : null;
}

/** Provider-check freshness for Settings. Cron-based mail and calendar
 * routines use their own schedule, not the one-minute threshold. The last
 * successful check is the newest completed run (`last_completed_at`). */
export function mailCalendarWatchFreshness(trigger, nowMs = Date.now()) {
  const scope = trigger.scope;
  if (!scope?.watch_new_mail && !scope?.calendar_event_trigger) return "not_watching";

  const checkedAt = trigger.last_completed_at;
  if (!checkedAt) return "unknown";

  // Only the continuous mode has a fixed one-minute polling contract. A
  // schedule such as weekday mornings must not be labelled late overnight.
  const every = intervalSecs(trigger);
  if (every === null || every > CONTINUOUS_WATCH_INTERVAL_SECS) return "scheduled";

  const checkedAtMs = Date.parse(checkedAt);
  if (!Number.isFinite(checkedAtMs)) return "unknown";
  return nowMs - checkedAtMs > OVERDUE_AFTER_MS ? "overdue" : "current";
}
