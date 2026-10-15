// How an automation and its runs are described to the person who owns them
// (doc 75 §7: Automation, Run). Shared by the Automations sheet and the
// Canvas so the two never word the same state differently. When it runs
// is the server's `when_words` (`triggers::describe`), never worded here.

import type { RunStatus, Trigger } from "./types";

/** Which slots a skipped or caught-up record stands for, in words. */
export function missedWords(run: { missed?: { from: string; through: string } | null; coalesced_into?: string | null }, when: (at: string) => string): string | null {
  if (!run.missed) return null;
  const { from, through } = run.missed;
  const one = from === through;
  const range = one ? `The run due ${when(from)} was` : `The runs due from ${when(from)} to ${when(through)} were`;
  return run.coalesced_into ? `${range} missed and caught up by the next run.` : `${range} missed.`;
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
    pending: "Sending",
    delivered: "Sent",
    failed: "Didn’t send",
    unknown: "Not sure it was sent",
    inbox: "Saved in Inbox",
    agent_session: "Kept in the Agent’s conversation",
  } as Record<string, string>)[status] ?? status;
}

/** Whether the last run's message can be sent again: one that didn't
 *  send, may not have, or is waiting for another try. */
export function canSendAgain(trigger: Pick<Trigger, "delivery_effect">): boolean {
  return Boolean(trigger.delivery_effect);
}

/** What sending a message again did. */
export function sentAgainWords(result: { new: boolean; status: string }): string {
  if (result.status === "sent") return result.new ? "Sent again as a new message" : "Sent";
  return "Tried again; it hasn’t arrived yet";
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


/** What it does, in one line. */
export function actionText(trigger: Pick<Trigger, "action">): string {
  switch (trigger.action.kind) {
    case "prompt": return trigger.action.text;
    case "script": return trigger.action.command;
    case "ability_review": return "Looks at the abilities that failed and proposes fixes for your review";
  }
}
