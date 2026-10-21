import { Index, Show, createMemo } from "solid-js";
import type { ActivityStep, DetailTurn } from "../turnDetails";
import Icon from "./Icon";

/** One selected turn, with repeated successful operations folded together.
 * Failed and waiting calls never disappear into a successful group. */
export default function ActivitySteps(props: { turn: DetailTurn; onInspect: (id: string) => void }) {
  const groups = createMemo(() => {
    const out: ActivityStep[][] = [];
    for (const step of props.turn.steps) {
      const previous = out[out.length - 1];
      if (step.state === "done" && step.tool && previous?.[0].state === "done" && previous[0].tool === step.tool && previous[0].label === step.label) previous.push(step);
      else out.push([step]);
    }
    return out;
  });
  return <section class="activity-steps" aria-label="Activity for this request">
    <Show when={groups().length} fallback={<p class="details-empty">{props.turn.live ? "Waiting for the first activity…" : "No activity was recorded for this request."}</p>}>
      <ol class="activity-list"><Index each={groups()}>{group => {
        const first = () => group()[0];
        return <li class="activity-step" data-state={first().state}>
          <span class="activity-mark" aria-hidden="true"><Show when={first().state === "running" || first().state === "waiting"} fallback={<Icon name={first().state === "failed" ? "warning" : first().state === "done" ? "check" : "history"} size={14} />}><span class="activity-dot" /></Show></span>
          <div class="activity-copy"><span class="activity-label">{first().label}</span><span class="activity-state">{first().state === "done" ? "Finished" : first().state === "failed" ? "Failed" : first().state === "waiting" ? "Needs your decision" : first().state === "ended" ? "Ended · completion not confirmed" : "In progress"}<Show when={group().length > 1}> · {group().length} calls</Show></span></div>
          <Show when={first().state === "failed" && first().result}><p class="activity-problem">{first().result}</p></Show>
          <Show when={group().some(s => s.args || s.result || s.executionId)}>
            <details class="activity-detail"><summary>{group().length > 1 ? `Inspect ${group().length} calls` : "Inspect step"}</summary><Index each={group()}>{step => <div class="activity-call">
              <Show when={step().tool}><p class="activity-tool">Tool: {step().tool}</p></Show>
              <Show when={step().args}><details><summary>Arguments</summary><pre>{step().args}</pre></details></Show>
              <Show when={step().result && step().state !== "failed"}><details><summary>Recorded output</summary><pre>{step().result}</pre></details></Show>
              <Show when={step().executionId}><button type="button" class="btn" onClick={() => props.onInspect(step().executionId!)}>Command output and checks</button></Show>
            </div>}</Index></details>
          </Show>
        </li>;
      }}</Index></ol>
    </Show>
  </section>;
}
