import { For, Show, createMemo } from "solid-js";
import { draftingOf, isRunning, itemsOf, presentationOf, technicalDetails, type Item } from "../store";
import { stripControlScaffolding } from "../structured";
import Icon from "./Icon";

type Step = { key: string; label: string; state: "running" | "done" | "failed" | "waiting"; detail?: string };
type TurnSteps = { key: string; request: string; steps: Step[] };

const readable = (name: string) => name.replace(/_/g, " ");

/** The form a card call drew, from the card recorded for it or, while it
 *  is written, the form its tool declared. */
function cardForm(sessionId: string, callId: string): string | undefined {
  // A call can have several items (its progress, its card); the card is the
  // structured one.
  for (const item of presentationOf(sessionId)?.items ?? []) {
    if (item.provenance?.tool_call_id === callId && item.content.type === "structured") {
      return item.content.output.block ?? undefined;
    }
  }
  return draftingOf(sessionId).find((card) => card.id === callId)?.block;
}

function plannedParts(argsJson: string): string[] {
  try {
    const parsed = JSON.parse(argsJson) as { deliverables?: { name?: string }[] };
    return (parsed.deliverables ?? []).map((part) => part.name ?? "").filter(Boolean);
  } catch {
    return [];
  }
}

function stepOf(sessionId: string, item: Item, index: number): Step | null {
  switch (item.kind) {
    case "thinking":
      return { key: item.key, label: item.done ? "Thought it through" : "Thinking", state: item.done ? "done" : "running", detail: item.text };
    case "tool": {
      const state = !item.done ? "running" : item.isError ? "failed" : "done";
      const detail = item.isError ? item.preview ?? undefined : undefined;
      if (item.name === "plan") {
        const parts = plannedParts(item.argsJson);
        return { key: item.id, label: parts.length ? `Planned: ${parts.join(", ")}` : "Planned the answer", state, detail };
      }
      if (item.name === "recall") return { key: item.id, label: "Looked back in the conversation", state, detail };
      const form = cardForm(sessionId, item.id);
      if (form) {
        const shown = readable(form);
        return { key: item.id, label: state === "failed" ? `A ${shown} card was refused` : state === "running" ? `Drawing a ${shown} card` : `Showed a ${shown} card`, state, detail };
      }
      return { key: item.id, label: `Used ${readable(item.name)}`, state, detail };
    }
    case "approval":
      return {
        key: item.id,
        label: `Asked to allow ${readable(item.tool)}`,
        state: item.resolved === null ? "waiting" : item.resolved === "allowed" ? "done" : "failed",
        detail: item.resolved === "denied" ? "Declined" : undefined,
      };
    case "question":
      return { key: item.id, label: `Asked: ${item.question}`, state: item.resolved === null ? "waiting" : "done" };
    case "worker":
      return { key: `worker-${index}`, label: `Worker: ${item.label}`, state: item.isError ? "failed" : item.open ? "running" : "done" };
    default:
      return null;
  }
}

/** What each turn of a conversation did, newest first: its model steps,
 *  every call and its state, approvals, questions and workers. The chat
 *  shows outcomes only; this is where the work is seen. */
export default function ActivitySteps(props: { sessionId: string | null }) {
  const turns = createMemo<TurnSteps[]>(() => {
    const sid = props.sessionId;
    if (!sid) return [];
    const out: TurnSteps[] = [];
    itemsOf(sid).forEach((item, index) => {
      if (item.kind === "user") {
        out.push({ key: item.entryId ?? `turn-${index}`, request: stripControlScaffolding(item.text).trim(), steps: [] });
        return;
      }
      const step = stepOf(sid, item, index);
      if (!step) return;
      if (out.length === 0) out.push({ key: "turn-0", request: "", steps: [] });
      const steps = out[out.length - 1].steps;
      // A thought the model has moved on from is finished.
      const previous = steps[steps.length - 1];
      if (previous?.state === "running" && previous.label === "Thinking") {
        steps[steps.length - 1] = { ...previous, label: "Thought it through", state: "done" };
      }
      steps.push(step);
    });
    // The running turn shows from the moment it starts, before its first
    // step arrives.
    const last = out[out.length - 1];
    if (isRunning(sid) && last && last.steps.length === 0) {
      last.steps.push({ key: `${last.key}-waiting`, label: "Waiting for the model", state: "running" });
    }
    return out.filter((turn) => turn.steps.length > 0).reverse();
  });
  return (
    <Show when={turns().length > 0}>
      <section class="activity-steps" aria-label="What Vakyartha did">
        <For each={turns()}>
          {(turn) => (
            <div class="activity-turn">
              <Show when={turn.request}>
                <p class="activity-request" title={turn.request}>{turn.request}</p>
              </Show>
              <ol class="activity-list">
                <For each={turn.steps}>
                  {(step) => (
                    <li class="activity-step" data-state={step.state}>
                      <span class="activity-mark" aria-hidden="true">
                        <Show when={step.state === "failed"} fallback={<Show when={step.state === "done"} fallback={<span class="activity-dot" />}><Icon name="check" size={13} /></Show>}>
                          <Icon name="warning" size={13} />
                        </Show>
                      </span>
                      <span class="activity-label">{step.label}</span>
                      <Show when={step.detail && (step.state === "failed" || technicalDetails())}>
                        <details class="activity-detail">
                          <summary>{step.state === "failed" ? "Why" : "Details"}</summary>
                          <p>{step.detail}</p>
                        </details>
                      </Show>
                    </li>
                  )}
                </For>
              </ol>
            </div>
          )}
        </For>
      </section>
    </Show>
  );
}

/** Whether a conversation has any step to show. */
export function hasActivitySteps(sessionId: string | null): boolean {
  if (!sessionId) return false;
  return isRunning(sessionId) || itemsOf(sessionId).some((item) => item.kind !== "user" && item.kind !== "assistant" && item.kind !== "system");
}
