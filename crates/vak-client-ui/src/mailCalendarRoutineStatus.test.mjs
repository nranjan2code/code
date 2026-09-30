import test from "node:test";
import assert from "node:assert/strict";
import { mailCalendarWatchFreshness } from "./mailCalendarRoutineStatus.mjs";

const now = Date.parse("2026-10-01T12:00:00Z");
const watch = (overrides = {}) => ({
  interval_secs: 60,
  mail_calendar_last_check_at: "2026-10-01T11:59:00Z",
  mail_calendar_scope: { watch_new_mail: true },
  ...overrides,
});

test("continuous mail watch freshness distinguishes unknown, current, and overdue", () => {
  assert.equal(mailCalendarWatchFreshness(watch({ mail_calendar_last_check_at: null }), now), "unknown");
  assert.equal(mailCalendarWatchFreshness(watch(), now), "current");
  assert.equal(
    mailCalendarWatchFreshness(watch({ mail_calendar_last_check_at: "2026-10-01T11:56:59.999Z" }), now),
    "overdue",
  );
});

test("scheduled watches are not judged by the continuous polling threshold", () => {
  assert.equal(
    mailCalendarWatchFreshness(watch({
      interval_secs: 86400,
      mail_calendar_last_check_at: "2026-09-28T12:00:00Z",
    }), now),
    "scheduled",
  );
  assert.equal(mailCalendarWatchFreshness({ interval_secs: 60 }, now), "not_watching");
});
