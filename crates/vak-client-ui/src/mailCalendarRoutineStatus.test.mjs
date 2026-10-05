import test from "node:test";
import assert from "node:assert/strict";
import { mailCalendarWatchFreshness } from "./mailCalendarRoutineStatus.mjs";

const now = Date.parse("2026-10-01T12:00:00Z");
const every = (every_secs) => ({
  kind: "schedule",
  schedule: { kind: "interval", every_secs, anchor: "2026-10-01T00:00:00Z" },
});
const watch = (overrides = {}) => ({
  kind: every(60),
  last_completed_at: "2026-10-01T11:59:00Z",
  scope: { watch_new_mail: true },
  ...overrides,
});

test("continuous mail watch freshness distinguishes unknown, current, and overdue", () => {
  assert.equal(mailCalendarWatchFreshness(watch({ last_completed_at: null }), now), "unknown");
  assert.equal(mailCalendarWatchFreshness(watch(), now), "current");
  assert.equal(
    mailCalendarWatchFreshness(watch({ last_completed_at: "2026-10-01T11:56:59.999Z" }), now),
    "overdue",
  );
});

test("scheduled watches are not judged by the continuous polling threshold", () => {
  assert.equal(
    mailCalendarWatchFreshness(watch({ kind: every(86400), last_completed_at: "2026-09-28T12:00:00Z" }), now),
    "scheduled",
  );
  assert.equal(
    mailCalendarWatchFreshness(watch({ kind: { kind: "schedule", schedule: { kind: "cron", expr: "0 9 * * 1-5" } } }), now),
    "scheduled",
  );
  assert.equal(mailCalendarWatchFreshness({ kind: every(60) }, now), "not_watching");
});

test("calendar event triggers use the same provider-check freshness indicator", () => {
  const eventRoutine = (overrides = {}) => ({
    kind: every(60),
    last_completed_at: "2026-10-01T11:59:00Z",
    scope: { calendar_event_trigger: { boundary: "start" } },
    ...overrides,
  });
  assert.equal(mailCalendarWatchFreshness(eventRoutine(), now), "current");
  assert.equal(mailCalendarWatchFreshness(eventRoutine({ last_completed_at: null }), now), "unknown");
  assert.equal(mailCalendarWatchFreshness(eventRoutine({
    last_completed_at: "2026-10-01T11:56:59.999Z",
  }), now), "overdue");
  assert.equal(mailCalendarWatchFreshness(eventRoutine({ kind: every(3600) }), now), "scheduled");
});
