// How an automation and its runs are described to the person who owns them
// (doc 75 §7: Automation, Run). Shared by the Automations sheet and the
// Canvas so the two never word the same state differently.

import type { RunStatus, Trigger } from "./types";

export function intervalWords(seconds: number): string {
  if (seconds % 3600 === 0) return seconds === 3600 ? "Every hour" : `Every ${seconds / 3600} hours`;
  if (seconds % 60 === 0) return seconds === 60 ? "Every minute" : `Every ${seconds / 60} minutes`;
  return `Every ${seconds} seconds`;
}

/** The short form beside an automation's name: `15m`, `once`, or the cron expression. */
export function cadenceBadge(trigger: Pick<Trigger, "kind">): string {
  if (trigger.kind.kind === "manual") return "manual";
  const schedule = trigger.kind.schedule;
  if (schedule.kind === "cron") return `cron ${schedule.expr}`;
  if (schedule.kind === "once") return "once";
  const s = schedule.every_secs;
  if (s % 3600 === 0) return `${s / 3600}h`;
  if (s % 60 === 0) return `${s / 60}m`;
  return `${s}s`;
}

/** When it runs, in words; the cron expression is kept for the reader who asks for details. */
export function cadenceWords(trigger: Pick<Trigger, "kind">, technical = false): string {
  if (trigger.kind.kind === "manual") return "Only when you run it";
  const schedule = trigger.kind.schedule;
  if (schedule.kind === "cron") return technical ? `On a schedule · ${schedule.expr}` : "On a schedule";
  if (schedule.kind === "once") return `Once, ${new Date(schedule.at).toLocaleString()}`;
  return intervalWords(schedule.every_secs);
}

/** The zone a cron is read in, when it names one. */
export function scheduleZone(trigger: Pick<Trigger, "kind">): string | null {
  return trigger.kind.kind === "schedule" && trigger.kind.schedule.kind === "cron"
    ? trigger.kind.schedule.timezone ?? null
    : null;
}

export function runStatusLabel(status: RunStatus): string {
  return ({
    running: "Working now",
    completed: "Finished",
    failed: "Couldn’t finish",
    cancelled: "Stopped",
    abandoned: "Interrupted",
    skipped: "Didn’t run",
  } as Record<RunStatus, string>)[status] ?? status;
}

export function deliveryStatusLabel(status: string): string {
  return ({
    pending: "Delivery waiting",
    delivered: "Sent",
    failed: "Couldn’t send",
    inbox: "Saved in Inbox",
    agent_session: "Kept in the Agent’s conversation",
  } as Record<string, string>)[status] ?? status;
}

/** Whether a waiting delivery can be tried again. */
export function canRetryDelivery(trigger: Pick<Trigger, "delivery_state">): boolean {
  return trigger.delivery_state === "pending";
}

export type AutomationState = "paused" | "running" | "scheduled";

export function automationState(trigger: Pick<Trigger, "enabled" | "running">): AutomationState {
  if (!trigger.enabled) return "paused";
  return trigger.running ? "running" : "scheduled";
}

/** When it runs next, in the reader's own time with that time's zone
 *  named. `next_run_at` is an instant: shown in one zone and labelled with
 *  another, it would say the wrong hour. */
export function nextRunWords(at: string | null | undefined): string {
  const date = at ? new Date(at) : null;
  if (!date || Number.isNaN(date.getTime())) return "Not scheduled";
  return date.toLocaleString(undefined, { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit", timeZoneName: "short" });
}

/** How many deliveries, in words. */
export function deliveryCount(count: number): string {
  return `${count} ${count === 1 ? "delivery" : "deliveries"}`;
}

/** What it does, in one line. */
export function actionText(trigger: Pick<Trigger, "action">): string {
  return trigger.action.kind === "prompt" ? trigger.action.text : trigger.action.command;
}
