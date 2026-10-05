import assert from "node:assert/strict";
import { actionText, automationState, cadenceBadge, cadenceWords, canRetryDelivery, deliveryCount, deliveryStatusLabel, intervalWords, nextRunWords, runStatusLabel, scheduleZone } from "../src/automationWords.ts";

const every = (every_secs) => ({ kind: { kind: "schedule", schedule: { kind: "interval", every_secs, anchor: "2026-10-01T00:00:00Z" } } });
const cron = (expr, timezone) => ({ kind: { kind: "schedule", schedule: { kind: "cron", expr, timezone } } });

assert.equal(cadenceBadge(every(900)), "15m");
assert.equal(cadenceBadge(every(7200)), "2h");
assert.equal(cadenceBadge(every(45)), "45s");
assert.equal(cadenceBadge(cron("*/5 * * * *")), "cron */5 * * * *");
assert.equal(cadenceBadge({ kind: { kind: "manual" } }), "manual");
assert.equal(intervalWords(60), "Every minute");
assert.equal(intervalWords(3600), "Every hour");
assert.equal(intervalWords(900), "Every 15 minutes");
assert.equal(intervalWords(10800), "Every 3 hours");
assert.equal(cadenceWords(every(900)), "Every 15 minutes");
assert.equal(cadenceWords(cron("0 9 * * 1")), "On a schedule");
assert.equal(cadenceWords(cron("0 9 * * 1"), true), "On a schedule · 0 9 * * 1");
assert.equal(cadenceWords({ kind: { kind: "manual" } }), "Only when you run it");
assert.equal(scheduleZone(cron("0 9 * * 1", "Asia/Kolkata")), "Asia/Kolkata");
assert.equal(scheduleZone(every(60)), null);

assert.equal(runStatusLabel("completed"), "Finished");
assert.equal(runStatusLabel("failed"), "Couldn’t finish");
assert.equal(runStatusLabel("abandoned"), "Interrupted");
assert.equal(runStatusLabel("skipped"), "Didn’t run");
assert.equal(deliveryStatusLabel("inbox"), "Saved in Inbox");
assert.equal(deliveryStatusLabel("something new"), "something new");
assert.equal(canRetryDelivery({ delivery_state: "pending" }), true);
assert.equal(canRetryDelivery({ delivery_state: "delivered" }), false);
assert.equal(canRetryDelivery({}), false);

// A paused automation is paused whatever it is doing; otherwise it is working while a run is open.
assert.equal(automationState({ enabled: false, running: true }), "paused");
assert.equal(automationState({ enabled: true, running: true }), "running");
assert.equal(automationState({ enabled: true, running: false }), "scheduled");

assert.equal(actionText({ action: { kind: "prompt", text: "summarise" } }), "summarise");
assert.equal(actionText({ action: { kind: "script", command: "df -h" } }), "df -h");
assert.equal(deliveryCount(1), "1 delivery");
assert.equal(deliveryCount(2), "2 deliveries");
// The next run is an instant, shown with the reader's zone named.
assert.match(nextRunWords("2026-10-03T09:00:00Z"), /\d/);
assert.ok(/[A-Z]{2,}|GMT|UTC/.test(nextRunWords("2026-10-03T09:00:00Z")), nextRunWords("2026-10-03T09:00:00Z"));
assert.equal(nextRunWords(null), "Not scheduled");
assert.equal(nextRunWords("not a time"), "Not scheduled");

console.log("automation words ok");
