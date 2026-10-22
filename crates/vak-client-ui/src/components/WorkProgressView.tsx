import { For, Show } from "solid-js";
import type { WorkProgressItem } from "../types";

const states: Record<WorkProgressItem["state"], string> = {
  pending: "Waiting", in_progress: "In progress", done: "Done", dropped: "Left out",
};

/** The model's declared steps, separate from verified outcomes and card layout. */
export default function WorkProgressView(props: { items: WorkProgressItem[] }) {
  return <Show when={props.items.length}>
    <section class="work-progress" aria-label="Work progress">
      <strong>Progress</strong>
      <ol><For each={props.items}>{item => <li data-state={item.state}>
        <span class="work-progress-state">{states[item.state]}</span>
        <span>{item.label}<Show when={item.reason}><small>{item.reason}</small></Show></span>
      </li>}</For></ol>
    </section>
  </Show>;
}
