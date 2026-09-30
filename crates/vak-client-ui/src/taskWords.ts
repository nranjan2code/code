// How a scheduled routine is described to the person who owns it. Shared by the
// task list and the Canvas so the two never word the same state differently.

import type { TaskDef } from "./types";

export function intervalWords(seconds: number): string {
  if (seconds % 3600 === 0) return seconds === 3600 ? "Every hour" : `Every ${seconds / 3600} hours`;
  if (seconds % 60 === 0) return seconds === 60 ? "Every minute" : `Every ${seconds / 60} minutes`;
  return `Every ${seconds} seconds`;
}

/** The short form beside a routine's name: `15m`, or the cron expression. */
export function cadenceBadge(task: Pick<TaskDef, "schedule" | "interval_secs">): string {
  if (task.schedule) return `cron ${task.schedule}`;
  const s = task.interval_secs;
  if (s % 3600 === 0) return `${s / 3600}h`;
  if (s % 60 === 0) return `${s / 60}m`;
  return `${s}s`;
}

/** When it runs, in words; the cron expression is kept for the reader who asks for details. */
export function cadenceWords(task: Pick<TaskDef, "schedule" | "interval_secs">, technical = false): string {
  if (task.schedule) return technical ? `On a schedule · ${task.schedule}` : "On a schedule";
  return intervalWords(task.interval_secs);
}

export function runStatusLabel(status: string): string {
  return ({
    working: "Working now",
    complete: "Finished",
    failed: "Couldn’t finish",
    interrupted: "Paused by restart",
  } as Record<string, string>)[status] ?? status;
}

export function deliveryStatusLabel(status: string): string {
  return ({
    pending: "Delivery waiting",
    delivered: "Sent",
    queued: "Queued for delivery",
    inbox: "Saved in Inbox",
  } as Record<string, string>)[status] ?? status;
}

/** Whether a failed delivery can be tried again. */
export function canRetryDelivery(task: Pick<TaskDef, "last_delivery_state">): boolean {
  return task.last_delivery_state === "pending" || task.last_delivery_state === "queued";
}

export type RoutineState = "paused" | "running" | "scheduled";

export function routineState(task: Pick<TaskDef, "enabled" | "last_run_status">, running = false): RoutineState {
  if (!task.enabled) return "paused";
  return running || task.last_run_status === "working" ? "running" : "scheduled";
}
