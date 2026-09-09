import { For, Show } from "solid-js";

export interface TimelineData {
  title: string;
  items: { label: string; detail?: string; status?: string }[];
}

/** Generic, non-technical timeline for plans, itineraries, schedules, and lessons. */
export default function TimelineCard(props: { data: TimelineData }) {
  return (
    <section class="adaptive-timeline" aria-label={props.data.title}>
      <header class="adaptive-timeline-head">
        <span class="adaptive-timeline-kicker">Plan</span>
        <h3>{props.data.title}</h3>
      </header>
      <ol>
        <For each={props.data.items}>
          {(item, index) => (
            <li classList={{ complete: item.status === "complete" }}>
              <span class="adaptive-timeline-marker" aria-hidden="true">{index() + 1}</span>
              <div>
                <strong>{item.label}</strong>
                <Show when={item.detail}><p>{item.detail}</p></Show>
                <Show when={item.status}><small>{item.status}</small></Show>
              </div>
            </li>
          )}
        </For>
      </ol>
    </section>
  );
}
