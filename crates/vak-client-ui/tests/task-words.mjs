import assert from "node:assert/strict";
import { cadenceBadge, cadenceWords, canRetryDelivery, deliveryCount, deliveryStatusLabel, intervalWords, nextRunWords, routineState, runStatusLabel } from "../src/taskWords.ts";

assert.equal(cadenceBadge({ interval_secs: 900 }), "15m");
assert.equal(cadenceBadge({ interval_secs: 7200 }), "2h");
assert.equal(cadenceBadge({ interval_secs: 45 }), "45s");
assert.equal(cadenceBadge({ interval_secs: 60, schedule: "*/5 * * * *" }), "cron */5 * * * *");
assert.equal(intervalWords(60), "Every minute");
assert.equal(intervalWords(3600), "Every hour");
assert.equal(intervalWords(900), "Every 15 minutes");
assert.equal(intervalWords(10800), "Every 3 hours");
assert.equal(cadenceWords({ interval_secs: 900 }), "Every 15 minutes");
assert.equal(cadenceWords({ interval_secs: 900, schedule: "0 9 * * 1" }), "On a schedule");
assert.equal(cadenceWords({ interval_secs: 900, schedule: "0 9 * * 1" }, true), "On a schedule · 0 9 * * 1");

assert.equal(runStatusLabel("complete"), "Finished");
assert.equal(runStatusLabel("failed"), "Couldn’t finish");
assert.equal(runStatusLabel("something new"), "something new");
assert.equal(deliveryStatusLabel("inbox"), "Saved in Inbox");
assert.equal(canRetryDelivery({ last_delivery_state: "queued" }), true);
assert.equal(canRetryDelivery({ last_delivery_state: "delivered" }), false);
assert.equal(canRetryDelivery({}), false);

// A paused routine is paused whatever it last did; otherwise it is working while it runs.
assert.equal(routineState({ enabled: false, last_run_status: "working" }), "paused");
assert.equal(routineState({ enabled: true, last_run_status: "working" }), "running");
assert.equal(routineState({ enabled: true, last_run_status: "complete" }, true), "running");
assert.equal(routineState({ enabled: true, last_run_status: "complete" }), "scheduled");

assert.equal(deliveryCount(1), "1 delivery");
assert.equal(deliveryCount(2), "2 deliveries");
// The next run is an instant, shown with the reader's zone named.
assert.match(nextRunWords("2026-10-03T09:00:00Z"), /\d/);
assert.ok(/[A-Z]{2,}|GMT|UTC/.test(nextRunWords("2026-10-03T09:00:00Z")), nextRunWords("2026-10-03T09:00:00Z"));
assert.equal(nextRunWords(null), "Not scheduled yet");
assert.equal(nextRunWords("not a time"), "Not scheduled yet");

console.log("task words ok");
