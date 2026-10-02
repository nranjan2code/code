import assert from "node:assert/strict";
import test from "node:test";
import { overlappingMailCalendarEventIds } from "./mailCalendarConflicts.mjs";

const timed = (provider_id, starts_at, ends_at) => ({
  provider_id,
  all_day: false,
  starts_on: null,
  ends_on: null,
  starts_at,
  ends_at,
});

const allDay = (provider_id, starts_on, ends_on) => ({
  provider_id,
  all_day: true,
  starts_on,
  ends_on,
  starts_at: null,
  ends_at: null,
});

test("marks both timed events when their intervals overlap", () => {
  const result = overlappingMailCalendarEventIds([
    timed("first", "2026-10-01T09:00:00Z", "2026-10-01T10:00:00Z"),
    timed("second", "2026-10-01T09:30:00Z", "2026-10-01T10:30:00Z"),
    timed("later", "2026-10-01T10:30:00Z", "2026-10-01T11:00:00Z"),
  ]);
  assert.deepEqual([...result].sort(), ["first", "second"]);
});

test("detects cross-account conflicts without conflating equal provider IDs", () => {
  const result = overlappingMailCalendarEventIds([
    { ...timed("same-id", "2026-10-01T09:00:00Z", "2026-10-01T10:00:00Z"), account_id: "account-a" },
    { ...timed("same-id", "2026-10-01T09:30:00Z", "2026-10-01T10:30:00Z"), account_id: "account-b" },
  ]);
  assert.deepEqual([...result].sort(), ["account-a::same-id", "account-b::same-id"]);
});

test("uses exclusive all-day end dates and skips invalid or missing bounds", () => {
  const result = overlappingMailCalendarEventIds([
    allDay("all-day", "2026-10-01", "2026-10-02"),
    allDay("same-day", "2026-10-01", "2026-10-02"),
    allDay("next-day", "2026-10-02", "2026-10-03"),
    allDay("invalid", "2026-02-30", "2026-03-01"),
    timed("unknown", null, null),
  ]);
  assert.deepEqual([...result].sort(), ["all-day", "same-day"]);
});

test("does not infer conflicts from a private event without usable times", () => {
  const result = overlappingMailCalendarEventIds([
    timed("private", "invalid", "invalid"),
    timed("public", "2026-10-01T09:00:00Z", "2026-10-01T10:00:00Z"),
  ]);
  assert.deepEqual([...result], []);
});
